//! Local features of the service: shell profiles, scripts (own files and links), notes (in the vault), icons.
//! Nothing here touches a server.
use super::{new_id, now, ApiError, Service};
use crate::localshell::{self, ShellProfile};
use crate::sftp::codec;
use crate::vault::Note;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Script types the program can open and run.
const SCRIPT_EXT: [&str; 4] = ["bat", "cmd", "ps1", "sh"];
const ICON_EXT: [&str; 7] = ["png", "svg", "jpg", "jpeg", "webp", "ico", "gif"];
const ICON_MAX_BYTES: u64 = 1024 * 1024;

fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn is_script(path: &str) -> bool {
    SCRIPT_EXT.contains(&ext_of(path).as_str())
}

/// A file name for a new own script: no folders, a known extension, not hidden or reserved.
fn valid_script_name(name: &str) -> Result<String, ApiError> {
    let n = name.trim();
    let bad = n.is_empty()
        || n.chars().count() > 100
        || n.starts_with('.')
        || n.chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c));
    if bad {
        return Err(ApiError::new("invalid", "Invalid file name."));
    }
    if !is_script(n) {
        return Err(ApiError::new(
            "invalid",
            "A script file name must end with .bat, .cmd, .ps1 or .sh.",
        ));
    }
    Ok(n.to_string())
}

fn io_err(what: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError::new("io", format!("{what}: {e}"))
}

impl Service {
    fn all_profiles(&self) -> Vec<ShellProfile> {
        let mut v = localshell::detect_system();
        v.extend(localshell::custom_profiles(
            &self.settings_snapshot().shell_profiles,
        ));
        v
    }

    pub(super) fn shell_profiles(&self) -> Value {
        let s = self.settings_snapshot();
        json!({ "profiles": self.all_profiles(), "startup": s.startup_profile })
    }

    fn dim(params: &Value, k: &str, d: u64) -> u16 {
        params
            .get(k)
            .and_then(Value::as_u64)
            .unwrap_or(d)
            .clamp(1, 1000) as u16
    }

    fn open_local(
        &self,
        program: &str,
        args: Vec<String>,
        cwd: &str,
        params: &Value,
    ) -> Result<Value, ApiError> {
        let dir = if cwd.trim().is_empty() {
            None
        } else {
            let p = PathBuf::from(cwd);
            if !p.is_dir() {
                return Err(ApiError::new(
                    "invalid",
                    format!("The start folder does not exist: {cwd}"),
                ));
            }
            Some(p)
        };
        let id = self.hub.local_open(
            PathBuf::from(program),
            args,
            dir,
            Self::dim(params, "cols", 80),
            Self::dim(params, "rows", 24),
        )?;
        Ok(json!({ "term_id": id }))
    }

    pub(super) fn local_term_open(&self, params: &Value) -> Result<Value, ApiError> {
        let id = params
            .get("profile_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new("invalid", "Missing parameter profile_id"))?;
        let profile = self
            .all_profiles()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| {
                ApiError::new("not_found", "The shell was not found on this computer.")
            })?;
        self.open_local(&profile.program, profile.args.clone(), &profile.cwd, params)
    }

    // ------------------------------------------------------------ scripts

    /// Resolve "own:<name>" / "link:<path>" to a file path (own scripts never leave the scripts folder).
    fn script_path(&self, id: &str) -> Result<PathBuf, ApiError> {
        if let Some(name) = id.strip_prefix("own:") {
            let n = valid_script_name(name)?;
            return Ok(self.dirs.scripts().join(n));
        }
        if let Some(p) = id.strip_prefix("link:") {
            if !self.settings_snapshot().script_links.iter().any(|l| l == p) {
                return Err(ApiError::new("not_found", "The script is not in the list."));
            }
            return Ok(PathBuf::from(p));
        }
        Err(ApiError::new("invalid", "Unknown script."))
    }

    pub(super) fn scripts_list(&self) -> Result<Value, ApiError> {
        let mut out: Vec<Value> = Vec::new();
        let dir = self.dirs.scripts();
        let mut own: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| is_script(n) && !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        own.sort_by_key(|n| n.to_lowercase());
        for n in own {
            out.push(json!({ "id": format!("own:{n}"), "name": n, "ext": ext_of(&n), "kind": "own", "path": dir.join(&n).to_string_lossy(), "exists": true }));
        }
        for l in self.settings_snapshot().script_links {
            let name = Path::new(&l)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| l.clone());
            out.push(json!({ "id": format!("link:{l}"), "name": name, "ext": ext_of(&l), "kind": "link", "path": l, "exists": Path::new(&l).is_file() }));
        }
        Ok(json!({ "scripts": out }))
    }

    pub(super) fn script_read(&self, id: &str) -> Result<Value, ApiError> {
        let path = self.script_path(id)?;
        let limit = u64::from(self.settings_snapshot().editor_limit_mb) * 1024 * 1024;
        let meta = std::fs::metadata(&path).map_err(|e| io_err("Could not read the file", e))?;
        if meta.len() > limit {
            return Err(ApiError::new(
                "too_large",
                "The file is too large for the editor.",
            ));
        }
        let bytes = std::fs::read(&path).map_err(|e| io_err("Could not read the file", e))?;
        let d = codec::decode(&bytes, None)
            .map_err(|f| ApiError::new("encoding", codec::fail_message(&f)))?;
        let crlf = d.text.matches("\r\n").count();
        let lf = d.text.matches('\n').count() - crlf;
        let eol = if crlf > lf { "crlf" } else { "lf" };
        let text = d.text.replace("\r\n", "\n");
        Ok(json!({
            "id": id, "path": path.to_string_lossy(), "text": text, "eol": eol,
            "bom": d.bom, "encoding": d.encoding.name(),
        }))
    }

    pub(super) fn script_write(&self, params: &Value) -> Result<Value, ApiError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new("invalid", "Missing parameter id"))?;
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new("invalid", "Missing parameter text"))?;
        let path = self.script_path(id)?;
        let eol = params.get("eol").and_then(Value::as_str).unwrap_or("lf");
        let bom = params.get("bom").and_then(Value::as_bool).unwrap_or(false);
        let enc_name = params
            .get("encoding")
            .and_then(Value::as_str)
            .unwrap_or("UTF-8");
        let enc = codec::by_name(enc_name)
            .ok_or_else(|| ApiError::new("encoding", format!("Unknown encoding “{enc_name}”.")))?;
        let text = if eol == "crlf" {
            text.replace('\n', "\r\n")
        } else {
            text.to_string()
        };
        let bytes = codec::encode(&text, bom, enc).map_err(|c| {
            ApiError::new("encoding", format!("Character “{c}” (U+{:04X}) cannot be written in the encoding {enc_name}. Remove it or choose another encoding (for example, UTF-8).", c as u32))
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("Could not write the file", e))?;
        }
        // write next to the target and rename, so that a failure never leaves a half-written script
        let tmp = path.with_extension(format!("{}.d256tmp", ext_of(&path.to_string_lossy())));
        std::fs::write(&tmp, &bytes).map_err(|e| io_err("Could not write the file", e))?;
        if let Err(e) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io_err("Could not write the file", e));
        }
        Ok(json!({ "ok": true }))
    }

    pub(super) fn script_create(&self, params: &Value) -> Result<Value, ApiError> {
        let name = valid_script_name(params.get("name").and_then(Value::as_str).unwrap_or(""))?;
        let text = params.get("text").and_then(Value::as_str).unwrap_or("");
        let dir = self.dirs.scripts();
        std::fs::create_dir_all(&dir)
            .map_err(|e| io_err("Could not create the scripts folder", e))?;
        let path = dir.join(&name);
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        ApiError::new("exists", format!("“{name}” already exists."))
                    } else {
                        io_err("Could not create the file", e)
                    }
                })?;
            f.write_all(text.as_bytes())
                .map_err(|e| io_err("Could not write the file", e))?;
        }
        Ok(json!({ "id": format!("own:{name}") }))
    }

    pub(super) fn script_delete(&self, id: &str) -> Result<Value, ApiError> {
        if let Some(path) = id.strip_prefix("link:") {
            // a link: only the entry goes, never the file
            let mut s = self.settings_snapshot();
            s.script_links.retain(|l| l != path);
            self.store_settings(s)?;
            return Ok(json!({ "ok": true }));
        }
        let path = self.script_path(id)?;
        std::fs::remove_file(&path).map_err(|e| io_err("Could not delete the file", e))?;
        Ok(json!({ "ok": true }))
    }

    pub(super) fn script_link_add(&self, params: &Value) -> Result<Value, ApiError> {
        let paths: Vec<String> = params
            .get("paths")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let mut s = self.settings_snapshot();
        let (mut added, mut skipped) = (0, Vec::new());
        for p in paths {
            if !is_script(&p) || !Path::new(&p).is_file() {
                skipped.push(p);
                continue;
            }
            if !s.script_links.contains(&p) {
                s.script_links.push(p);
                added += 1;
            }
        }
        if added > 0 {
            self.store_settings(s)?;
        }
        Ok(json!({ "added": added, "skipped": skipped }))
    }

    /// Point a link whose file was moved to its new place.
    pub(super) fn script_link_move(&self, params: &Value) -> Result<Value, ApiError> {
        let old = params.get("old").and_then(Value::as_str).unwrap_or("");
        let new = params.get("new").and_then(Value::as_str).unwrap_or("");
        if !is_script(new) || !Path::new(new).is_file() {
            return Err(ApiError::new(
                "invalid",
                "Choose an existing .bat, .cmd, .ps1 or .sh file.",
            ));
        }
        let mut s = self.settings_snapshot();
        let Some(i) = s.script_links.iter().position(|l| l == old) else {
            return Err(ApiError::new("not_found", "The script is not in the list."));
        };
        s.script_links[i] = new.to_string();
        s.script_links.dedup();
        self.store_settings(s)?;
        Ok(json!({ "id": format!("link:{new}") }))
    }

    pub(super) fn script_run(&self, params: &Value) -> Result<Value, ApiError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new("invalid", "Missing parameter id"))?;
        let path = self.script_path(id)?;
        if !path.is_file() {
            return Err(ApiError::new("not_found", "The script file was not found."));
        }
        let script = path.to_string_lossy().into_owned();
        let (program, args) = localshell::script_command(
            &ext_of(&script),
            cfg!(windows),
            &self.all_profiles(),
            &script,
        )
        .map_err(|m| ApiError::new("unsupported", m))?;
        let cwd = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.open_local(&program, args, &cwd, params)
    }

    // ------------------------------------------------------------ notes

    pub(super) fn note_list(&self) -> Result<Value, ApiError> {
        self.with_vault(|v| {
            let mut list: Vec<&Note> = v.data.notes.iter().collect();
            list.sort_by_key(|a| std::cmp::Reverse(a.updated));
            Ok(json!({ "notes": list.iter().map(|n| json!({
                "id": n.id, "title": n.title, "updated": n.updated, "size": n.text.len(),
            })).collect::<Vec<_>>() }))
        })
    }

    pub(super) fn note_get(&self, id: &str) -> Result<Value, ApiError> {
        let found = self.with_vault(|v| Ok(v.data.notes.iter().find(|n| n.id == id).cloned()))?;
        match found {
            Some(n) => {
                Ok(json!({ "id": n.id, "title": n.title, "text": n.text, "updated": n.updated }))
            }
            None => Err(ApiError::new("not_found", "The note was not found.")),
        }
    }

    pub(super) fn note_save(&self, params: &Value) -> Result<Value, ApiError> {
        let id = params.get("id").and_then(Value::as_str).map(str::to_owned);
        let title = params
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if title.chars().count() > 200 {
            return Err(ApiError::new("invalid", "The note title is too long."));
        }
        if text.len() > 5 * 1024 * 1024 {
            return Err(ApiError::new("invalid", "The note is too large."));
        }
        let title = if title.is_empty() {
            "Untitled".to_string()
        } else {
            title
        };
        let fresh = new_id()?;
        let t = now();
        self.with_vault(|v| {
            let id = match id {
                Some(id) if v.data.notes.iter().any(|n| n.id == id) => {
                    let n = v
                        .data
                        .notes
                        .iter_mut()
                        .find(|n| n.id == id)
                        .expect("checked");
                    n.title = title;
                    n.text = text;
                    n.updated = t;
                    id
                }
                _ => {
                    v.data.notes.push(Note {
                        id: fresh.clone(),
                        title,
                        text,
                        created: t,
                        updated: t,
                    });
                    fresh
                }
            };
            v.save().map(|_| json!({ "id": id, "updated": t }))
        })
    }

    pub(super) fn note_delete(&self, id: &str) -> Result<Value, ApiError> {
        self.with_vault(|v| {
            v.data.notes.retain(|n| n.id != id);
            v.save().map(|_| json!({ "ok": true }))
        })
    }

    // ------------------------------------------------------------ icons

    /// Copy an image into the data folder (so the data folder stays self-contained) and return its file name.
    pub(super) fn icon_import(&self, path: &str) -> Result<Value, ApiError> {
        let ext = ext_of(path);
        if !ICON_EXT.contains(&ext.as_str()) {
            return Err(ApiError::new(
                "invalid",
                "Choose a png, svg, jpg, webp, ico or gif image.",
            ));
        }
        let meta = std::fs::metadata(path).map_err(|e| io_err("Could not read the image", e))?;
        if !meta.is_file() || meta.len() > ICON_MAX_BYTES {
            return Err(ApiError::new(
                "invalid",
                "The image is too large (limit 1 MB).",
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| io_err("Could not read the image", e))?;
        let name = format!("{}.{ext}", new_id()?);
        let dir = self.dirs.icons();
        std::fs::create_dir_all(&dir)
            .map_err(|e| io_err("Could not create the icons folder", e))?;
        std::fs::write(dir.join(&name), bytes).map_err(|e| io_err("Could not save the icon", e))?;
        Ok(json!({ "name": name }))
    }

    pub(super) fn icon_get(&self, name: &str) -> Result<Value, ApiError> {
        // only plain file names written by icon_import
        let ok = !name.is_empty()
            && !name.contains(['/', '\\'])
            && !name.starts_with('.')
            && ICON_EXT.contains(&ext_of(name).as_str());
        if !ok {
            return Err(ApiError::new("invalid", "Invalid icon."));
        }
        let bytes = std::fs::read(self.dirs.icons().join(name))
            .map_err(|e| io_err("Could not read the icon", e))?;
        let mime = match ext_of(name).as_str() {
            "svg" => "image/svg+xml",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "ico" => "image/x-icon",
            "gif" => "image/gif",
            _ => "image/png",
        };
        use base64ct::{Base64, Encoding};
        Ok(json!({ "data_url": format!("data:{mime};base64,{}", Base64::encode_string(&bytes)) }))
    }

    /// Save a modified copy of the settings (and make it current).
    fn store_settings(&self, s: crate::settings::Settings) -> Result<(), ApiError> {
        let s = s.sanitized();
        s.save(&self.dirs.settings())
            .map_err(|e| io_err("Could not save the settings", e))?;
        *self.settings.lock().expect("lock") = s;
        Ok(())
    }
}
