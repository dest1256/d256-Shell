mod common;
use common::*;
use d256shell_core::known_hosts;
use d256shell_core::prompt::PromptKind;
use d256shell_core::sftp::{ConnectError, SftpConnection};
use std::time::{Duration, Instant};

/// `ssh-keyscan -t` names for the key types.
fn scan_type(key_type: &str) -> &'static str {
    match key_type {
        "ssh-ed25519" => "ed25519",
        "ecdsa-sha2-nistp256" => "ecdsa",
        "ssh-rsa" => "rsa",
        other => panic!("unknown key type {other}"),
    }
}

macro_rules! need_servers {
    () => {
        if !enabled() {
            return;
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn password_login_trusts_host_once_and_lists_home() {
    need_servers!();
    let d = tmp("login");
    let kh = d.join("known_hosts");
    let (i, calls) = interaction(true, |_| None);
    let c = SftpConnection::connect(params("sshd-password", Some(PASS), &kh), i)
        .await
        .unwrap();
    assert_eq!(
        calls.host_keys.lock().unwrap().len(),
        1,
        "unknown host asks exactly once"
    );
    assert!(
        calls.prompts.lock().unwrap().is_empty(),
        "saved password must be used silently"
    );
    assert_eq!(c.home, "/home/tester");
    let names: Vec<_> = c
        .list_dir(&c.home)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.contains(&"zone".to_string()), "{names:?}");
    c.close().await;

    // Remembered: the second connection does not ask again.
    let (i, calls) = interaction(false, |_| None);
    let c = SftpConnection::connect(params("sshd-password", Some(PASS), &kh), i)
        .await
        .unwrap();
    assert!(calls.host_keys.lock().unwrap().is_empty());
    c.close().await;
    let text = std::fs::read_to_string(&kh).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.starts_with("sshd-password ssh-"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_makes_exactly_one_attempt() {
    need_servers!();
    let d = tmp("wrongpw");
    let kh = d.join("known_hosts");
    let before = log_count("sshd-password", &["Failed password"]);
    let (i, calls) = interaction(true, |_| panic!("no prompt expected"));
    let err = SftpConnection::connect(params("sshd-password", Some("definitely-wrong"), &kh), i)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            err,
            ConnectError::AuthFailed {
                saved_password_used: true
            }
        ),
        "{err:?}"
    );
    assert!(calls.prompts.lock().unwrap().is_empty());
    let after = log_count("sshd-password", &["Failed password"]);
    assert_eq!(
        after - before,
        1,
        "sshd saw {} failed password attempts",
        after - before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn keyboard_interactive_login_and_single_wrong_attempt() {
    need_servers!();
    let d = tmp("kbd");
    let kh = d.join("known_hosts");
    let (i, calls) = interaction(true, |_| None);
    let c = SftpConnection::connect(params("sshd-kbdint", Some(PASS), &kh), i)
        .await
        .unwrap();
    assert!(
        calls.prompts.lock().unwrap().is_empty(),
        "password prompt answered by the saved password"
    );
    c.close().await;

    let pats = ["Failed keyboard-interactive", "Failed password"];
    let before = log_count("sshd-kbdint", &pats);
    let (i, calls) = interaction(true, |_| {
        panic!("must not prompt after a wrong saved password")
    });
    let err = SftpConnection::connect(params("sshd-kbdint", Some("wrong"), &kh), i)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            err,
            ConnectError::AuthFailed {
                saved_password_used: true
            }
        ),
        "{err:?}"
    );
    assert!(calls.prompts.lock().unwrap().is_empty());
    assert_eq!(log_count("sshd-kbdint", &pats) - before, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_saved_password_asks_the_user() {
    need_servers!();
    let d = tmp("nopw");
    let (i, calls) = interaction(true, |p| {
        assert_eq!(p.kind, PromptKind::Password);
        Some(PASS.to_string())
    });
    let c = SftpConnection::connect(params("sshd-password", None, &d.join("kh")), i)
        .await
        .unwrap();
    assert_eq!(calls.prompts.lock().unwrap().len(), 1);
    c.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_factor_prompt_goes_to_the_user_not_the_saved_password() {
    need_servers!();
    let d = tmp("2fa");
    let (i, calls) = interaction(true, |p| {
        assert!(
            p.text.to_lowercase().contains("verification code"),
            "unexpected prompt {:?}",
            p.text
        );
        assert_eq!(p.kind, PromptKind::Other);
        Some(totp_now())
    });
    let c = SftpConnection::connect(params("sshd-2fa", Some(PASS), &d.join("kh")), i)
        .await
        .unwrap();
    {
        let prompts = calls.prompts.lock().unwrap();
        assert_eq!(
            prompts.len(),
            1,
            "only the verification code is asked: {prompts:?}"
        );
    }
    c.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_factor_cancel_by_user() {
    need_servers!();
    let d = tmp("2facancel");
    let (i, _) = interaction(true, |_| None);
    let err = SftpConnection::connect(params("sshd-2fa", Some(PASS), &d.join("kh")), i)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, ConnectError::AuthCancelled), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_unknown_host_key_is_not_remembered_and_password_not_sent() {
    need_servers!();
    let d = tmp("reject");
    let kh = d.join("known_hosts");
    let accepted_before = log_count("sshd-password", &["Accepted password", "Failed password"]);
    let (i, calls) = interaction(false, |_| None);
    let err = SftpConnection::connect(params("sshd-password", Some(PASS), &kh), i)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, ConnectError::HostKeyRejected), "{err:?}");
    assert_eq!(calls.host_keys.lock().unwrap().len(), 1);
    assert!(!kh.exists() || std::fs::read_to_string(&kh).unwrap().is_empty());
    assert_eq!(
        log_count("sshd-password", &["Accepted password", "Failed password"]),
        accepted_before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_host_key_refuses_and_known_hosts_stays_byte_identical() {
    need_servers!();
    let d = tmp("changed");
    // Learn the *other* server's key, then claim it belongs to "sshd-password".
    let other_kh = d.join("other");
    let (i, calls) = interaction(true, |_| None);
    SftpConnection::connect(params("sshd-otherkey", Some(PASS), &other_kh), i)
        .await
        .unwrap()
        .close()
        .await;
    let other = calls.host_keys.lock().unwrap()[0].clone();

    let kh = d.join("known_hosts");
    std::fs::write(
        &kh,
        format!(
            "# mine\nsshd-password {} {}\n",
            other.key_type, other.key_b64
        ),
    )
    .unwrap();
    let before = std::fs::read(&kh).unwrap();
    let logs_before = log_count("sshd-password", &["Accepted password", "Failed password"]);

    let (i, calls) = interaction(true, |_| panic!("must not ask anything"));
    let err = SftpConnection::connect(params("sshd-password", Some(PASS), &kh), i)
        .await
        .err()
        .unwrap();
    match err {
        ConnectError::HostKeyChanged(info) => {
            assert_ne!(info.key_b64, other.key_b64);
            assert!(info.fingerprint.starts_with("SHA256:"));
        }
        e => panic!("expected HostKeyChanged, got {e:?}"),
    }
    assert!(
        calls.host_keys.lock().unwrap().is_empty(),
        "a changed key must not offer 'trust'"
    );
    assert_eq!(
        std::fs::read(&kh).unwrap(),
        before,
        "known_hosts must not be modified"
    );
    assert_eq!(
        log_count("sshd-password", &["Accepted password", "Failed password"]),
        logs_before,
        "no password may be sent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revoked_key_is_refused() {
    need_servers!();
    let d = tmp("revoked");
    let (i, calls) = interaction(true, |_| None);
    SftpConnection::connect(params("sshd-password", Some(PASS), &d.join("first")), i)
        .await
        .unwrap()
        .close()
        .await;
    let k = calls.host_keys.lock().unwrap()[0].clone();
    let kh = d.join("known_hosts");
    std::fs::write(
        &kh,
        format!("@revoked sshd-password {} {}\n", k.key_type, k.key_b64),
    )
    .unwrap();
    let (i, _) = interaction(true, |_| None);
    let err = SftpConnection::connect(params("sshd-password", Some(PASS), &kh), i)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, ConnectError::HostKeyRevoked(_)), "{err:?}");
}

/// russh must negotiate the key type that is on file (as OpenSSH does), otherwise a valid host
/// would be reported as "changed" or ask to trust a second key.
#[tokio::test(flavor = "multi_thread")]
async fn negotiates_the_host_key_type_found_in_known_hosts() {
    need_servers!();
    for kt in ["ssh-ed25519", "ecdsa-sha2-nistp256", "ssh-rsa"] {
        let d = tmp("keytype");
        let kh = d.join("known_hosts");
        let out = std::process::Command::new("ssh-keyscan")
            .args(["-t", scan_type(kt), "sshd-password"])
            .output()
            .expect("ssh-keyscan");
        let line = String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|l| !l.starts_with('#'))
            .unwrap_or("")
            .to_string();
        assert!(line.contains(kt), "keyscan gave {line:?} for {kt}");
        std::fs::write(&kh, format!("{line}\n")).unwrap();
        let (i, calls) = interaction(false, |_| None);
        let c = SftpConnection::connect(params("sshd-password", Some(PASS), &kh), i)
            .await
            .unwrap_or_else(|e| panic!("{kt}: {e:?}"));
        assert!(
            calls.host_keys.lock().unwrap().is_empty(),
            "{kt}: asked to trust although key is on file"
        );
        c.close().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_host_times_out_without_hanging() {
    need_servers!();
    let d = tmp("timeout");
    let mut p = params("10.255.255.1", Some(PASS), &d.join("kh"));
    p.connect_timeout = Duration::from_secs(2);
    let (i, _) = interaction(true, |_| None);
    let t = Instant::now();
    let err = SftpConnection::connect(p, i).await.err().unwrap();
    assert!(
        matches!(err, ConnectError::Timeout | ConnectError::Network(_)),
        "{err:?}"
    );
    assert!(
        t.elapsed() < Duration::from_secs(6),
        "took {:?}",
        t.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn known_hosts_hashed_entries_from_real_ssh_keygen() {
    need_servers!();
    let d = tmp("hashed");
    let kh = d.join("known_hosts");
    let (i, calls) = interaction(true, |_| None);
    SftpConnection::connect(params("sshd-password", Some(PASS), &kh), i)
        .await
        .unwrap()
        .close()
        .await;
    let k = calls.host_keys.lock().unwrap()[0].clone();
    // Let OpenSSH hash the entry, then check that our matcher understands its output.
    let st = std::process::Command::new("ssh-keygen")
        .args(["-H", "-f"])
        .arg(&kh)
        .output()
        .expect("ssh-keygen");
    assert!(
        st.status.success(),
        "{}",
        String::from_utf8_lossy(&st.stderr)
    );
    let text = std::fs::read_to_string(&kh).unwrap();
    assert!(text.starts_with("|1|"), "entry was not hashed: {text}");
    assert_eq!(
        known_hosts::check_file(&kh, "sshd-password", 22, &k.key_type, &k.key_b64).unwrap(),
        known_hosts::Verdict::Match
    );
    assert_eq!(
        known_hosts::check_file(&kh, "other-host", 22, &k.key_type, &k.key_b64).unwrap(),
        known_hosts::Verdict::Unknown
    );
    // ...and the connection still works against the hashed file without asking again.
    let (i, calls) = interaction(false, |_| None);
    SftpConnection::connect(params("sshd-password", Some(PASS), &kh), i)
        .await
        .unwrap()
        .close()
        .await;
    assert!(calls.host_keys.lock().unwrap().is_empty());
}
