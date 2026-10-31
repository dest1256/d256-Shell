//! Private key login against the `sshd-key` test server (keys are generated into the /testkeys volume).
mod common;
use common::*;
use d256shell_core::prompt::PromptKind;
use d256shell_core::sftp::{ConnectError, KeyAuth, SftpConnection};

macro_rules! need_servers {
    () => {
        if !enabled() {
            return;
        }
    };
}

const KEYS: &str = "/testkeys";

fn key(name: &str, passphrase: Option<&str>) -> KeyAuth {
    KeyAuth {
        path: format!("{KEYS}/{name}"),
        passphrase: passphrase.map(str::to_string),
    }
}

async fn connect(
    host: &str,
    key: Option<KeyAuth>,
    password: Option<&str>,
    answer: impl Fn(&d256shell_core::sftp::AuthPrompt) -> Option<String> + Send + Sync + 'static,
) -> (Result<SftpConnection, ConnectError>, std::sync::Arc<Calls>) {
    let d = tmp(&format!("key-{host}-{}", rand_suffix()));
    let (i, calls) = interaction(true, answer);
    let mut p = params(host, password, &d.join("kh"));
    p.key = key;
    (SftpConnection::connect(p, i).await, calls)
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

#[tokio::test(flavor = "multi_thread")]
async fn every_key_type_logs_in_without_any_password_and_without_prompts() {
    need_servers!();
    let before = log_count("sshd-key", &["Accepted publickey"]);
    let before_pw = log_count("sshd-key", &["Accepted password"]);
    for name in ["id_ed25519", "id_rsa", "id_rsa_pem", "id_ecdsa"] {
        let (r, calls) = connect("sshd-key", Some(key(name, None)), None, |_| None).await;
        let c = r.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(c.home, "/home/tester", "{name}");
        assert!(
            calls.prompts.lock().unwrap().is_empty(),
            "{name}: no question expected"
        );
        c.close().await;
    }
    assert_eq!(
        log_count("sshd-key", &["Accepted publickey"]) - before,
        4,
        "the server really used the keys"
    );
    assert_eq!(
        log_count("sshd-key", &["Accepted password"]),
        before_pw,
        "no password was involved"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn encrypted_key_uses_the_saved_passphrase_or_asks_for_it() {
    need_servers!();
    // saved: no question
    let (r, calls) = connect(
        "sshd-key",
        Some(key("id_enc", Some("keypass"))),
        None,
        |_| None,
    )
    .await;
    r.expect("saved passphrase").close().await;
    assert!(calls.prompts.lock().unwrap().is_empty());

    // not saved: the user is asked (masked passphrase prompt), the answer works
    let (r, calls) = connect("sshd-key", Some(key("id_enc", None)), None, |p| {
        assert_eq!(p.kind, PromptKind::Passphrase);
        Some("keypass".into())
    })
    .await;
    r.expect("typed passphrase").close().await;
    {
        let asked = calls.prompts.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].text.contains("id_enc"), "{}", asked[0].text);
        assert!(!asked[0].echo);
    }

    // wrong saved passphrase: the user is asked, flagged as "the saved one did not work"
    let (r, calls) = connect("sshd-key", Some(key("id_enc", Some("nope"))), None, |_| {
        Some("keypass".into())
    })
    .await;
    r.expect("saved was wrong, typed is right").close().await;
    assert!(calls.prompts.lock().unwrap()[0].saved_password_rejected);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_or_cancelled_passphrase_never_reaches_the_server() {
    need_servers!();
    let attempts = || log_count("sshd-key", &["Failed", "Accepted", "Invalid user"]);
    let before = attempts();
    let (r, calls) = connect("sshd-key", Some(key("id_enc", None)), None, |_| {
        Some("wrong".into())
    })
    .await;
    match r {
        Err(ConnectError::KeyError(m)) => assert!(m.contains("passphrase"), "{m}"),
        other => panic!("{:?}", other.err()),
    }
    assert_eq!(
        calls.prompts.lock().unwrap().len(),
        3,
        "three tries, then the key is given up"
    );
    let (r, _) = connect("sshd-key", Some(key("id_enc", None)), None, |_| None).await;
    assert!(matches!(r, Err(ConnectError::AuthCancelled)));
    assert_eq!(
        attempts(),
        before,
        "decrypting a file is local: the server saw no login attempt"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_key_is_reported_or_falls_back_to_the_saved_password_exactly_once() {
    need_servers!();
    // not authorised on the server, no password: a clear "key rejected"
    let (r, _) = connect("sshd-key", Some(key("id_other", None)), None, |_| None).await;
    assert!(matches!(r, Err(ConnectError::KeyRejected)), "{:?}", r.err());
    // with a saved password the login goes on with the password (one attempt, as always)
    let before_ok = log_count("sshd-key", &["Accepted password"]);
    let (r, _) = connect("sshd-key", Some(key("id_other", None)), Some(PASS), |_| {
        None
    })
    .await;
    r.expect("password after a rejected key").close().await;
    assert_eq!(log_count("sshd-key", &["Accepted password"]) - before_ok, 1);
    // wrong password after a rejected key: exactly one failed password attempt, no retry
    let before_fail = log_count("sshd-key", &["Failed password"]);
    let (r, _) = connect(
        "sshd-key",
        Some(key("id_other", None)),
        Some("wrong"),
        |_| None,
    )
    .await;
    assert!(
        matches!(r, Err(ConnectError::AuthFailed { .. })),
        "{:?}",
        r.err()
    );
    assert_eq!(log_count("sshd-key", &["Failed password"]) - before_fail, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn unusable_key_files_are_explained_before_any_connection() {
    need_servers!();
    let msg = |r: Result<SftpConnection, ConnectError>| match r {
        Err(ConnectError::KeyError(m)) => m,
        other => panic!("expected KeyError, got {:?}", other.err()),
    };
    let (r, _) = connect("sshd-key", Some(key("id_putty.ppk", None)), None, |_| None).await;
    assert!(msg(r).contains(".ppk"));
    let (r, _) = connect("sshd-key", Some(key("id_ed25519.pub", None)), None, |_| {
        None
    })
    .await;
    assert!(msg(r).contains("public"));
    let (r, _) = connect("sshd-key", Some(key("does-not-exist", None)), None, |_| {
        None
    })
    .await;
    assert!(msg(r).contains("not found"));
    // a server that has public key login switched off says so (sshd-password: PubkeyAuthentication no)
    let (r, _) = connect(
        "sshd-password",
        Some(key("id_ed25519", None)),
        Some(PASS),
        |_| None,
    )
    .await;
    assert!(msg(r).contains("publickey"));
}
