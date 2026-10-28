//! Terminal integration tests: the real system `ssh` in a PTY against the docker test servers, with this
//! package's binary acting as the SSH_ASKPASS helper (exactly like the shipped exe).
#![cfg(unix)]
#[path = "../../core/tests/common/mod.rs"]
mod common;

use base64ct::{Base64, Encoding};
use common::*;
use d256shell_core::paths;
use d256shell_core::service::Service;
use d256shell_core::terminal::args::{build_args, SshTarget};
use d256shell_core::terminal::askpass::{self, AskInfo, AskpassServer, SavedPasswordPolicy};
use d256shell_core::terminal::session::{ReadResult, TermOptions, TermSession};
use d256shell_core::vault::KdfParams;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const ASKPASS_EXE: &str = env!("CARGO_BIN_EXE_d256shell-devbridge");

macro_rules! need_servers {
    () => {
        if !enabled() {
            return;
        }
    };
}

fn service(name: &str, exe: &str) -> Arc<Service> {
    let dirs = paths::prepare(tmp(name)).unwrap();
    let svc = Service::with_kdf(
        dirs,
        KdfParams {
            m_cost_kib: 64,
            t_cost: 1,
            p_cost: 1,
        },
    )
    .with_askpass_exe(PathBuf::from(exe));
    svc.call("create_vault", json!({"master": "long-enough-pw"}))
        .unwrap();
    Arc::new(svc)
}

fn add_server(svc: &Service, host: &str, password: &str) -> String {
    svc.call(
        "save_server",
        json!({"name": host, "host": host, "port": 22, "username": USER, "password": password}),
    )
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Connect like the UI does: trust the host key, answer further prompts with `answer`.
fn connect(
    svc: &Arc<Service>,
    server_id: &str,
    answer: impl Fn(&Value) -> Option<String>,
) -> String {
    let attempt = format!("att-{}", server_id);
    let s2 = svc.clone();
    let (id, at) = (server_id.to_string(), attempt.clone());
    let h = thread::spawn(move || s2.call("connect", json!({"server_id": id, "attempt": at})));
    let t = Instant::now();
    while !h.is_finished() {
        assert!(t.elapsed() < Duration::from_secs(30), "connect hangs");
        for p in svc
            .call("poll_prompts", json!({"attempt": attempt}))
            .unwrap()
            .as_array()
            .unwrap()
        {
            let id = p["id"].clone();
            if p["kind"] == "host_key" {
                svc.call("answer_prompt", json!({"id": id, "accept": true}))
                    .unwrap();
            } else {
                match answer(p) {
                    Some(v) => svc
                        .call("answer_prompt", json!({"id": id, "value": v}))
                        .unwrap(),
                    None => svc.call("answer_prompt", json!({"id": id})).unwrap(),
                };
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    let r = h
        .join()
        .unwrap()
        .unwrap_or_else(|e| panic!("connect failed: {e:?}"));
    r["session_id"].as_str().unwrap().to_string()
}

struct Term {
    svc: Arc<Service>,
    id: String,
    out: Vec<u8>,
    exit: Option<i32>,
}

impl Term {
    fn open(svc: &Arc<Service>, session: &str) -> (Term, Value) {
        let r = svc
            .call(
                "term_open",
                json!({"session_id": session, "cols": 100, "rows": 30}),
            )
            .unwrap();
        let id = r["term_id"].as_str().unwrap().to_string();
        (
            Term {
                svc: svc.clone(),
                id,
                out: Vec::new(),
                exit: None,
            },
            r,
        )
    }

    fn pump(&mut self, wait: Duration) {
        match self.svc.term_read_raw(&self.id, wait) {
            Ok(ReadResult::Data(d)) => self.out.extend(d),
            Ok(ReadResult::Exited(c)) => self.exit = Some(c),
            _ => {}
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }

    /// Wait until the output contains `needle` (searching only output received after `from`).
    fn expect(&mut self, needle: &str, from: usize, timeout: Duration) -> usize {
        let t = Instant::now();
        loop {
            let s = String::from_utf8_lossy(&self.out[from.min(self.out.len())..]).into_owned();
            if let Some(i) = s.find(needle) {
                return from + i + needle.len();
            }
            assert!(
                self.exit.is_none(),
                "terminal exited ({:?}) while waiting for {needle:?}. Output:\n{}",
                self.exit,
                self.text()
            );
            assert!(
                t.elapsed() < timeout,
                "timeout waiting for {needle:?}. Output:\n{}",
                self.text()
            );
            self.pump(Duration::from_millis(100));
        }
    }

    fn send(&self, s: &str) {
        self.svc
            .call(
                "term_write",
                json!({"term_id": self.id, "data_b64": Base64::encode_string(s.as_bytes())}),
            )
            .unwrap();
    }

    fn wait_exit(&mut self, timeout: Duration) -> i32 {
        let t = Instant::now();
        while self.exit.is_none() {
            assert!(t.elapsed() < timeout, "no exit. Output:\n{}", self.text());
            self.pump(Duration::from_millis(100));
        }
        self.exit.unwrap()
    }
}

const READY: &str = "$ ";
const T: Duration = Duration::from_secs(30);

fn login(svc: &Arc<Service>, host: &str, pw: &str) -> Term {
    let sid = add_server(svc, host, pw);
    let session = connect(svc, &sid, |_| None);
    let (mut t, _) = Term::open(svc, &session);
    t.expect(READY, 0, T);
    t
}

#[test]
fn askpass_login_runs_a_command_without_typing_a_password() {
    need_servers!();
    let svc = service("tl1", ASKPASS_EXE);
    let sid = add_server(&svc, "sshd-password", PASS);
    let session = connect(&svc, &sid, |_| None);
    let (mut t, info) = Term::open(&svc, &session);
    assert_eq!(info["mode"], "askpass", "{info}");
    let at = t.expect(READY, 0, T);
    t.send("echo marker-$((1000+234))\r");
    t.expect("marker-1234", at, T);
    let text = t.text();
    assert!(
        !text.to_lowercase().contains("password:"),
        "askpass must be invisible in the terminal:\n{text}"
    );
    // no question was shown to the user for a plain saved password
    assert_eq!(
        svc.call("poll_prompts", json!({"attempt": t.id})).unwrap(),
        json!([])
    );
    t.send("exit\r");
    assert_eq!(t.wait_exit(T), 0);
}

#[test]
fn askpass_helper_path_with_spaces_works() {
    need_servers!();
    let dir = tmp("spaces").join("Test Folder").join("Shell port");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("d256 Shell askpass");
    std::fs::copy(ASKPASS_EXE, &exe).unwrap();
    let svc = service("tl2", exe.to_str().unwrap());
    let mut t = login(&svc, "sshd-password", PASS);
    t.send("echo spaces-ok\r");
    t.expect("spaces-ok", 0, T);
}

#[test]
fn two_factor_code_in_the_terminal_goes_to_the_user() {
    need_servers!();
    let svc = service("tl3", ASKPASS_EXE);
    let sid = add_server(&svc, "sshd-2fa", PASS);
    // SFTP login needs the code as well
    let session = connect(&svc, &sid, |_| Some(totp_now()));
    let (mut t, _) = Term::open(&svc, &session);
    // the terminal's ssh asks again: password (saved, automatic) then the verification code (user)
    let t0 = Instant::now();
    let prompt = loop {
        let ps = svc.call("poll_prompts", json!({"attempt": t.id})).unwrap();
        if let Some(p) = ps.as_array().and_then(|a| a.first()) {
            break p.clone();
        }
        t.pump(Duration::from_millis(50));
        assert!(t0.elapsed() < T, "no 2FA prompt. Output:\n{}", t.text());
    };
    assert!(
        prompt["text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("verification code"),
        "{prompt}"
    );
    assert_eq!(prompt["is_password"], false);
    svc.call(
        "answer_prompt",
        json!({"id": prompt["id"], "value": totp_now()}),
    )
    .unwrap();
    t.expect(READY, 0, T);
}

/// Wrong saved password: ssh asks askpass again after the failure; the second request must go to the
/// user (here: cancelled) and the server must see exactly one failed password.
#[test]
fn wrong_saved_password_means_one_failed_attempt_and_a_question_for_the_user() {
    need_servers!();
    let d = tmp("tl4");
    let kh = d.join("known_hosts");
    let scan = std::process::Command::new("ssh-keyscan")
        .args(["-t", "ed25519", "sshd-password"])
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&scan.stdout)
        .lines()
        .find(|l| !l.starts_with('#'))
        .unwrap()
        .to_string();
    std::fs::write(&kh, format!("{line}\n")).unwrap();

    let asked: Arc<Mutex<Vec<(String, bool, bool)>>> = Arc::default();
    let (a2, policy) = (
        asked.clone(),
        SavedPasswordPolicy::new(Some("definitely-wrong".into())),
    );
    // like Hub: when the user refuses to answer, ssh is killed BEFORE the helper is told, otherwise
    // OpenSSH sends an empty password for its next attempt
    let victim: Arc<Mutex<Option<std::sync::Weak<TermSession>>>> = Arc::default();
    let v2 = victim.clone();
    let server = AskpassServer::start(
        Arc::new(move |prompt: &str| {
            policy.decide(prompt, &|i: AskInfo<'_>| {
                a2.lock().unwrap().push((
                    i.prompt.to_string(),
                    i.is_password,
                    i.saved_password_rejected,
                ));
                None // the user cancels the dialog
            })
        }),
        Some(Arc::new(move || {
            if let Some(s) = v2
                .lock()
                .unwrap()
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
            {
                s.close();
            }
        })),
    )
    .unwrap();

    let args = build_args(
        &SshTarget {
            host: "sshd-password",
            port: 22,
            username: USER,
            extra: "",
            identity: None,
        },
        &kh,
    )
    .unwrap();
    let env = vec![
        (
            "SSH_ASKPASS".to_string(),
            askpass::askpass_program_path(std::path::Path::new(ASKPASS_EXE)),
        ),
        ("SSH_ASKPASS_REQUIRE".to_string(), "force".to_string()),
        (askpass::ENV_FLAG.to_string(), "1".to_string()),
        (askpass::ENV_ADDR.to_string(), server.addr.to_string()),
        (askpass::ENV_TOKEN.to_string(), server.token.clone()),
    ];
    let before = log_count("sshd-password", &["Failed password"]);
    let s = TermSession::spawn(TermOptions {
        program: "ssh".into(),
        args,
        env,
        cols: 80,
        rows: 24,
        expect_password: None,
    })
    .unwrap();
    *victim.lock().unwrap() = Some(Arc::downgrade(&s));
    let t0 = Instant::now();
    let mut out = Vec::new();
    let code = loop {
        match s.read(Duration::from_millis(200)) {
            ReadResult::Data(d) => out.extend(d),
            ReadResult::Exited(c) => break c,
            ReadResult::Timeout => {}
        }
        assert!(t0.elapsed() < T, "ssh did not end");
    };
    assert_ne!(
        code,
        0,
        "login must fail, output: {}",
        String::from_utf8_lossy(&out)
    );
    let asked = asked.lock().unwrap().clone();
    let failed = log_count("sshd-password", &["Failed password"]) - before;
    eprintln!(
        "questions for the user: {}, failed attempts seen by sshd: {failed}, ssh output: {:?}",
        asked.len(),
        String::from_utf8_lossy(&out)
    );
    assert_eq!(
        failed, 1,
        "server must see exactly one failed attempt (a cancelled dialog must not send anything)"
    );
    assert_eq!(
        asked.len(),
        1,
        "exactly one question reached the user: {asked:?}"
    );
    assert!(
        asked[0].1 && asked[0].2,
        "it is a password request flagged as 'saved password rejected': {asked:?}"
    );
}

#[test]
fn expect_fallback_logs_in_with_the_saved_password() {
    need_servers!();
    let svc = service("tl5", ASKPASS_EXE);
    svc.call("set_settings", json!({"auto_password": "expect"}))
        .unwrap();
    let sid = add_server(&svc, "sshd-password", PASS);
    let session = connect(&svc, &sid, |_| None);
    let (mut t, info) = Term::open(&svc, &session);
    assert_eq!(info["mode"], "expect");
    let at = t.expect(READY, 0, T);
    // the fallback typed the password (visible only as a successful login, ssh does not echo it)
    assert!(!t.text().contains(PASS));
    // after login the user prints a password-looking prompt: nothing may be typed for it
    t.send("printf 'password: '\r");
    let at2 = t.expect("password: ", at, T);
    thread::sleep(Duration::from_millis(1200));
    for _ in 0..10 {
        t.pump(Duration::from_millis(50));
    }
    let tail = String::from_utf8_lossy(&t.out[at2..]).into_owned();
    assert!(
        !tail.contains(PASS),
        "the saved password was typed into the shell: {tail:?}"
    );
}

#[test]
fn resize_is_visible_to_the_remote_side() {
    need_servers!();
    let svc = service("tl6", ASKPASS_EXE);
    let mut t = login(&svc, "sshd-password", PASS);
    svc.call(
        "term_resize",
        json!({"term_id": t.id, "cols": 132, "rows": 43}),
    )
    .unwrap();
    thread::sleep(Duration::from_millis(300));
    let at = t.out.len();
    t.send("stty size\r");
    t.expect("43 132", at, T);
}

#[test]
fn remote_exit_code_is_reported() {
    need_servers!();
    let svc = service("tl7", ASKPASS_EXE);
    let mut t = login(&svc, "sshd-password", PASS);
    t.send("exit 3\r");
    assert_eq!(t.wait_exit(T), 3);
}

#[test]
fn closing_the_terminal_kills_ssh() {
    need_servers!();
    let svc = service("tl8", ASKPASS_EXE);
    let sid = add_server(&svc, "sshd-password", PASS);
    let session = connect(&svc, &sid, |_| None);
    let (mut t, _) = Term::open(&svc, &session);
    t.expect(READY, 0, T);
    let pid = svc_pid(&svc, &t.id);
    assert!(PathBuf::from(format!("/proc/{pid}")).exists());
    svc.call("term_close", json!({"term_id": t.id})).unwrap();
    for _ in 0..50 {
        if !PathBuf::from(format!("/proc/{pid}")).exists() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("ssh process {pid} is still running after term_close");
}

fn svc_pid(svc: &Service, term_id: &str) -> u32 {
    svc.term_pid(term_id).expect("pid")
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[test]
fn fifty_megabytes_of_output_neither_hang_nor_balloon_memory() {
    need_servers!();
    let svc = service("tl9", ASKPASS_EXE);
    let mut t = login(&svc, "sshd-password", PASS);
    let rss0 = rss_kib();
    let at = t.out.len();
    t.send("head -c 50M /dev/urandom | base64; echo DONE-$((7*6))\r");
    // consume without storing everything: only a sliding tail is kept
    let t0 = Instant::now();
    let mut total = 0usize;
    let mut tail: Vec<u8> = Vec::new();
    let mut peak = rss0;
    t.out.drain(..at);
    loop {
        match svc
            .term_read_raw(&t.id, Duration::from_millis(200))
            .unwrap()
        {
            ReadResult::Data(d) => {
                total += d.len();
                tail.extend_from_slice(&d);
                if tail.len() > 4096 {
                    tail.drain(..tail.len() - 4096);
                }
            }
            ReadResult::Exited(c) => panic!("terminal exited ({c})"),
            ReadResult::Timeout => {}
        }
        peak = peak.max(rss_kib());
        if String::from_utf8_lossy(&tail).contains("DONE-42") {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(180),
            "stalled after {total} bytes"
        );
    }
    assert!(total > 66_000_000, "got only {total} bytes");
    let growth_mib = (peak.saturating_sub(rss0)) / 1024;
    assert!(growth_mib < 150, "memory grew by {growth_mib} MiB");
    eprintln!(
        "50 MB output: {total} bytes in {:?}, RSS growth {growth_mib} MiB",
        t0.elapsed()
    );
}

#[test]
fn terminal_works_on_a_server_without_sftp() {
    need_servers!();
    let svc = service("tl11", ASKPASS_EXE);
    let mut t = login(&svc, "sshd-nosftp", PASS);
    let at = t.out.len();
    t.send("echo no-sftp-shell-$((20+22))\r");
    t.expect("no-sftp-shell-42", at, T);
}

#[test]
fn utf8_output_arrives_byte_exact_even_when_chunks_split_characters() {
    need_servers!();
    let svc = service("tl10", ASKPASS_EXE);
    let mut t = login(&svc, "sshd-password", PASS);
    let at = t.out.len();
    t.send("yes 'ёжик-✓-🙂' | head -n 100000; echo END-$((6*7))\r");
    t.expect("END-42", at, Duration::from_secs(60));
    let bytes = &t.out[at..];
    let text =
        String::from_utf8(bytes.to_vec()).expect("the byte stream must be valid UTF-8 as a whole");
    assert_eq!(text.matches("ёжик-✓-🙂").count(), 100_000);
    assert!(!text.contains('\u{FFFD}'));
}

/// "Duplicate terminal": the shell of a terminal is found on the server (no command is run for that) and its
/// folder, foreground program and virtualenv are reported, so that a copy can reproduce them.
#[test]
fn terminal_context_reports_folder_foreground_program_and_virtualenv() {
    need_servers!();
    let svc = service("tl-ctx", ASKPASS_EXE);
    let mut t = login(&svc, "sshd-password", PASS);
    let ctx = |t: &Term| svc.call("term_context", json!({"term_id": t.id})).unwrap();

    // idle prompt in /tmp: a folder, no foreground program
    t.send("cd /tmp\r");
    let at = t.expect("/tmp[00m$ ", 0, T);
    let c = ctx(&t);
    assert_eq!(c["found"], true, "{c}");
    assert_eq!(c["cwd"], "/tmp");
    assert!(c["fg"].is_null(), "{c}");

    // a virtualenv (exported variable) and a long running program in the foreground
    t.send("export VIRTUAL_ENV=/opt/venv-test; cd /var/log; tail -f /etc/hostname\r");
    let at = t.expect("\n", at + 40, T);
    thread::sleep(Duration::from_millis(600));
    let c = ctx(&t);
    assert_eq!(c["found"], true, "{c}");
    assert_eq!(c["cwd"], "/var/log", "{c}");
    assert_eq!(c["fg"], json!(["tail", "-f", "/etc/hostname"]), "{c}");
    assert_eq!(c["env"]["VIRTUAL_ENV"], "/opt/venv-test", "{c}");
    assert!(c["shell"].as_str().unwrap().contains("bash"), "{c}");

    // a second terminal of the same server is a different shell: contexts do not mix
    let (mut t2, _) = Term::open(&svc, &svc_session_of(&svc, &t));
    t2.expect(READY, 0, T);
    t2.send("cd /usr\r");
    t2.expect("/usr[00m$ ", 0, T);
    let c2 = ctx(&t2);
    assert_eq!(c2["cwd"], "/usr", "{c2}");
    assert!(c2["fg"].is_null());
    assert_eq!(ctx(&t)["cwd"], "/var/log");

    // Ctrl+C returns the first one to the prompt
    t.send("\x03");
    t.expect(READY, at, T);
    thread::sleep(Duration::from_millis(300));
    assert!(ctx(&t)["fg"].is_null());
}

fn svc_session_of(svc: &Service, t: &Term) -> String {
    svc.term_session_id(&t.id).expect("session of terminal")
}

/// `su - otheruser` as root (no password prompt): the copy has to know about the nested shell, the command that
/// started it, and the folder / program inside it.
#[test]
fn terminal_context_sees_a_nested_shell_started_with_su() {
    need_servers!();
    let svc = service("tl-su", ASKPASS_EXE);
    let sid = svc
        .call(
            "save_server",
            json!({"name": "root", "host": "sshd-password", "port": 22, "username": "root", "password": "rootpass"}),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let session = connect(&svc, &sid, |_| None);
    let (mut t, _) = Term::open(&svc, &session);
    t.expect("# ", 0, T);
    let ctx = |t: &Term| svc.call("term_context", json!({"term_id": t.id})).unwrap();

    // plain root shell first
    let c = ctx(&t);
    assert_eq!(c["found"], true, "{c}");
    assert_eq!(c["root"], true, "{c}");
    assert!(c["nested"].is_null(), "{c}");

    // su - otheruser, then a folder in the nested shell
    t.send("su - otheruser\r");
    let at = t.expect("otheruser@", 0, T);
    t.send("cd /tmp\r");
    t.expect("/tmp", at, T);
    thread::sleep(Duration::from_millis(500));
    let c = ctx(&t);
    assert_eq!(
        c["nested"]["launcher"],
        json!(["su", "-", "otheruser"]),
        "{c}"
    );
    // reading another user's cwd needs CAP_SYS_PTRACE, which docker drops even for root: unknown here, "/tmp" on a real host
    assert!(c["cwd"].is_null() || c["cwd"] == "/tmp", "{c}");
    assert!(c["fg"].is_null(), "{c}");
    assert_eq!(c["root"], true);

    // a program in the nested shell
    t.send("sleep 300\r");
    thread::sleep(Duration::from_millis(600));
    let c = ctx(&t);
    assert_eq!(
        c["nested"]["launcher"],
        json!(["su", "-", "otheruser"]),
        "{c}"
    );
    assert_eq!(c["fg"], json!(["sleep", "300"]), "{c}");
    t.send("\x03exit\r");
    thread::sleep(Duration::from_millis(500));
    let c = ctx(&t);
    assert!(c["nested"].is_null(), "back in the root shell: {c}");
}

/// A server saved with a private key (and optionally its passphrase), like the server form does.
fn add_key_server(svc: &Service, key: &str, passphrase: Option<&str>, password: &str) -> String {
    let mut p = json!({
        "name": "key", "host": "sshd-key", "port": 22, "username": USER,
        "password": password, "key_path": format!("/testkeys/{key}"),
    });
    if let Some(pp) = passphrase {
        p["key_passphrase"] = json!(pp);
    }
    svc.call("save_server", p).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn key_login_in_the_terminal_needs_neither_password_nor_questions() {
    need_servers!();
    let svc = service("tl-key1", ASKPASS_EXE);
    let sid = add_key_server(&svc, "id_ed25519", None, "");
    let before = log_count("sshd-key", &["Accepted publickey"]);
    let before_pw = log_count("sshd-key", &["Accepted password"]);
    let session = connect(&svc, &sid, |p| panic!("unexpected question: {p}"));
    let (mut t, info) = Term::open(&svc, &session);
    assert_eq!(info["mode"], "askpass", "{info}");
    let at = t.expect(READY, 0, T);
    t.send("echo key-login-$((20+22))\r");
    t.expect("key-login-42", at, T);
    // both the SFTP login and the terminal's own ssh used the key
    assert_eq!(log_count("sshd-key", &["Accepted publickey"]) - before, 2);
    assert_eq!(log_count("sshd-key", &["Accepted password"]), before_pw);
    // no password and no key path ever appears on the command line of ssh
    let cmdline = std::fs::read(format!("/proc/{}/cmdline", svc.term_pid(&t.id).unwrap())).unwrap();
    let cmd = String::from_utf8_lossy(&cmdline).replace('\0', " ");
    assert!(
        cmd.contains("-i /testkeys/id_ed25519") && cmd.contains("IdentitiesOnly=yes"),
        "{cmd}"
    );
    assert!(!cmd.contains(PASS), "{cmd}");
}

#[test]
fn encrypted_key_in_the_terminal_uses_the_saved_passphrase_once_otherwise_asks() {
    need_servers!();
    // saved passphrase: the user is not bothered (SFTP and terminal)
    let svc = service("tl-key2", ASKPASS_EXE);
    let sid = add_key_server(&svc, "id_enc", Some("keypass"), "");
    let session = connect(&svc, &sid, |p| panic!("unexpected question: {p}"));
    let (mut t, _) = Term::open(&svc, &session);
    t.expect(READY, 0, T);

    // not saved: the passphrase prompt of ssh goes to the user as a masked question
    let svc = service("tl-key3", ASKPASS_EXE);
    let sid = add_key_server(&svc, "id_enc", None, "");
    let session = connect(&svc, &sid, |_| Some("keypass".into())); // SFTP asks once
    let (mut t, _) = Term::open(&svc, &session);
    let t0 = Instant::now();
    let prompt = loop {
        let ps = svc.call("poll_prompts", json!({"attempt": t.id})).unwrap();
        if let Some(p) = ps.as_array().and_then(|a| a.first()) {
            break p.clone();
        }
        t.pump(Duration::from_millis(50));
        assert!(
            t0.elapsed() < T,
            "no passphrase prompt. Output:\n{}",
            t.text()
        );
    };
    assert!(
        prompt["text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("passphrase"),
        "{prompt}"
    );
    assert_eq!(prompt["is_password"], true, "masked input");
    svc.call(
        "answer_prompt",
        json!({"id": prompt["id"], "value": "keypass"}),
    )
    .unwrap();
    t.expect(READY, 0, T);
}

#[test]
fn rejected_key_in_the_terminal_falls_back_to_the_saved_password() {
    need_servers!();
    let svc = service("tl-key4", ASKPASS_EXE);
    let sid = add_key_server(&svc, "id_other", None, PASS);
    let before = log_count("sshd-key", &["Accepted password"]);
    let session = connect(&svc, &sid, |p| panic!("unexpected question: {p}"));
    let (mut t, _) = Term::open(&svc, &session);
    t.expect(READY, 0, T);
    assert_eq!(
        log_count("sshd-key", &["Accepted password"]) - before,
        2,
        "SFTP and terminal both used the password after the key was refused"
    );
}

#[test]
fn unusable_key_path_is_explained_before_ssh_is_started() {
    need_servers!();
    let svc = service("tl-key5", ASKPASS_EXE);
    // the SFTP login would already fail: open the terminal of a session whose key file vanished afterwards
    let dir = tmp("tl-key5-keys");
    std::fs::create_dir_all(&dir).unwrap();
    let k = dir.join("id");
    std::fs::copy("/testkeys/id_ed25519", &k).unwrap();
    let sid = svc
        .call(
            "save_server",
            json!({"name": "k", "host": "sshd-key", "port": 22, "username": USER, "password": "", "key_path": k.to_string_lossy()}),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let session = connect(&svc, &sid, |_| None);
    std::fs::remove_file(&k).unwrap();
    let e = svc
        .call(
            "term_open",
            json!({"session_id": session, "cols": 80, "rows": 24}),
        )
        .unwrap_err();
    assert_eq!(e.code, "key_error", "{e:?}");
    assert!(e.message.contains("не найден"), "{}", e.message);
}
