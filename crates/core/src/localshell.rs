//! Local shells (cmd, PowerShell, WSL, Git Bash, bash/zsh/fish ...) that run in tabs next to the SSH sessions.
//! Nothing is installed or started to find them: candidates are looked up on disk and in `PATH`; only the list of
//! WSL distributions comes from `wsl.exe -l -q` (a local, read-only listing).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A shell the user can open in a tab.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellProfile {
    /// Stable id: `auto:<name>` for detected shells, `custom:<id>` for the user's own.
    pub id: String,
    pub name: String,
    /// "powershell" | "pwsh" | "cmd" | "wsl" | "gitbash" | "unix" | "custom": picks the default icon.
    pub kind: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Start folder; empty = the user's home.
    #[serde(default)]
    pub cwd: String,
    pub custom: bool,
}

/// A profile the user defined in Settings (stored in settings.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CustomProfile {
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
}

/// Where to look: injected so the Windows logic can be tested on any host.
pub struct Probe<'a> {
    pub windows: bool,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub exists: &'a dyn Fn(&str) -> bool,
    /// Names printed by `wsl.exe -l -q` (already decoded); empty when WSL is not installed.
    pub wsl_distros: Vec<String>,
    /// Content of /etc/shells (Unix only).
    pub etc_shells: String,
}

fn join_win(a: &str, b: &str) -> String {
    format!("{}\\{}", a.trim_end_matches(['\\', '/']), b)
}

fn profile(id: &str, name: &str, kind: &str, program: &str, args: &[&str]) -> ShellProfile {
    ShellProfile {
        id: format!("auto:{id}"),
        name: name.to_string(),
        kind: kind.to_string(),
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: String::new(),
        custom: false,
    }
}

/// Shells found on this computer, in a stable, sensible order.
pub fn detect(p: &Probe) -> Vec<ShellProfile> {
    let mut out: Vec<ShellProfile> = Vec::new();
    if p.windows {
        let sys = (p.env)("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let pf = (p.env)("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into());
        let pf86 = (p.env)("ProgramFiles(x86)");
        let local = (p.env)("LOCALAPPDATA");
        let ps5 = join_win(&sys, "System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        if (p.exists)(&ps5) {
            out.push(profile(
                "powershell",
                "Windows PowerShell",
                "powershell",
                &ps5,
                &["-NoLogo"],
            ));
        }
        let mut pwsh = vec![join_win(&pf, "PowerShell\\7\\pwsh.exe")];
        if let Some(l) = &local {
            pwsh.push(join_win(l, "Microsoft\\WindowsApps\\pwsh.exe"));
        }
        if let Some(found) = pwsh.iter().find(|c| (p.exists)(c)) {
            out.push(profile("pwsh", "PowerShell 7", "pwsh", found, &["-NoLogo"]));
        }
        let cmd = (p.env)("ComSpec").unwrap_or_else(|| join_win(&sys, "System32\\cmd.exe"));
        if (p.exists)(&cmd) {
            out.push(profile("cmd", "Command Prompt", "cmd", &cmd, &[]));
        }
        let wsl = join_win(&sys, "System32\\wsl.exe");
        if (p.exists)(&wsl) {
            for d in &p.wsl_distros {
                out.push(profile(
                    &format!("wsl:{d}"),
                    d,
                    "wsl",
                    &wsl,
                    &["-d", d.as_str()],
                ));
            }
        }
        let mut git = vec![join_win(&pf, "Git\\bin\\bash.exe")];
        if let Some(x) = &pf86 {
            git.push(join_win(x, "Git\\bin\\bash.exe"));
        }
        if let Some(l) = &local {
            git.push(join_win(l, "Programs\\Git\\bin\\bash.exe"));
        }
        if let Some(found) = git.iter().find(|c| (p.exists)(c)) {
            out.push(profile(
                "gitbash",
                "Git Bash",
                "gitbash",
                found,
                &["--login", "-i"],
            ));
        }
    } else {
        let mut seen: Vec<String> = Vec::new();
        let mut add = |path: &str, out: &mut Vec<ShellProfile>| {
            let base = Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if base.is_empty() || seen.contains(&base) || !(p.exists)(path) {
                return;
            }
            seen.push(base.clone());
            out.push(profile(&base, &base, "unix", path, &["-l"]));
        };
        if let Some(s) = (p.env)("SHELL") {
            add(&s, &mut out);
        }
        for line in p.etc_shells.lines() {
            let l = line.trim();
            if l.starts_with('/') && !l.ends_with("nologin") && !l.ends_with("/false") {
                add(l, &mut out);
            }
        }
        // the login shell first, then the rest alphabetically
        if out.len() > 2 {
            out[1..].sort_by(|a, b| a.name.cmp(&b.name));
        }
    }
    out
}

/// `wsl.exe -l -q` prints UTF-16LE (often with a BOM and NULs in between): decode it into distribution names.
pub fn parse_wsl_list(bytes: &[u8]) -> Vec<String> {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let text = String::from_utf16_lossy(&units);
    text.lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}').trim().to_string())
        .filter(|l| !l.is_empty() && !l.contains('\u{fffd}'))
        .collect()
}

/// Look at the real system (called by the service).
pub fn detect_system() -> Vec<ShellProfile> {
    let windows = cfg!(windows);
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let exists = |p: &str| Path::new(p).is_file();
    let wsl_distros = if windows { list_wsl() } else { Vec::new() };
    let etc_shells = if windows {
        String::new()
    } else {
        std::fs::read_to_string("/etc/shells").unwrap_or_default()
    };
    detect(&Probe {
        windows,
        env: &env,
        exists: &exists,
        wsl_distros,
        etc_shells,
    })
}

fn list_wsl() -> Vec<String> {
    let sys = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let wsl = PathBuf::from(sys).join("System32").join("wsl.exe");
    if !wsl.is_file() {
        return Vec::new();
    }
    // a listing only: it does not start any distribution
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new(&wsl);
        cmd.args(["-l", "-q"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let _ = tx.send(cmd.output().ok().map(|o| o.stdout));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(4)) {
        Ok(Some(bytes)) => parse_wsl_list(&bytes),
        _ => Vec::new(),
    }
}

/// Custom profiles from settings as profiles; entries without a program are ignored.
pub fn custom_profiles(list: &[CustomProfile]) -> Vec<ShellProfile> {
    list.iter()
        .filter(|c| !c.program.trim().is_empty() && !c.id.is_empty())
        .map(|c| ShellProfile {
            id: format!("custom:{}", c.id),
            name: if c.name.trim().is_empty() {
                c.program.clone()
            } else {
                c.name.clone()
            },
            kind: "custom".into(),
            program: c.program.clone(),
            args: c.args.clone(),
            cwd: c.cwd.clone(),
            custom: true,
        })
        .collect()
}

/// How a script file is run: the interpreter and its arguments (the script path is the last argument).
pub fn script_command(
    ext: &str,
    windows: bool,
    profiles: &[ShellProfile],
    script: &str,
) -> Result<(String, Vec<String>), String> {
    let by_kind = |k: &str| profiles.iter().find(|p| p.kind == k);
    let ext = ext.trim_start_matches('.').to_lowercase();
    match ext.as_str() {
        "bat" | "cmd" => {
            if !windows {
                return Err("Batch files (.bat) run only on Windows.".into());
            }
            let cmd = by_kind("cmd").ok_or("cmd.exe was not found.")?;
            Ok((cmd.program.clone(), vec!["/K".into(), script.into()]))
        }
        "ps1" => {
            let ps = by_kind("pwsh")
                .or_else(|| by_kind("powershell"))
                .ok_or("PowerShell was not found.")?;
            Ok((
                ps.program.clone(),
                vec![
                    "-NoLogo".into(),
                    "-NoExit".into(),
                    "-File".into(),
                    script.into(),
                ],
            ))
        }
        "sh" | "bash" => {
            if windows {
                let git = by_kind("gitbash").ok_or(
                    "Shell scripts (.sh) need Git Bash on Windows (WSL scripts: open a WSL terminal and run the file there).",
                )?;
                Ok((git.program.clone(), vec!["--login".into(), script.into()]))
            } else {
                let sh = profiles
                    .iter()
                    .find(|p| p.name == "bash")
                    .or_else(|| profiles.iter().find(|p| p.name == "sh"))
                    .or_else(|| profiles.first())
                    .ok_or("No shell was found.")?;
                Ok((sh.program.clone(), vec![script.into()]))
            }
        }
        other => Err(format!("Files of type “.{other}” cannot be run.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn win(files: &[&str], distros: &[&str]) -> Vec<ShellProfile> {
        let set: HashSet<String> = files.iter().map(|s| s.to_string()).collect();
        let env = |k: &str| match k {
            "SystemRoot" => Some("C:\\Windows".to_string()),
            "ProgramFiles" => Some("C:\\Program Files".to_string()),
            "LOCALAPPDATA" => Some("C:\\Users\\u\\AppData\\Local".to_string()),
            _ => None,
        };
        let exists = |p: &str| set.contains(p);
        detect(&Probe {
            windows: true,
            env: &env,
            exists: &exists,
            wsl_distros: distros.iter().map(|s| s.to_string()).collect(),
            etc_shells: String::new(),
        })
    }

    #[test]
    fn windows_finds_only_what_exists() {
        let none = win(&[], &[]);
        assert!(none.is_empty());
        let all = win(
            &[
                "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
                "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
                "C:\\Windows\\System32\\cmd.exe",
                "C:\\Windows\\System32\\wsl.exe",
                "C:\\Program Files\\Git\\bin\\bash.exe",
            ],
            &["Ubuntu-24.04", "archlinux"],
        );
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Windows PowerShell",
                "PowerShell 7",
                "Command Prompt",
                "Ubuntu-24.04",
                "archlinux",
                "Git Bash"
            ]
        );
        let wsl = all.iter().find(|p| p.name == "archlinux").unwrap();
        assert_eq!(wsl.args, ["-d", "archlinux"]);
        assert_eq!(wsl.id, "auto:wsl:archlinux");
        // WSL without wsl.exe: the distributions are not offered
        let no_wsl = win(&["C:\\Windows\\System32\\cmd.exe"], &["Ubuntu"]);
        assert_eq!(no_wsl.len(), 1);
    }

    #[test]
    fn wsl_list_is_utf16_with_bom_and_blank_lines() {
        let text = "\u{feff}Ubuntu-24.04\r\narchlinux\r\n\r\n";
        let bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_wsl_list(&bytes), ["Ubuntu-24.04", "archlinux"]);
        assert!(parse_wsl_list(b"").is_empty());
    }

    #[test]
    fn unix_lists_login_shell_first_and_skips_nologin_and_duplicates() {
        let env = |k: &str| (k == "SHELL").then(|| "/usr/bin/zsh".to_string());
        let exists = |p: &str| {
            [
                "/usr/bin/zsh",
                "/bin/bash",
                "/usr/bin/fish",
                "/usr/bin/bash",
            ]
            .contains(&p)
        };
        let v = detect(&Probe {
            windows: false,
            env: &env,
            exists: &exists,
            wsl_distros: vec![],
            etc_shells:
                "# list\n/bin/bash\n/usr/bin/bash\n/usr/sbin/nologin\n/usr/bin/fish\n/usr/bin/zsh\n"
                    .into(),
        });
        let names: Vec<&str> = v.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["zsh", "bash", "fish"]);
        assert!(v.iter().all(|p| p.kind == "unix" && !p.custom));
    }

    #[test]
    fn custom_profiles_need_an_id_and_a_program() {
        let v = custom_profiles(&[
            CustomProfile {
                id: "a".into(),
                name: "".into(),
                program: "python".into(),
                ..Default::default()
            },
            CustomProfile {
                id: "b".into(),
                name: "x".into(),
                program: " ".into(),
                ..Default::default()
            },
            CustomProfile {
                id: "".into(),
                name: "y".into(),
                program: "z".into(),
                ..Default::default()
            },
        ]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "custom:a");
        assert_eq!(v[0].name, "python");
    }

    #[test]
    fn scripts_pick_the_right_interpreter() {
        let profiles = win(
            &[
                "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
                "C:\\Windows\\System32\\cmd.exe",
                "C:\\Program Files\\Git\\bin\\bash.exe",
            ],
            &[],
        );
        let (p, a) = script_command("bat", true, &profiles, "C:\\a b\\x.bat").unwrap();
        assert!(p.ends_with("cmd.exe"));
        assert_eq!(a, ["/K", "C:\\a b\\x.bat"]);
        let (p, a) = script_command(".ps1", true, &profiles, "x.ps1").unwrap();
        assert!(p.ends_with("powershell.exe"));
        assert_eq!(a.last().unwrap(), "x.ps1");
        assert!(a.contains(&"-File".to_string()));
        let (p, _) = script_command("sh", true, &profiles, "x.sh").unwrap();
        assert!(p.ends_with("bash.exe"));
        assert!(script_command("bat", false, &[], "x.bat").is_err());
        assert!(script_command("py", true, &profiles, "x.py").is_err());
        assert!(script_command("ps1", true, &[], "x.ps1").is_err());
    }
}
