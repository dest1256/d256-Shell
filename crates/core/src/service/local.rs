//! Local features of the service: shell profiles, scripts (own files and links), notes (in the vault), icons.
//! Nothing here touches a server.
use super::{new_id, now, ApiError, Service};
use crate::localfs;
use crate::localshell::{self, ShellProfile};
use crate::sftp::codec;
use crate::vault::{Note, VaultScript};
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
        cleanup: Option<PathBuf>,
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
            cleanup,
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
        self.open_local(
            &profile.program,
            profile.args.clone(),
            &profile.cwd,
            params,
            None,
        )
    }

    // ------------------------------------------------------------ scripts

    /// Resolve "own:<name>" / "link:<path>" to a file path (own scripts never leave the scripts folder).
    fn script_path(&self, id: &str) -> Result<PathBuf, ApiError> {
        // any file chosen in the local explorer (read and write only; it is not a script of the list)
        if let Some(p) = id.strip_prefix("file:") {
            return localfs::real_path(p, cfg!(windows)).map_err(|m| ApiError::new("invalid", m));
        }
        if let Some(p) = id.strip_prefix("link:") {
            if !self.settings_snapshot().script_links.iter().any(|l| l == p) {
                return Err(ApiError::new("not_found", "The script is not in the list."));
            }
            return Ok(PathBuf::from(p));
        }
        Err(ApiError::new("invalid", "Unknown script."))
    }

    /// Name of an own script from its id ("own:<name>").
    fn own_name(id: &str) -> Result<String, ApiError> {
        match id.strip_prefix("own:") {
            Some(n) => valid_script_name(n),
            None => Err(ApiError::new("invalid", "Unknown script.")),
        }
    }

    /// Scripts that 1.1.x kept as plain files in `data/scripts` move into the encrypted vault (once, when the vault is
    /// open); a file is removed only after the vault was saved. Returns how many were moved.
    fn migrate_plain_scripts(&self) -> usize {
        let dir = self.dirs.scripts();
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return 0;
        };
        let files: Vec<(String, PathBuf, Vec<u8>)> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                if !is_script(&name) || name.starts_with('.') {
                    return None;
                }
                let bytes = std::fs::read(e.path()).ok()?;
                Some((name, e.path(), bytes))
            })
            .collect();
        if files.is_empty() {
            return 0;
        }
        let t = now();
        let saved = self.with_vault(|v| {
            for (name, _, bytes) in &files {
                let mut unique = name.clone();
                let mut n = 1;
                while v
                    .data
                    .scripts
                    .iter()
                    .any(|x| x.name.eq_ignore_ascii_case(&unique))
                {
                    n += 1;
                    unique = match name.rfind('.') {
                        Some(i) => format!("{} ({n}){}", &name[..i], &name[i..]),
                        None => format!("{name} ({n})"),
                    };
                }
                v.data.scripts.push(VaultScript::new(unique, bytes, t));
            }
            v.save()
        });
        if saved.is_err() {
            return 0;
        }
        let mut moved = 0;
        for (_, path, _) in &files {
            if std::fs::remove_file(path).is_ok() {
                moved += 1;
            }
        }
        let _ = std::fs::remove_dir(&dir); // only when empty
        moved
    }

    pub(super) fn scripts_list(&self) -> Result<Value, ApiError> {
        self.with_vault(|_| Ok(()))?; // a locked vault has no scripts to show
        let migrated = self.migrate_plain_scripts();
        let mut out: Vec<Value> = Vec::new();
        let mut own: Vec<String> =
            self.with_vault(|v| Ok(v.data.scripts.iter().map(|s| s.name.clone()).collect()))?;
        own.sort_by_key(|n| n.to_lowercase());
        for n in own {
            out.push(json!({ "id": format!("own:{n}"), "name": n, "ext": ext_of(&n), "kind": "own", "path": "", "exists": true }));
        }
        for l in self.settings_snapshot().script_links {
            let name = Path::new(&l)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| l.clone());
            out.push(json!({ "id": format!("link:{l}"), "name": name, "ext": ext_of(&l), "kind": "link", "path": l, "exists": Path::new(&l).is_file() }));
        }
        Ok(json!({ "scripts": out, "migrated": migrated }))
    }

    /// The raw bytes of a script (own: from the vault, otherwise from its file).
    fn script_bytes(&self, id: &str) -> Result<(Vec<u8>, String), ApiError> {
        let limit = u64::from(self.settings_snapshot().editor_limit_mb) * 1024 * 1024;
        if id.starts_with("own:") {
            let name = Self::own_name(id)?;
            let bytes = self
                .with_vault(|v| {
                    Ok(v.data
                        .scripts
                        .iter()
                        .find(|s| s.name == name)
                        .map(|s| s.bytes()))
                })?
                .ok_or_else(|| ApiError::new("not_found", "The script was not found."))?;
            if bytes.len() as u64 > limit {
                return Err(ApiError::new(
                    "too_large",
                    "The file is too large for the editor.",
                ));
            }
            return Ok((bytes, String::new()));
        }
        let path = self.script_path(id)?;
        let meta = std::fs::metadata(&path).map_err(|e| io_err("Could not read the file", e))?;
        if meta.len() > limit {
            return Err(ApiError::new(
                "too_large",
                "The file is too large for the editor.",
            ));
        }
        let bytes = std::fs::read(&path).map_err(|e| io_err("Could not read the file", e))?;
        Ok((bytes, path.to_string_lossy().into_owned()))
    }

    pub(super) fn script_read(&self, params: &Value) -> Result<Value, ApiError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new("invalid", "Missing parameter id"))?;
        let forced = params.get("encoding").and_then(Value::as_str);
        let (bytes, path) = self.script_bytes(id)?;
        let d = codec::decode(&bytes, forced).map_err(|f| {
            let code = match f {
                codec::DecodeFail::Binary => "binary",
                codec::DecodeFail::Unknown(_) => "encoding",
                _ => "bad_encoding",
            };
            ApiError::new(code, codec::fail_message(&f))
        })?;
        let crlf = d.text.matches("\r\n").count();
        let lf = d.text.matches('\n').count() - crlf;
        let eol = if crlf > lf { "crlf" } else { "lf" };
        let text = d.text.replace("\r\n", "\n");
        Ok(json!({
            "id": id, "path": path, "text": text, "eol": eol,
            "bom": d.bom, "encoding": d.encoding.name(), "guessed": d.guessed,
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
        if id.starts_with("own:") {
            // an own script is stored encrypted in the vault, as exact file bytes
            let name = Self::own_name(id)?;
            let t = now();
            let found = self.with_vault(|v| {
                let Some(s) = v.data.scripts.iter_mut().find(|s| s.name == name) else {
                    return Ok(false);
                };
                s.set_bytes(&bytes, t);
                v.save().map(|_| true)
            })?;
            return if found {
                Ok(json!({ "ok": true }))
            } else {
                Err(ApiError::new("not_found", "The script was not found."))
            };
        }
        let path = self.script_path(id)?;
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
        let t = now();
        let created = self.with_vault(|v| {
            if v.data
                .scripts
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(&name))
            {
                return Ok(false);
            }
            v.data
                .scripts
                .push(VaultScript::new(name.clone(), text.as_bytes(), t));
            v.save().map(|_| true)
        })?;
        if !created {
            return Err(ApiError::new("exists", format!("“{name}” already exists.")));
        }
        Ok(json!({ "id": format!("own:{name}") }))
    }

    pub(super) fn script_delete(&self, id: &str) -> Result<Value, ApiError> {
        if id.starts_with("file:") {
            return Err(ApiError::new("invalid", "Unknown script."));
        }
        if let Some(path) = id.strip_prefix("link:") {
            // a link: only the entry goes, never the file
            let mut s = self.settings_snapshot();
            s.script_links.retain(|l| l != path);
            self.store_settings(s)?;
            return Ok(json!({ "ok": true }));
        }
        let name = Self::own_name(id)?;
        self.with_vault(|v| {
            v.data.scripts.retain(|s| s.name != name);
            v.save().map(|_| json!({ "ok": true }))
        })
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
        if id.starts_with("file:") {
            return Err(ApiError::new("invalid", "Unknown script."));
        }
        let (path, cwd, cleanup) = if id.starts_with("own:") {
            // an own script is decrypted into a private folder only for this run; it is removed when the terminal
            // closes (and the staging folder is emptied at every start and exit)
            let name = Self::own_name(id)?;
            let (bytes, _) = self.script_bytes(id)?;
            let dir = self.dirs.staging().join(format!("run-{}", new_id()?));
            std::fs::create_dir_all(&dir).map_err(|e| io_err("Could not prepare the script", e))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            }
            let file = dir.join(&name);
            std::fs::write(&file, bytes).map_err(|e| io_err("Could not prepare the script", e))?;
            let home = localfs::real_path(&localfs::home(), cfg!(windows))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            (file, home, Some(dir))
        } else {
            let path = self.script_path(id)?;
            if !path.is_file() {
                return Err(ApiError::new("not_found", "The script file was not found."));
            }
            let cwd = path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            (path, cwd, None)
        };
        let script = path.to_string_lossy().into_owned();
        let (program, args) = localshell::script_command(
            &ext_of(&script),
            cfg!(windows),
            &self.all_profiles(),
            &script,
        )
        .map_err(|m| {
            if let Some(d) = &cleanup {
                let _ = std::fs::remove_dir_all(d);
            }
            ApiError::new("unsupported", m)
        })?;
        match self.open_local(&program, args, &cwd, params, cleanup.clone()) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(d) = &cleanup {
                    let _ = std::fs::remove_dir_all(d);
                }
                Err(e)
            }
        }
    }

    // ------------------------------------------------------------ local explorer

    pub(super) fn lfs_list(&self, params: &Value) -> Result<Value, ApiError> {
        let path = params.get("path").and_then(Value::as_str);
        localfs::list(path)
            .map(|(p, entries)| json!({ "path": p, "entries": entries }))
            .map_err(|m| ApiError::new("io", m))
    }

    pub(super) fn lfs(&self, params: &Value) -> Result<Value, ApiError> {
        let s = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_owned);
        let need = |k: &str| {
            s(k).ok_or_else(|| ApiError::new("invalid", format!("Missing parameter {k}")))
        };
        let fail = |m: String| ApiError::new("io", m);
        let paths: Vec<String> = params
            .get("paths")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let op = need("op")?;
        match op.as_str() {
            "mkdir" => localfs::mkdir(&need("parent")?, &need("name")?)
                .map(|p| json!({ "path": p }))
                .map_err(fail),
            "touch" => localfs::create_file(&need("parent")?, &need("name")?)
                .map(|p| json!({ "path": p }))
                .map_err(fail),
            "rename" => localfs::rename(&need("path")?, &need("name")?)
                .map(|p| json!({ "path": p }))
                .map_err(fail),
            "count" => {
                let (mut files, mut dirs) = (0, 0);
                for p in &paths {
                    let (f, d) = localfs::count_tree(p).map_err(fail)?;
                    files += f;
                    dirs += d;
                }
                Ok(json!({ "files": files, "dirs": dirs }))
            }
            "move" | "copy" | "delete" => {
                let dest = if op == "delete" {
                    String::new()
                } else {
                    need("dest")?
                };
                let (mut done, mut errors) = (Vec::new(), Vec::new());
                for p in &paths {
                    let r = match op.as_str() {
                        "move" => localfs::move_entry(p, &dest),
                        "copy" => localfs::copy_entry(p, &dest),
                        _ => localfs::delete_entry(p).map(|_| p.clone()),
                    };
                    match r {
                        Ok(n) => done.push(n),
                        Err(m) => {
                            errors.push(json!({ "path": p, "message": m }));
                            if op == "delete" {
                                break; // stop at the first error, as the SFTP explorer does
                            }
                        }
                    }
                }
                Ok(json!({ "done": done, "errors": errors }))
            }
            _ => Err(ApiError::new("invalid", format!("Unknown operation: {op}"))),
        }
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
