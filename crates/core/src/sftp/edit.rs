//! Backend of the built-in editor: open a text file with everything needed to write it back byte-exactly,
//! and save it through the safe write procedure after a conflict check. No commands, no sudo: SFTP only.
use super::conn::{describe, SftpConnection};
use super::xfer::{WriteOpts, XferError};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

pub struct EditError {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

impl EditError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }
}

type R<T> = Result<T, EditError>;

/// Line ending style found in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eol {
    Lf,
    Crlf,
    /// Both styles (or lone CR) are present.
    Mixed,
}

impl Eol {
    pub fn name(self) -> &'static str {
        match self {
            Eol::Lf => "lf",
            Eol::Crlf => "crlf",
            Eol::Mixed => "mixed",
        }
    }
}

/// Everything learnt from the raw bytes of a text file.
#[derive(Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Text with every line ending normalised to `\n` (what the editor shows).
    pub text: String,
    pub bom: bool,
    pub eol: Eol,
}

/// Decode a file for editing. `Err` carries the reason it is not editable (binary / not UTF-8).
pub fn decode(bytes: &[u8]) -> Result<Decoded, &'static str> {
    if bytes.contains(&0) {
        return Err("binary");
    }
    let (bom, body) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    let s = std::str::from_utf8(body).map_err(|_| "not_utf8")?;
    let crlf = s.matches("\r\n").count();
    let lf_total = s.matches('\n').count();
    let cr_total = s.matches('\r').count();
    let eol = if crlf == 0 && cr_total == 0 {
        Eol::Lf
    } else if crlf == lf_total && crlf == cr_total {
        Eol::Crlf
    } else {
        Eol::Mixed
    };
    let text = if crlf > 0 {
        s.replace("\r\n", "\n")
    } else {
        s.to_string()
    };
    Ok(Decoded { text, bom, eol })
}

/// Inverse of [`decode`]: restore the line endings and the BOM. `eol` must be `Lf` or `Crlf`.
pub fn encode(text: &str, bom: bool, eol: Eol) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 3);
    if bom {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    // the editor may contain stray "\r\n" (typed or pasted): normalise first, then apply the chosen style
    let norm = text.replace("\r\n", "\n");
    if eol == Eol::Crlf {
        out.extend_from_slice(norm.replace('\n', "\r\n").as_bytes());
    } else {
        out.extend_from_slice(norm.as_bytes());
    }
    out
}

/// Line endings of `s` and the text normalised to `\n`.
fn analyse(s: &str) -> (String, Eol) {
    let crlf = s.matches("\r\n").count();
    let lf_total = s.matches('\n').count();
    let cr_total = s.matches('\r').count();
    let eol = if crlf == 0 && cr_total == 0 {
        Eol::Lf
    } else if crlf == lf_total && crlf == cr_total {
        Eol::Crlf
    } else {
        Eol::Mixed
    };
    let text = if crlf > 0 {
        s.replace("\r\n", "\n")
    } else {
        s.to_string()
    };
    (text, eol)
}

/// A file opened in some encoding (see `codec`).
pub struct Opened {
    pub text: String,
    pub bom: bool,
    pub eol: Eol,
    pub encoding: &'static str,
    pub guessed: bool,
}

/// Decode for the editor in the forced encoding (or detect it). Error = message for the user.
pub fn open_bytes(bytes: &[u8], forced: Option<&str>) -> Result<Opened, super::codec::DecodeFail> {
    let d = super::codec::decode(bytes, forced)?;
    let (text, eol) = analyse(&d.text);
    Ok(Opened {
        text,
        bom: d.bom,
        eol,
        encoding: d.encoding.name(),
        guessed: d.guessed,
    })
}

/// Inverse of [`open_bytes`]. `Err(c)` = the character `c` does not exist in the encoding.
pub fn encode_with(text: &str, bom: bool, eol: Eol, encoding: &str) -> Result<Vec<u8>, char> {
    let enc = super::codec::by_name(encoding).unwrap_or(encoding_rs::UTF_8);
    let norm = text.replace("\r\n", "\n");
    let body = if eol == Eol::Crlf {
        norm.replace('\n', "\r\n")
    } else {
        norm
    };
    super::codec::encode(&body, bom, enc)
}

pub fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// The version of the file the editor started from.
#[derive(Debug, Clone, Default)]
pub struct Base {
    pub sha256: String,
    pub size: u64,
    pub mtime: Option<u32>,
    pub mode: Option<u32>,
    pub uid: Option<u32>,
}

impl Base {
    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            sha256: v.get("sha256")?.as_str()?.to_string(),
            size: v.get("size")?.as_u64()?,
            mtime: v.get("mtime").and_then(Value::as_u64).map(|x| x as u32),
            mode: v.get("mode").and_then(Value::as_u64).map(|x| x as u32),
            uid: v.get("uid").and_then(Value::as_u64).map(|x| x as u32),
        })
    }

    pub fn to_json(&self) -> Value {
        json!({ "sha256": self.sha256, "size": self.size, "mtime": self.mtime, "mode": self.mode, "uid": self.uid })
    }
}

fn sftp_code(e: &russh_sftp::client::error::Error) -> Option<russh_sftp::protocol::StatusCode> {
    match e {
        russh_sftp::client::error::Error::Status(s) => Some(s.status_code),
        _ => None,
    }
}

impl SftpConnection {
    /// Read a whole file (the size was checked before).
    async fn read_all(&self, path: &str) -> R<Vec<u8>> {
        let mut f = self
            .sftp
            .open(path.to_string())
            .await
            .map_err(|e| EditError::new("sftp", describe(e)))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)
            .await
            .map_err(|e| EditError::new("sftp", format!("Read: {e}")))?;
        Ok(buf)
    }

    fn perm_msg(&self, path: &str) -> String {
        format!("{path}: no write permission for user {}.", self.user)
    }

    /// Open `path` for editing. `new` = the file must not exist yet (it is created by the first save).
    pub async fn edit_open(&self, path: &str, limit: u64, new: bool) -> R<Value> {
        self.edit_open_enc(path, limit, new, None).await
    }

    /// Like `edit_open`; `encoding` = the encoding chosen by the user (None: detect).
    pub async fn edit_open_enc(
        &self,
        path: &str,
        limit: u64,
        new: bool,
        encoding: Option<&str>,
    ) -> R<Value> {
        if !path.starts_with('/') {
            return Err(EditError::new("invalid", "Invalid path."));
        }
        if new {
            if self
                .exists_lstat(path)
                .await
                .map_err(|e| EditError::new("sftp", e.to_string()))?
                .is_some()
            {
                return Err(EditError::new(
                    "exists",
                    format!("“{}” already exists.", crate::remote_path::file_name(path)),
                ));
            }
            return Ok(json!({
                "path": path, "text": "", "bom": false, "eol": "lf", "base": null, "new": true,
                "encoding": "UTF-8", "guessed": false,
            }));
        }
        let m = self
            .sftp
            .metadata(path.to_string())
            .await
            .map_err(|e| EditError::new("sftp", describe(e)))?;
        if !m.file_type().is_file() {
            return Err(EditError::new("not_file", "This is not a regular file."));
        }
        let size = m.size.unwrap_or(0);
        if size > limit {
            return Err(EditError::new(
                "too_large",
                format!(
                    "The file is too large for the editor ({} MB, limit {} MB). Download it.",
                    size / (1024 * 1024),
                    limit / (1024 * 1024)
                ),
            ));
        }
        let bytes = self.read_all(path).await?;
        let d = open_bytes(&bytes, encoding).map_err(|f| {
            let code = if f == super::codec::DecodeFail::Binary {
                "binary"
            } else {
                "encoding"
            };
            EditError::new(code, super::codec::fail_message(&f))
        })?;
        let base = Base {
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
            mtime: m.mtime,
            mode: m.permissions,
            uid: m.uid,
        };
        Ok(json!({
            "path": path, "text": d.text, "bom": d.bom, "eol": d.eol.name(),
            "encoding": d.encoding, "guessed": d.guessed,
            "base": base.to_json(), "new": false,
            "writable_hint": m.permissions.map(|p| p & 0o200 != 0),
        }))
    }

    /// Save in UTF-8 (see `edit_save_enc`).
    #[allow(clippy::too_many_arguments)]
    pub async fn edit_save(
        &self,
        path: &str,
        text: &str,
        bom: bool,
        eol: Eol,
        base: Option<&Base>,
        limit: u64,
        force: bool,
        allow_foreign: bool,
        in_place: bool,
    ) -> R<Value> {
        self.edit_save_enc(
            path,
            text,
            bom,
            eol,
            "UTF-8",
            base,
            limit,
            force,
            allow_foreign,
            in_place,
        )
        .await
    }

    /// Save. `base` = None for a new file. `force` skips the conflict check (user chose "overwrite anyway").
    #[allow(clippy::too_many_arguments)]
    pub async fn edit_save_enc(
        &self,
        path: &str,
        text: &str,
        bom: bool,
        eol: Eol,
        encoding: &str,
        base: Option<&Base>,
        limit: u64,
        force: bool,
        allow_foreign: bool,
        in_place: bool,
    ) -> R<Value> {
        if !path.starts_with('/') {
            return Err(EditError::new("invalid", "Invalid path."));
        }
        let bytes = encode_with(text, bom, if eol == Eol::Mixed { Eol::Lf } else { eol }, encoding).map_err(|c| {
            let mut e = EditError::new(
                "unmappable",
                format!("Character “{c}” (U+{:04X}) cannot be written in the encoding {encoding}. Remove it or choose another encoding (for example, UTF-8).", c as u32),
            );
            e.details = Some(json!({ "char": c.to_string() }));
            e
        })?;
        let cur = self
            .exists_lstat(path)
            .await
            .map_err(|e| EditError::new("sftp", e.to_string()))?;
        // follow a symlink for the comparison: the editor edits the real file
        let cur_meta = match &cur {
            Some(m) if m.file_type().is_symlink() => {
                self.sftp.metadata(path.to_string()).await.ok()
            }
            other => other.clone(),
        };
        match (&cur_meta, base) {
            (None, Some(_)) if !force => {
                return Err(EditError::new(
                    "deleted",
                    "The file was deleted on the server. Create it again?",
                ));
            }
            (Some(_), None) if !force => {
                return Err(EditError::new(
                    "exists",
                    "A file with this name has appeared on the server: it will not be overwritten.",
                ));
            }
            (Some(m), Some(b)) if !force => {
                let changed = m.size.unwrap_or(0) != b.size
                    || if m.size.unwrap_or(0) <= limit {
                        let now = self.read_all(path).await?;
                        sha256_hex(&now) != b.sha256
                    } else {
                        m.mtime != b.mtime
                    };
                if changed {
                    let remote_text = if m.size.unwrap_or(0) <= limit {
                        self.read_all(path)
                            .await
                            .ok()
                            .and_then(|raw| open_bytes(&raw, Some(encoding)).ok())
                            .map(|d| d.text)
                    } else {
                        None
                    };
                    let mut e = EditError::new(
                        "conflict",
                        "The file changed on the server since it was opened.",
                    );
                    e.details = Some(json!({ "remote_text": remote_text }));
                    return Err(e);
                }
            }
            _ => {}
        }

        let overwrite = cur_meta.is_some();
        let o = WriteOpts {
            overwrite,
            allow_foreign,
            in_place,
            mtime: None,
        };
        let mut src = std::io::Cursor::new(bytes.clone());
        match self
            .write_safe(path, &mut src, bytes.len() as u64, &o, None)
            .await
        {
            Ok(()) => {}
            Err(XferError::ForeignOwner) => {
                let mut e = EditError::new(
                    "foreign_owner",
                    "The file belongs to another user: an atomic replacement will change the owner. Write in place (not atomic) or cancel?",
                );
                e.details = Some(json!({ "can_in_place": true }));
                return Err(e);
            }
            Err(XferError::NoPerm) => {
                if in_place || !overwrite {
                    return Err(EditError::new("no_permission", self.perm_msg(path)));
                }
                let mut e = EditError::new(
                    "dir_not_writable",
                    format!("Cannot write to the folder of the file for user {}, so a safe (atomic) replacement is impossible. You can write in place: this is not atomic and the file may be damaged if the connection drops.", self.user),
                );
                e.details = Some(json!({ "can_in_place": true }));
                return Err(e);
            }
            Err(XferError::Exists) => {
                return Err(EditError::new(
                    "exists",
                    "The file already exists and was not overwritten.",
                ))
            }
            Err(XferError::Cancelled) => return Err(EditError::new("cancelled", "Cancelled.")),
            Err(XferError::Msg(m)) => return Err(EditError::new("sftp", m)),
        }
        let m = self
            .sftp
            .metadata(path.to_string())
            .await
            .map_err(|e| EditError::new("sftp", describe(e)))?;
        let _ = sftp_code;
        let new_base = Base {
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
            mtime: m.mtime,
            mode: m.permissions,
            uid: m.uid,
        };
        Ok(json!({ "base": new_base.to_json() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(bytes: &[u8]) {
        let d = decode(bytes).unwrap();
        let eol = if d.eol == Eol::Mixed { Eol::Lf } else { d.eol };
        if d.eol != Eol::Mixed {
            assert_eq!(encode(&d.text, d.bom, eol), bytes, "{bytes:?}");
        }
    }

    #[test]
    fn detects_line_endings() {
        assert_eq!(decode(b"a\nb\n").unwrap().eol, Eol::Lf);
        assert_eq!(decode(b"a\r\nb\r\n").unwrap().eol, Eol::Crlf);
        assert_eq!(decode(b"a\r\nb\n").unwrap().eol, Eol::Mixed);
        assert_eq!(decode(b"a\rb").unwrap().eol, Eol::Mixed);
        assert_eq!(decode(b"no newline").unwrap().eol, Eol::Lf);
        assert_eq!(decode(b"").unwrap().eol, Eol::Lf);
    }

    #[test]
    fn text_is_normalised_for_the_editor() {
        assert_eq!(decode(b"a\r\nb\r\n").unwrap().text, "a\nb\n");
    }

    #[test]
    fn bom_and_binary_and_encoding() {
        let d = decode(b"\xEF\xBB\xBFhi\n").unwrap();
        assert!(d.bom && d.text == "hi\n");
        assert_eq!(decode(b"a\0b"), Err("binary"));
        assert_eq!(decode(b"\xff\xfe\xfa"), Err("not_utf8"));
        assert_eq!(
            decode("юникод ✓ 🙂".as_bytes()).unwrap().text,
            "юникод ✓ 🙂"
        );
    }

    #[test]
    fn roundtrips_are_byte_exact() {
        for b in [
            &b"plain\n"[..],
            b"no final newline",
            b"",
            b"\n",
            b"crlf\r\nlines\r\n",
            b"crlf no final\r\nx",
            "\u{feff}bom юникод\r\nline\r\n".as_bytes(),
            "bom lf\nend".as_bytes(),
            b"\xEF\xBB\xBF",
            b"trailing spaces  \n\n\n",
        ] {
            roundtrip(b);
        }
    }

    #[test]
    fn mixed_can_be_saved_in_a_chosen_style() {
        let d = decode(b"a\r\nb\nc").unwrap();
        assert_eq!(encode(&d.text, false, Eol::Lf), b"a\nb\nc");
        assert_eq!(encode(&d.text, false, Eol::Crlf), b"a\r\nb\r\nc");
    }

    #[test]
    fn stray_crlf_typed_in_the_editor_follows_the_chosen_style() {
        assert_eq!(encode("a\r\nb\n", false, Eol::Lf), b"a\nb\n");
        assert_eq!(encode("a\r\nb\n", false, Eol::Crlf), b"a\r\nb\r\n");
    }

    #[test]
    fn sha256_known_value() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
