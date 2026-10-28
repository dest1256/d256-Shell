//! DEV ONLY. Serves `ui/dist` and `POST /rpc` ({"method","params"}) backed by the real core.
//! Listens on 127.0.0.1 only. Env: D256SHELL_DATA_DIR (required), DEVBRIDGE_PORT (default 5174),
//! DEVBRIDGE_UI (default ui/dist).
use d256shell_core::{paths, service::Service};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

fn respond(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn ctype(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn handle(mut s: TcpStream, svc: &Service, ui: &Path) {
    let mut r = BufReader::new(s.try_clone().expect("clone"));
    let mut line = String::new();
    if r.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if len > 8 * 1024 * 1024 {
        return respond(&mut s, "413 Payload Too Large", "text/plain", b"too large");
    }
    let mut body = vec![0u8; len];
    if r.read_exact(&mut body).is_err() {
        return;
    }

    if method == "POST" && target == "/rpc" {
        let out = match serde_json::from_slice::<Value>(&body) {
            Ok(req) => {
                let m = req["method"].as_str().unwrap_or("");
                match svc.call(m, req.get("params").cloned().unwrap_or(json!({}))) {
                    Ok(v) => json!({"ok": true, "result": v}),
                    Err(e) => json!({"ok": false, "error": e}),
                }
            }
            Err(e) => json!({"ok": false, "error": {"code": "invalid", "message": e.to_string()}}),
        };
        return respond(
            &mut s,
            "200 OK",
            "application/json",
            out.to_string().as_bytes(),
        );
    }

    if method == "GET" {
        let rel = target
            .split('?')
            .next()
            .unwrap_or("/")
            .trim_start_matches('/');
        let rel = if rel.is_empty() { "index.html" } else { rel };
        // reject traversal: only plain path components
        let safe = Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
        if safe {
            let p: PathBuf = ui.join(rel);
            if let Ok(data) = std::fs::read(&p) {
                return respond(&mut s, "200 OK", ctype(&p), &data);
            }
        }
    }
    respond(&mut s, "404 Not Found", "text/plain", b"not found");
}

fn main() {
    // Same executable doubles as the SSH_ASKPASS helper (tests use this binary for that).
    d256shell_core::terminal::askpass::run_helper_if_requested();
    let dirs = paths::init().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    if std::env::var(paths::ENV_DATA_DIR).is_err() {
        eprintln!("devbridge requires D256SHELL_DATA_DIR (it must never touch a real data folder)");
        std::process::exit(1);
    }
    let port: u16 = std::env::var("DEVBRIDGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5174);
    let ui = PathBuf::from(std::env::var("DEVBRIDGE_UI").unwrap_or_else(|_| "ui/dist".into()));
    let svc = Arc::new(Service::new(dirs));
    let l = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    println!("devbridge listening on http://127.0.0.1:{port}");
    for conn in l.incoming().flatten() {
        let (svc, ui) = (svc.clone(), ui.clone());
        std::thread::spawn(move || handle(conn, &svc, &ui));
    }
}
