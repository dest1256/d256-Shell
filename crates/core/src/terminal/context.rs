//! "What is the terminal doing right now?" — found without running anything on the server: the login shell of
//! this terminal is recognised by the client port in its `SSH_CONNECTION` / `SSH_CLIENT`, then `/proc` is read
//! over SFTP: working folder, foreground program and its environment, and — when the user went deeper with
//! `su - user`, `sudo -i`, `bash` … — the nested shell and the command that started it.
use crate::sftp::SftpConnection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::task::JoinSet;

/// Variables worth carrying into a copy (virtual environments).
const ENV_KEYS: [&str; 5] = [
    "VIRTUAL_ENV",
    "VIRTUAL_ENV_PROMPT",
    "CONDA_DEFAULT_ENV",
    "CONDA_PREFIX",
    "PYENV_VERSION",
];

const SHELLS: [&str; 9] = [
    "sh", "ash", "bash", "dash", "zsh", "fish", "ksh", "tcsh", "csh",
];

/// Client port the server saw for this login (`SSH_CONNECTION="ip port ip port"` or `SSH_CLIENT="ip port port"`).
pub fn ssh_client_port(environ: &str) -> Option<u16> {
    for v in environ.split('\0') {
        let rest = v
            .strip_prefix("SSH_CONNECTION=")
            .or_else(|| v.strip_prefix("SSH_CLIENT="));
        if let Some(r) = rest {
            return r.split_whitespace().nth(1).and_then(|p| p.parse().ok());
        }
    }
    None
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Stat {
    pub comm: String,
    pub ppid: i64,
    pub pgrp: i64,
    pub session: i64,
    pub tty_nr: i64,
    pub tpgid: i64,
}

/// `/proc/<pid>/stat`: the command name is in parentheses and may itself contain spaces and `)`.
pub fn parse_stat(text: &str) -> Option<Stat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text.get(open + 1..close)?.to_string();
    let f: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    // state ppid pgrp session tty_nr tpgid ...
    let n = |i: usize| f.get(i).and_then(|v| v.parse::<i64>().ok());
    Some(Stat {
        comm,
        ppid: n(1)?,
        pgrp: n(2)?,
        session: n(3)?,
        tty_nr: n(4)?,
        tpgid: n(5)?,
    })
}

/// Real user id from `/proc/<pid>/status`.
pub fn parse_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|v| v.parse().ok())
}

fn split_nul(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text.split('\0').map(str::to_owned).collect();
    while v.last().is_some_and(String::is_empty) {
        v.pop();
    }
    v
}

fn env_subset(environ: &str) -> BTreeMap<String, String> {
    environ
        .split('\0')
        .filter_map(|e| e.split_once('='))
        .filter(|(k, v)| ENV_KEYS.contains(k) && !v.is_empty())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn is_shell(comm: &str) -> bool {
    SHELLS.contains(&comm.trim_start_matches('-'))
}

/// Where the user really is: the chain of processes from the login shell down to the foreground program.
#[derive(Debug, PartialEq, Eq)]
pub struct Chain {
    /// The innermost interactive shell (the login shell itself when nothing is nested).
    pub inner: i64,
    /// The first process the login shell started on the way down (`su`, `sudo`, `bash` …), when nested.
    pub launcher: Option<i64>,
    /// The foreground program running in the innermost shell (None at its prompt).
    pub fg: Option<i64>,
}

/// Walk from the foreground process group leader `leader` up to the login shell `shell`.
pub fn analyze(stats: &HashMap<i64, Stat>, shell: i64, leader: i64) -> Chain {
    let mut path = vec![leader];
    let mut cur = leader;
    while cur != shell {
        match stats.get(&cur) {
            Some(s) if path.len() < 32 && s.ppid > 0 => {
                cur = s.ppid;
                path.push(cur);
            }
            _ => {
                // the foreground program is not a descendant of the shell (daemonised, sudo'd away ...)
                return Chain {
                    inner: shell,
                    launcher: None,
                    fg: (leader != shell).then_some(leader),
                };
            }
        }
    }
    path.reverse(); // shell, child, ..., leader
    let inner = path
        .iter()
        .skip(1)
        .rev()
        .find(|p| stats.get(p).is_some_and(|s| is_shell(&s.comm)))
        .copied()
        .unwrap_or(shell);
    let launcher = (inner != shell).then(|| path[1]);
    let fg = (leader != inner).then_some(leader);
    Chain {
        inner,
        launcher,
        fg,
    }
}

impl SftpConnection {
    /// Find the shell of the terminal whose `ssh` uses local port `local_port` and describe its state.
    pub async fn shell_context(self: &Arc<Self>, local_port: u16) -> Result<Value, String> {
        let names: Vec<String> = self
            .sftp
            .read_dir("/proc".to_string())
            .await
            .map_err(|_| "Сервер не отдаёт /proc (нужен Linux).".to_string())?
            .map(|e| e.file_name())
            .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .collect();

        // pass 1: `stat` of every process (tiny files, 48 at a time)
        let mut stats: HashMap<i64, Stat> = HashMap::new();
        for chunk in names.chunks(48) {
            let mut set = JoinSet::new();
            for n in chunk {
                let (c, n) = (self.clone(), n.clone());
                set.spawn(async move {
                    let st = c
                        .read_small(&format!("/proc/{n}/stat"), 4096)
                        .await
                        .as_deref()
                        .and_then(parse_stat)?;
                    Some((n.parse::<i64>().ok()?, st))
                });
            }
            while let Some(r) = set.join_next().await {
                if let Ok(Some((pid, st))) = r {
                    stats.insert(pid, st);
                }
            }
        }

        // pass 2: only session leaders with a terminal can be a login shell: read their environment
        let leaders: Vec<i64> = stats
            .iter()
            .filter(|(p, s)| s.session == **p && s.tty_nr != 0)
            .map(|(p, _)| *p)
            .collect();
        let mut shell: Option<i64> = None;
        for chunk in leaders.chunks(32) {
            let mut set = JoinSet::new();
            for pid in chunk {
                let (c, pid) = (self.clone(), *pid);
                set.spawn(async move {
                    let env = c
                        .read_small(&format!("/proc/{pid}/environ"), 256 * 1024)
                        .await?;
                    (ssh_client_port(&env) == Some(local_port)).then_some(pid)
                });
            }
            while let Some(r) = set.join_next().await {
                if let Ok(Some(pid)) = r {
                    shell = Some(shell.map_or(pid, |s: i64| s.min(pid)));
                }
            }
        }
        let Some(shell_pid) = shell else {
            return Ok(
                json!({ "found": false, "reason": "Оболочка этого терминала не найдена на сервере (нет SSH_CONNECTION/SSH_CLIENT в окружении процессов)." }),
            );
        };
        let st = stats[&shell_pid].clone();

        // the foreground process group of the terminal (the shell's own group at an idle prompt)
        let leader = if st.tpgid > 0 { st.tpgid } else { shell_pid };
        let chain = analyze(&stats, shell_pid, leader);

        let argv = |pid: i64| {
            let c = self.clone();
            async move {
                c.read_small(&format!("/proc/{pid}/cmdline"), 64 * 1024)
                    .await
                    .map(|t| split_nul(&t))
                    .filter(|a| !a.is_empty())
            }
        };
        let cwd = self
            .sftp
            .read_link(format!("/proc/{}/cwd", chain.inner))
            .await
            .ok()
            .map(|c| c.trim_end_matches(" (deleted)").to_string())
            .filter(|c| c.starts_with('/'));

        let mut fg: Option<Vec<String>> = None;
        let mut env = BTreeMap::new();
        if let Some(f) = chain.fg {
            fg = argv(f).await;
            if let Some(e) = self
                .read_small(&format!("/proc/{f}/environ"), 256 * 1024)
                .await
            {
                env = env_subset(&e);
            }
        }
        if env.is_empty() {
            if let Some(e) = self
                .read_small(&format!("/proc/{}/environ", chain.inner), 256 * 1024)
                .await
            {
                env = env_subset(&e);
            }
        }
        let root = self
            .read_small(&format!("/proc/{shell_pid}/status"), 8192)
            .await
            .as_deref()
            .and_then(parse_uid)
            == Some(0);
        let nested = match chain.launcher {
            Some(l) => argv(l)
                .await
                .map(|launcher| json!({ "launcher": launcher, "shell": stats[&chain.inner].comm })),
            None => None,
        };
        let fg_comm = chain.fg.and_then(|f| stats.get(&f).map(|s| s.comm.clone()));
        Ok(json!({
            "found": true,
            "shell": st.comm,
            "root": root,
            "cwd": cwd,
            "fg": fg,
            "fg_comm": fg_comm,
            "env": env,
            "nested": nested,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(comm: &str, ppid: i64, pgrp: i64) -> Stat {
        Stat {
            comm: comm.into(),
            ppid,
            pgrp,
            session: 100,
            tty_nr: 1,
            tpgid: 0,
        }
    }

    #[test]
    fn client_port_from_ssh_variables() {
        assert_eq!(
            ssh_client_port("A=1\0SSH_CONNECTION=10.0.0.5 51234 10.0.0.1 22\0B=2\0"),
            Some(51234)
        );
        assert_eq!(ssh_client_port("SSH_CLIENT=10.0.0.5 4444 22\0"), Some(4444));
        assert_eq!(ssh_client_port("HOME=/root\0"), None);
        assert_eq!(ssh_client_port("SSH_CONNECTION=bad\0"), None);
    }

    #[test]
    fn stat_with_spaces_and_parentheses_in_the_name() {
        let s = parse_stat("1234 (my (weird) cmd) S 1 1234 1234 34816 1234 4194560 100 0").unwrap();
        assert_eq!(
            s,
            Stat {
                comm: "my (weird) cmd".into(),
                ppid: 1,
                pgrp: 1234,
                session: 1234,
                tty_nr: 34816,
                tpgid: 1234
            }
        );
        assert!(parse_stat("garbage").is_none());
        assert!(parse_stat("1 (x) S").is_none());
    }

    #[test]
    fn cmdline_environment_and_uid_helpers() {
        assert_eq!(split_nul("btop\0-p\0\0"), vec!["btop", "-p"]);
        assert!(split_nul("").is_empty());
        let e = env_subset("PATH=/bin\0VIRTUAL_ENV=/opt/venv\0CONDA_DEFAULT_ENV=\0HOME=/root\0");
        assert_eq!(e.len(), 1);
        assert_eq!(e["VIRTUAL_ENV"], "/opt/venv");
        assert_eq!(
            parse_uid("Name:\tbash\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\n"),
            Some(0)
        );
        assert_eq!(parse_uid("Uid:\t1000\t1000\t1000\t1000"), Some(1000));
        assert_eq!(parse_uid("nothing"), None);
        assert!(is_shell("bash") && is_shell("-bash") && !is_shell("su") && !is_shell("python3"));
    }

    #[test]
    fn idle_prompt_and_plain_foreground_program() {
        let m: HashMap<i64, Stat> = [(100, st("bash", 1, 100)), (200, st("btop", 100, 200))].into();
        assert_eq!(
            analyze(&m, 100, 100),
            Chain {
                inner: 100,
                launcher: None,
                fg: None
            }
        );
        assert_eq!(
            analyze(&m, 100, 200),
            Chain {
                inner: 100,
                launcher: None,
                fg: Some(200)
            }
        );
    }

    #[test]
    fn su_to_another_user_is_a_nested_shell_with_its_launcher() {
        // root bash (100) -> su (200) -> comfyui bash (300) [foreground at its prompt]
        let m: HashMap<i64, Stat> = [
            (100, st("bash", 1, 100)),
            (200, st("su", 100, 200)),
            (300, st("bash", 200, 300)),
        ]
        .into();
        assert_eq!(
            analyze(&m, 100, 300),
            Chain {
                inner: 300,
                launcher: Some(200),
                fg: None
            }
        );
        // ... and a program started inside that shell
        let mut m2 = m.clone();
        m2.insert(400, st("python3", 300, 400));
        assert_eq!(
            analyze(&m2, 100, 400),
            Chain {
                inner: 300,
                launcher: Some(200),
                fg: Some(400)
            }
        );
        // sudo -i: sudo (200) -> sudo child (250)? -> bash (300)
        let mut m3 = m.clone();
        m3.insert(200, st("sudo", 100, 200));
        m3.insert(300, st("bash", 200, 300));
        assert_eq!(analyze(&m3, 100, 300).launcher, Some(200));
    }

    #[test]
    fn a_program_outside_the_shell_tree_is_just_a_foreground_program() {
        let m: HashMap<i64, Stat> = [(100, st("bash", 1, 100)), (900, st("weird", 1, 900))].into();
        assert_eq!(
            analyze(&m, 100, 900),
            Chain {
                inner: 100,
                launcher: None,
                fg: Some(900)
            }
        );
    }
}
