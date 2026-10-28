//! End-to-end through `Service::call`, the same API the UI uses: connect blocks in one thread while
//! the "UI" polls and answers prompts from another.
mod common;
use common::*;
use d256shell_core::paths;
use d256shell_core::service::{ApiError, Service};
use d256shell_core::vault::KdfParams;
use serde_json::{json, Value};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn service(name: &str) -> Arc<Service> {
    let dirs = paths::prepare(tmp(name)).unwrap();
    let svc = Service::with_kdf(
        dirs,
        KdfParams {
            m_cost_kib: 64,
            t_cost: 1,
            p_cost: 1,
        },
    );
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

fn connect_in_thread(
    svc: &Arc<Service>,
    params: Value,
) -> thread::JoinHandle<Result<Value, ApiError>> {
    let svc = svc.clone();
    thread::spawn(move || svc.call("connect", params))
}

/// Wait for a pending prompt of this attempt (what the UI does by polling).
fn wait_prompt(svc: &Service, attempt: &str) -> Value {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(20) {
        let l = svc
            .call("poll_prompts", json!({"attempt": attempt}))
            .unwrap();
        if let Some(p) = l.as_array().and_then(|a| a.first()) {
            return p.clone();
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("no prompt appeared");
}

macro_rules! need_servers {
    () => {
        if !enabled() {
            return;
        }
    };
}

#[test]
fn unknown_host_prompt_then_session_and_listing() {
    need_servers!();
    let svc = service("flow1");
    let id = add_server(&svc, "sshd-password", PASS);
    let h = connect_in_thread(&svc, json!({"server_id": id, "attempt": "a1"}));
    let p = wait_prompt(&svc, "a1");
    assert_eq!(p["kind"], "host_key");
    assert!(p["fingerprint"].as_str().unwrap().starts_with("SHA256:"));
    svc.call("answer_prompt", json!({"id": p["id"], "accept": true}))
        .unwrap();
    let r = h.join().unwrap().unwrap();
    let sid = r["session_id"].as_str().unwrap();
    assert_eq!(r["home"], "/home/tester");

    let l = svc.call("list_dir", json!({"session_id": sid})).unwrap();
    assert_eq!(l["path"], "/home/tester");
    assert!(l["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["name"] == "zone"));
    let l = svc
        .call(
            "list_dir",
            json!({"session_id": sid, "path": "/home/tester/zone/link_to_protected"}),
        )
        .unwrap();
    assert_eq!(
        l["path"], "/protected",
        "path is canonicalised through the symlink"
    );
    let e = svc
        .call("list_dir", json!({"session_id": sid, "path": "/root"}))
        .unwrap_err();
    assert_eq!(e.code, "sftp");
    svc.call("disconnect", json!({"session_id": sid})).unwrap();
    assert_eq!(
        svc.call("list_dir", json!({"session_id": sid}))
            .unwrap_err()
            .code,
        "no_session"
    );

    // remembered: no prompt this time
    let r = svc
        .call("connect", json!({"server_id": id, "attempt": "a2"}))
        .unwrap();
    svc.call("disconnect", json!({"session_id": r["session_id"]}))
        .unwrap();
}

#[test]
fn two_factor_prompt_is_delivered_to_the_ui() {
    need_servers!();
    let svc = service("flow2");
    let id = add_server(&svc, "sshd-2fa", PASS);
    let h = connect_in_thread(&svc, json!({"server_id": id, "attempt": "b1"}));
    let p = wait_prompt(&svc, "b1");
    svc.call("answer_prompt", json!({"id": p["id"], "accept": true}))
        .unwrap(); // trust host
    let p = wait_prompt(&svc, "b1");
    assert_eq!(p["kind"], "input");
    assert!(p["text"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("verification code"));
    assert_eq!(p["is_password"], false);
    svc.call("answer_prompt", json!({"id": p["id"], "value": totp_now()}))
        .unwrap();
    let r = h.join().unwrap().unwrap();
    svc.call("disconnect", json!({"session_id": r["session_id"]}))
        .unwrap();
}

#[test]
fn cancel_while_prompt_is_pending() {
    need_servers!();
    let svc = service("flow3");
    let id = add_server(&svc, "sshd-password", PASS);
    let h = connect_in_thread(&svc, json!({"server_id": id, "attempt": "c1"}));
    let _ = wait_prompt(&svc, "c1");
    svc.call("cancel_connect", json!({"attempt": "c1"}))
        .unwrap();
    let e = h.join().unwrap().unwrap_err();
    assert_eq!(e.code, "cancelled");
    assert_eq!(
        svc.call("poll_prompts", json!({"attempt": "c1"})).unwrap(),
        json!([])
    );
}

#[test]
fn wrong_saved_password_then_user_password_is_saved_after_success() {
    need_servers!();
    let svc = service("flow4");
    let id = add_server(&svc, "sshd-password", "wrong-saved");
    // trust the host first so only the login matters
    let h = connect_in_thread(&svc, json!({"server_id": id, "attempt": "d1"}));
    let p = wait_prompt(&svc, "d1");
    svc.call("answer_prompt", json!({"id": p["id"], "accept": true}))
        .unwrap();
    let e = h.join().unwrap().unwrap_err();
    assert_eq!(e.code, "auth_failed");
    assert_eq!(e.details.unwrap()["saved_password_used"], true);

    let r = svc
        .call(
            "connect",
            json!({"server_id": id, "attempt": "d2", "password": PASS, "save_password": true}),
        )
        .unwrap();
    svc.call("disconnect", json!({"session_id": r["session_id"]}))
        .unwrap();
    assert_eq!(
        svc.call("get_password", json!({"id": id})).unwrap(),
        json!(PASS)
    );
}

#[test]
fn changed_key_is_refused_and_replaced_only_by_explicit_action() {
    need_servers!();
    let svc = service("flow5");
    // Trust sshd-otherkey's key, then pretend it is the key of sshd-password by editing known_hosts.
    let other = add_server(&svc, "sshd-otherkey", PASS);
    let h = connect_in_thread(&svc, json!({"server_id": other, "attempt": "e1"}));
    let p = wait_prompt(&svc, "e1");
    svc.call("answer_prompt", json!({"id": p["id"], "accept": true}))
        .unwrap();
    let r = h.join().unwrap().unwrap();
    svc.call("disconnect", json!({"session_id": r["session_id"]}))
        .unwrap();

    let kh = svc_known_hosts(&svc);
    let text = std::fs::read_to_string(&kh).unwrap();
    std::fs::write(&kh, text.replace("sshd-otherkey", "sshd-password")).unwrap();

    let id = add_server(&svc, "sshd-password", PASS);
    let e = svc
        .call("connect", json!({"server_id": id, "attempt": "e2"}))
        .unwrap_err();
    assert_eq!(e.code, "host_key_changed");
    let d = e.details.unwrap();
    assert!(d["fingerprint"].as_str().unwrap().starts_with("SHA256:"));
    assert_eq!(d["host"], "sshd-password");

    // Still refused on retry: nothing was replaced implicitly.
    assert_eq!(
        svc.call("connect", json!({"server_id": id, "attempt": "e3"}))
            .unwrap_err()
            .code,
        "host_key_changed"
    );
    // Explicit replacement, then it works without a prompt.
    svc.call("replace_host_key", json!({"attempt": "e3"}))
        .unwrap();
    let r = svc
        .call("connect", json!({"server_id": id, "attempt": "e4"}))
        .unwrap();
    svc.call("disconnect", json!({"session_id": r["session_id"]}))
        .unwrap();
}

#[test]
fn quick_connect_without_saved_password_asks_for_it() {
    need_servers!();
    let svc = service("flow6");
    let h = connect_in_thread(
        &svc,
        json!({"host": "sshd-password", "port": 22, "username": USER, "attempt": "f1"}),
    );
    let p = wait_prompt(&svc, "f1");
    svc.call("answer_prompt", json!({"id": p["id"], "accept": true}))
        .unwrap();
    let p = wait_prompt(&svc, "f1");
    assert_eq!(p["kind"], "input");
    assert_eq!(p["is_password"], true);
    svc.call("answer_prompt", json!({"id": p["id"], "value": PASS}))
        .unwrap();
    let r = h.join().unwrap().unwrap();
    svc.call("disconnect", json!({"session_id": r["session_id"]}))
        .unwrap();
    // option injection through quick connect is rejected
    let e = svc
        .call(
            "connect",
            json!({"host": "-oProxyCommand=x", "username": USER, "attempt": "f2"}),
        )
        .unwrap_err();
    assert_eq!(e.code, "invalid");
}

/// The reported case "SFTP: sender dropped": the server logs the user in but has no SFTP subsystem.
/// The session must still open (the terminal is a plain ssh shell) and explain why the file panel is empty.
#[test]
fn server_without_sftp_still_gives_a_session() {
    need_servers!();
    let svc = service("flow7");
    let id = add_server(&svc, "sshd-nosftp", PASS);
    let h = connect_in_thread(&svc, json!({"server_id": id, "attempt": "g1"}));
    let p = wait_prompt(&svc, "g1");
    svc.call("answer_prompt", json!({"id": p["id"], "accept": true}))
        .unwrap();
    let r = h.join().unwrap().unwrap();
    assert_eq!(r["home"], json!(null));
    let msg = r["sftp_error"].as_str().expect("sftp_error is reported");
    assert!(
        msg.contains("SFTP") && msg.contains("Терминал работает"),
        "{msg}"
    );
    let sid = r["session_id"].as_str().unwrap();
    let e = svc
        .call("list_dir", json!({"session_id": sid}))
        .unwrap_err();
    assert_eq!(e.code, "sftp_unavailable");
    svc.call("disconnect", json!({"session_id": sid})).unwrap();
}

fn svc_known_hosts(svc: &Service) -> std::path::PathBuf {
    svc.known_hosts_path()
}
