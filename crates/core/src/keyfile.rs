//! Private key files for key login: path handling and early, readable diagnostics.
use std::path::PathBuf;

/// `~/key` -> `<home>/key` (HOME, or USERPROFILE on Windows). Other paths are returned unchanged.
pub fn expand_home(p: &str) -> PathBuf {
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()));
    expand_with(p, home.as_deref())
}

fn expand_with(p: &str, home: Option<&str>) -> PathBuf {
    let p = p.trim();
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        if let Some(h) = home {
            return PathBuf::from(h).join(rest);
        }
    }
    PathBuf::from(p)
}

/// PuTTY's own format (`.ppk`): neither OpenSSH nor this program can use it directly.
pub fn looks_like_ppk(head: &str) -> bool {
    head.trim_start().starts_with("PuTTY-User-Key-File")
}

/// Check the file before any connection is made. `Err` is a message for the user.
pub fn check(path: &str) -> Result<PathBuf, String> {
    let p = expand_home(path);
    if path.trim().is_empty() {
        return Err("No private key file specified.".into());
    }
    if !p.is_file() {
        return Err(format!("Private key file not found: {}", p.display()));
    }
    let mut head = String::new();
    {
        use std::io::Read;
        let mut f = std::fs::File::open(&p)
            .map_err(|e| format!("Could not read the key {}: {e}", p.display()))?;
        let mut buf = [0u8; 64];
        let n = f.read(&mut buf).unwrap_or(0);
        head.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    if looks_like_ppk(&head) {
        return Err(format!(
            "File {} is in PuTTY format (.ppk): it is not supported. Convert it to OpenSSH format (PuTTYgen: Conversions → Export OpenSSH key, or `puttygen key.ppk -O private-openssh -o key`).",
            p.display()
        ));
    }
    if head.contains("PUBLIC KEY") || head.starts_with("ssh-") || head.starts_with("ecdsa-") {
        return Err(format!(
            "{} is a public key (.pub). A PRIVATE key file is needed.",
            p.display()
        ));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_is_expanded_other_paths_are_kept() {
        let h = Some("/home/u");
        assert_eq!(expand_with("~/k/id", h), PathBuf::from("/home/u/k/id"));
        assert_eq!(expand_with("/abs/id", h), PathBuf::from("/abs/id"));
        assert_eq!(expand_with("rel/id", h), PathBuf::from("rel/id"));
        assert_eq!(expand_with("  /x  ", h), PathBuf::from("/x"));
        assert_eq!(expand_with("~/k", None), PathBuf::from("~/k"));
    }

    #[test]
    fn check_explains_the_usual_mistakes() {
        let d = std::env::temp_dir().join(format!("d256shell-key-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let w = |n: &str, c: &str| {
            let p = d.join(n);
            std::fs::write(&p, c).unwrap();
            p.to_string_lossy().into_owned()
        };
        assert!(check(&d.join("missing").to_string_lossy())
            .unwrap_err()
            .contains("not found"));
        assert!(check("").unwrap_err().contains("specified"));
        assert!(check(&w("a.ppk", "PuTTY-User-Key-File-3: ssh-ed25519\n"))
            .unwrap_err()
            .contains(".ppk"));
        assert!(check(&w("a.pub", "ssh-ed25519 AAAA user@h\n"))
            .unwrap_err()
            .contains("public"));
        assert!(check(&w("ok", "-----BEGIN OPENSSH PRIVATE KEY-----\nx\n")).is_ok());
        std::fs::remove_dir_all(d).unwrap();
    }
}
