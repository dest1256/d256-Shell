//! Shared helpers for integration tests. They only ever talk to the docker test servers
//! (`sshd-password`, `sshd-kbdint`, `sshd-2fa`, `sshd-otherkey`, user `tester` / `testpass`).
#![allow(dead_code)]
use d256shell_core::sftp::conn::BoxFut;
use d256shell_core::sftp::{AuthPrompt, ConnectParams, HostKeyInfo, Interaction};
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const USER: &str = "tester";
pub const PASS: &str = "testpass";
pub const TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";

/// Returns false (and tells why) when the docker test servers are not configured, so plain
/// `cargo test` outside the dev container does not fail. `scripts/check-all.sh` requires them.
pub fn enabled() -> bool {
    let on = std::env::var("D256SHELL_TEST_SSH").is_ok();
    if !on {
        eprintln!("SKIPPED: D256SHELL_TEST_SSH is not set (run inside the dev container)");
    }
    on
}

pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "d256shell-it-{}-{}-{}",
        name,
        std::process::id(),
        rand_suffix()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rand_suffix() -> u32 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).unwrap();
    u32::from_le_bytes(b)
}

pub fn params(host: &str, password: Option<&str>, known_hosts: &Path) -> ConnectParams {
    ConnectParams {
        host: host.to_string(),
        port: 22,
        username: USER.to_string(),
        password: password.map(str::to_string),
        known_hosts: known_hosts.to_path_buf(),
        connect_timeout: Duration::from_secs(10),
        key: None,
    }
}

/// Records what the user interface would have been asked.
#[derive(Default)]
pub struct Calls {
    pub host_keys: Mutex<Vec<HostKeyInfo>>,
    pub prompts: Mutex<Vec<AuthPrompt>>,
}

pub fn interaction(
    trust: bool,
    answer: impl Fn(&AuthPrompt) -> Option<String> + Send + Sync + 'static,
) -> (Arc<Interaction>, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let (c1, c2) = (calls.clone(), calls.clone());
    let i = Interaction {
        confirm_host_key: Box::new(move |info| -> BoxFut<bool> {
            c1.host_keys.lock().unwrap().push(info);
            Box::pin(async move { trust })
        }),
        prompt: Box::new(move |p| -> BoxFut<Option<String>> {
            let a = answer(&p);
            c2.prompts.lock().unwrap().push(p);
            Box::pin(async move { a })
        }),
    };
    (Arc::new(i), calls)
}

/// Number of lines in the sshd log of `service` that contain any of `patterns`.
pub fn log_count(service: &str, patterns: &[&str]) -> usize {
    std::thread::sleep(Duration::from_millis(400)); // sshd flushes asynchronously
    let dir = std::env::var("D256SHELL_TEST_LOGS").unwrap_or_else(|_| "/sshlogs".into());
    let text = std::fs::read_to_string(format!("{dir}/{service}.log")).unwrap_or_default();
    text.lines()
        .filter(|l| patterns.iter().any(|p| l.contains(p)))
        .count()
}

fn base32_decode(s: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.bytes().filter(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("bad base32"),
        } as u32;
        bits = (bits << 5) | v;
        n += 5;
        if n >= 8 {
            out.push((bits >> (n - 8)) as u8);
            n -= 8;
        }
    }
    out
}

/// RFC 6238 TOTP (SHA-1, 6 digits, 30 s) for the test server's secret.
pub fn totp_now() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 30;
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(&base32_decode(TOTP_SECRET)).unwrap();
    mac.update(&t.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let o = (h[19] & 0xf) as usize;
    let code = ((h[o] as u32 & 0x7f) << 24)
        | ((h[o + 1] as u32) << 16)
        | ((h[o + 2] as u32) << 8)
        | h[o + 3] as u32;
    format!("{:06}", code % 1_000_000)
}
