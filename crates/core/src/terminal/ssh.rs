//! Locating the system `ssh` and reading its version.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshVersion {
    pub major: u32,
    pub minor: u32,
    /// The first line of `ssh -V` output, trimmed.
    pub raw: String,
}

impl SshVersion {
    /// `SSH_ASKPASS_REQUIRE=force` exists since OpenSSH 8.4.
    pub fn supports_askpass_force(&self) -> bool {
        self.major > 8 || (self.major == 8 && self.minor >= 4)
    }
}

/// Parse `ssh -V` output (written to stderr), e.g.
/// `OpenSSH_for_Windows_9.5p1, LibreSSL 3.8.2`, `OpenSSH_9.9p2 Debian-3, OpenSSL 3.0`, `OpenSSH_8.1p1`.
pub fn parse_version(output: &str) -> Option<SshVersion> {
    let line = output.lines().find(|l| l.contains("OpenSSH_"))?.trim();
    let after = &line[line.find("OpenSSH_")? + "OpenSSH_".len()..];
    // skip "for_Windows_" style infix: take the first token that starts with a digit
    let ver = after
        .split(|c: char| c == '_' || c == ',' || c.is_whitespace())
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut it = ver.split(|c: char| !c.is_ascii_digit());
    let major: u32 = it.next()?.parse().ok()?;
    let minor: u32 = it.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some(SshVersion {
        major,
        minor,
        raw: line.to_string(),
    })
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ssh.exe"
    } else {
        "ssh"
    }
}

/// Search order: explicit setting -> `<data>/OpenSSH/ssh.exe` -> `%SystemRoot%\System32\OpenSSH\ssh.exe` -> PATH.
pub fn find_ssh(configured: &str, data_openssh_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !configured.trim().is_empty() {
        candidates.push(PathBuf::from(configured.trim()));
    }
    candidates.push(data_openssh_dir.join(exe_name()));
    if let Some(root) = std::env::var_os("SystemRoot") {
        candidates.push(
            PathBuf::from(root)
                .join("System32/OpenSSH")
                .join(exe_name()),
        );
    }
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            candidates.push(d.join(exe_name()));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Run `ssh -V` (no console window on Windows) and parse it.
pub fn detect(ssh: &Path) -> Result<SshVersion, String> {
    let mut cmd = std::process::Command::new(ssh);
    cmd.arg("-V");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .output()
        .map_err(|e| format!("не удалось запустить {}: {e}", ssh.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    parse_version(&text).ok_or_else(|| format!("не удалось определить версию ssh: {}", text.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Option<(u32, u32)> {
        parse_version(s).map(|v| (v.major, v.minor))
    }

    #[test]
    fn parses_known_formats() {
        assert_eq!(v("OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2"), Some((8, 1)));
        assert_eq!(v("OpenSSH_for_Windows_9.5p1, LibreSSL 3.8.2"), Some((9, 5)));
        assert_eq!(v("OpenSSH_9.9p2, OpenSSL 3.0.15 3 Sep 2024"), Some((9, 9)));
        assert_eq!(
            v("OpenSSH_9.2p1 Debian-2+deb12u10, OpenSSL 3.0.22 25 Aug 2026\n"),
            Some((9, 2))
        );
        assert_eq!(v("OpenSSH_8.4p1"), Some((8, 4)));
        assert_eq!(v("OpenSSH_10.0"), Some((10, 0)));
        assert_eq!(v("some warning\nOpenSSH_7.9p1, OpenSSL"), Some((7, 9)));
    }

    #[test]
    fn rejects_garbage() {
        for s in [
            "",
            "garbage",
            "OpenSSH",
            "OpenSSH_",
            "OpenSSH_for_Windows_",
            "dropbear v2020.81",
        ] {
            assert_eq!(v(s), None, "{s:?}");
        }
    }

    #[test]
    fn askpass_force_threshold() {
        let f = |s| parse_version(s).unwrap().supports_askpass_force();
        assert!(!f("OpenSSH_for_Windows_8.1p1"));
        assert!(!f("OpenSSH_8.3p1"));
        assert!(f("OpenSSH_8.4p1"));
        assert!(f("OpenSSH_for_Windows_9.5p1"));
        assert!(f("OpenSSH_10.0p1"));
        assert!(!f("OpenSSH_7.9p1"));
    }

    #[test]
    fn find_prefers_configured_path() {
        let d = std::env::temp_dir().join(format!("d256shell-ssh-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let fake = d.join("my-ssh");
        std::fs::write(&fake, b"x").unwrap();
        assert_eq!(
            find_ssh(fake.to_str().unwrap(), &d.join("none")),
            Some(fake)
        );
        std::fs::remove_dir_all(d).unwrap();
    }
}
