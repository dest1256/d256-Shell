//! Session hub: owns the tokio runtime, open sessions (SFTP + connection data for terminals),
//! terminals and pending user prompts.
//!
//! The UI talks to it through plain request/response calls (works for Tauri and devbridge alike):
//! `connect` blocks while the connection is being established; meanwhile the UI polls `poll_prompts`
//! for questions (unknown host key, password, verification code) and answers with `answer_prompt`.
use crate::known_hosts;
use crate::prompt::PromptKind;
use crate::remote_path;
use crate::service::ApiError;
use crate::sftp::conn::BoxFut;
use crate::sftp::{
    AuthPrompt, ConnectError, ConnectParams, HostKeyInfo, Interaction, SftpConnection,
};
use crate::terminal::args::{build_args, SshTarget};
use crate::terminal::askpass::{self, AskInfo, AskpassServer, SavedPasswordPolicy};
use crate::terminal::session::{ReadResult, TermOptions, TermSession};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, Notify};
use zeroize::Zeroize;

const PROMPT_TIMEOUT: Duration = Duration::from_secs(300);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

enum Answer {
    Accept(bool),
    Text(Option<String>),
}

struct Pending {
    id: u64,
    attempt: String,
    view: Value,
    tx: oneshot::Sender<Answer>,
}

#[derive(Default)]
struct Prompts {
    next: AtomicU64,
    list: Mutex<Vec<Pending>>,
}

impl Prompts {
    /// Register a question and wait for the answer (cancel on timeout).
    async fn ask(&self, attempt: &str, mut view: Value) -> Answer {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        view["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.list.lock().expect("lock").push(Pending {
            id,
            attempt: attempt.to_string(),
            view,
            tx,
        });
        let a = tokio::time::timeout(PROMPT_TIMEOUT, rx).await;
        self.list.lock().expect("lock").retain(|p| p.id != id);
        match a {
            Ok(Ok(a)) => a,
            _ => Answer::Text(None),
        }
    }

    fn cancel_attempt(&self, attempt: &str) {
        let mut l = self.list.lock().expect("lock");
        let mut i = 0;
        while i < l.len() {
            if l[i].attempt == attempt {
                let p = l.remove(i);
                let _ = p.tx.send(Answer::Text(None));
            } else {
                i += 1;
            }
        }
    }
}

/// What a terminal needs to know about an open connection.
pub struct SessionInfo {
    pub server_id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub extra_args: String,
    /// Private key file of this session (key login), and its saved passphrase.
    pub key_path: Option<String>,
    key_passphrase: Option<String>,
    password: Mutex<Option<String>>,
}

impl SessionInfo {
    fn password(&self) -> Option<String> {
        self.password.lock().expect("lock").clone()
    }
}

impl Drop for SessionInfo {
    fn drop(&mut self) {
        if let Some(p) = self.password.lock().expect("lock").as_mut() {
            p.zeroize();
        }
    }
}

struct Session {
    sftp: Option<Arc<SftpConnection>>,
    sftp_error: Option<String>,
    info: Arc<SessionInfo>,
}

pub struct ConnectArgs {
    pub attempt: String,
    pub server_id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub extra_args: String,
    /// Private key file and its saved passphrase (key login).
    pub key_path: Option<String>,
    pub key_passphrase: Option<String>,
}

pub struct ConnectOk {
    pub session_id: String,
    pub home: Option<String>,
    pub sftp_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordMode {
    Askpass,
    Expect,
    Off,
}

impl PasswordMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Askpass => "askpass",
            Self::Expect => "expect",
            Self::Off => "off",
        }
    }
}

pub struct TermContext {
    pub ssh_program: PathBuf,
    pub mode: PasswordMode,
    pub known_hosts: PathBuf,
    /// Executable started by ssh as SSH_ASKPASS (this program in helper mode).
    pub askpass_exe: PathBuf,
}

struct TermEntry {
    session: Arc<TermSession>,
    ssh_session: String,
    _askpass: Option<AskpassServer>,
}

pub struct Hub {
    rt: tokio::runtime::Runtime,
    known_hosts: PathBuf,
    prompts: Arc<Prompts>,
    attempts: Mutex<HashMap<String, Arc<Notify>>>,
    /// Host key info of the last "key changed" refusal per attempt (for the explicit replace action).
    changed: Mutex<HashMap<String, HostKeyInfo>>,
    sessions: Mutex<HashMap<String, Session>>,
    terms: Mutex<HashMap<String, Arc<TermEntry>>>,
    pub transfers: Arc<crate::transfers::Transfers>,
}

fn new_id() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("rng");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn api_err(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError {
        code,
        message: message.into(),
        details: None,
    }
}

impl Hub {
    pub fn new(known_hosts: PathBuf) -> Hub {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let handle = rt.handle().clone();
        Hub {
            rt,
            known_hosts,
            prompts: Arc::new(Prompts::default()),
            attempts: Mutex::new(HashMap::new()),
            changed: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            terms: Mutex::new(HashMap::new()),
            transfers: Arc::new(crate::transfers::Transfers::new(handle)),
        }
    }

    fn interaction(&self, attempt: &str) -> Arc<Interaction> {
        let (p1, a1) = (self.prompts.clone(), attempt.to_string());
        let (p2, a2) = (self.prompts.clone(), attempt.to_string());
        Arc::new(Interaction {
            confirm_host_key: Box::new(move |info| -> BoxFut<bool> {
                let (p, a) = (p1.clone(), a1.clone());
                Box::pin(async move {
                    let view = json!({
                        "kind": "host_key", "host": info.host, "port": info.port,
                        "key_type": info.key_type, "fingerprint": info.fingerprint,
                    });
                    matches!(p.ask(&a, view).await, Answer::Accept(true))
                })
            }),
            prompt: Box::new(move |pr: AuthPrompt| -> BoxFut<Option<String>> {
                let (p, a) = (p2.clone(), a2.clone());
                Box::pin(async move {
                    let view = json!({
                        "kind": "input", "text": pr.text, "echo": pr.echo,
                        "is_password": matches!(pr.kind, PromptKind::Password | PromptKind::Passphrase),
                        "saved_password_rejected": pr.saved_password_rejected,
                    });
                    match p.ask(&a, view).await {
                        Answer::Text(t) => t,
                        Answer::Accept(_) => None,
                    }
                })
            }),
        })
    }

    pub fn connect(&self, a: ConnectArgs) -> Result<ConnectOk, ApiError> {
        let notify = Arc::new(Notify::new());
        self.attempts
            .lock()
            .expect("lock")
            .insert(a.attempt.clone(), notify.clone());
        let params = ConnectParams {
            host: a.host.clone(),
            port: a.port,
            username: a.username.clone(),
            password: a.password.clone(),
            known_hosts: self.known_hosts.clone(),
            connect_timeout: CONNECT_TIMEOUT,
            key: a.key_path.clone().map(|path| crate::sftp::KeyAuth {
                path,
                passphrase: a.key_passphrase.clone().filter(|p| !p.is_empty()),
            }),
        };
        let interaction = self.interaction(&a.attempt);
        let res = self.rt.block_on(async {
            tokio::select! {
                r = SftpConnection::connect(params, interaction) => Some(r),
                _ = notify.notified() => None,
            }
        });
        self.attempts.lock().expect("lock").remove(&a.attempt);
        self.prompts.cancel_attempt(&a.attempt);

        let (sftp, sftp_error) = match res {
            None => return Err(api_err("cancelled", "Connection cancelled.")),
            Some(Ok(conn)) => (Some(Arc::new(conn)), None),
            // Logged in, but no SFTP: the terminal can still work.
            Some(Err(ConnectError::SftpUnavailable(m))) => (None, Some(m)),
            Some(Err(e)) => return Err(self.map_connect_error(&a.attempt, e)),
        };
        let home = sftp.as_ref().map(|c| c.home.clone());
        let id = new_id();
        let info = Arc::new(SessionInfo {
            server_id: a.server_id,
            name: a.name,
            host: a.host,
            port: a.port,
            username: a.username,
            extra_args: a.extra_args,
            password: Mutex::new(a.password.filter(|p| !p.is_empty())),
            key_path: a.key_path,
            key_passphrase: a.key_passphrase.filter(|p| !p.is_empty()),
        });
        // Leftover temporary files of an earlier, dead session of the same server are cleaned by this one.
        if let Some(new_conn) = &sftp {
            for s in self.sessions.lock().expect("lock").values() {
                if s.info.host == info.host
                    && s.info.port == info.port
                    && s.info.username == info.username
                {
                    if let Some(old) = &s.sftp {
                        let stale: Vec<String> =
                            std::mem::take(&mut *old.stale.lock().expect("lock"));
                        new_conn.stale.lock().expect("lock").extend(stale);
                    }
                }
            }
        }
        self.sessions.lock().expect("lock").insert(
            id.clone(),
            Session {
                sftp,
                sftp_error: sftp_error.clone(),
                info,
            },
        );
        Ok(ConnectOk {
            session_id: id,
            home,
            sftp_error,
        })
    }

    fn map_connect_error(&self, attempt: &str, e: ConnectError) -> ApiError {
        let details = match &e {
            ConnectError::HostKeyChanged(i) | ConnectError::HostKeyRevoked(i) => {
                self.changed
                    .lock()
                    .expect("lock")
                    .insert(attempt.to_string(), (**i).clone());
                Some(json!({
                    "host": i.host, "port": i.port, "key_type": i.key_type,
                    "fingerprint": i.fingerprint, "known_types": i.known_types,
                }))
            }
            ConnectError::AuthFailed {
                saved_password_used,
            } => Some(json!({ "saved_password_used": saved_password_used })),
            _ => None,
        };
        ApiError {
            code: e.code(),
            message: e.to_string(),
            details,
        }
    }

    pub fn poll_prompts(&self, attempt: &str) -> Value {
        let l = self.prompts.list.lock().expect("lock");
        json!(l
            .iter()
            .filter(|p| p.attempt == attempt)
            .map(|p| p.view.clone())
            .collect::<Vec<_>>())
    }

    pub fn answer_prompt(
        &self,
        id: u64,
        accept: Option<bool>,
        value: Option<String>,
    ) -> Result<(), ApiError> {
        let mut l = self.prompts.list.lock().expect("lock");
        let Some(pos) = l.iter().position(|p| p.id == id) else {
            return Err(api_err("no_prompt", "The request is already closed."));
        };
        let p = l.remove(pos);
        let ans = match accept {
            Some(a) => Answer::Accept(a),
            None => Answer::Text(value),
        };
        let _ = p.tx.send(ans);
        Ok(())
    }

    pub fn cancel(&self, attempt: &str) {
        if let Some(n) = self.attempts.lock().expect("lock").get(attempt) {
            n.notify_one();
        }
        self.prompts.cancel_attempt(attempt);
    }

    /// Explicit replacement of a changed host key (UI asks for confirmation twice before calling this).
    pub fn replace_host_key(&self, attempt: &str) -> Result<(), ApiError> {
        let Some(i) = self.changed.lock().expect("lock").remove(attempt) else {
            return Err(api_err(
                "no_pending_key",
                "There is no key to replace. Connect again.",
            ));
        };
        known_hosts::replace(&self.known_hosts, &i.host, i.port, &i.key_type, &i.key_b64)
            .map_err(|e| api_err("io", format!("Could not update known_hosts: {e}")))
    }

    fn sftp(&self, id: &str) -> Result<Arc<SftpConnection>, ApiError> {
        let g = self.sessions.lock().expect("lock");
        let s = g
            .get(id)
            .ok_or_else(|| api_err("no_session", "The session is closed."))?;
        s.sftp.clone().ok_or_else(|| {
            api_err(
                "sftp_unavailable",
                s.sftp_error
                    .clone()
                    .unwrap_or_else(|| "SFTP is unavailable.".into()),
            )
        })
    }

    pub fn list_dir(&self, session_id: &str, path: Option<String>) -> Result<Value, ApiError> {
        let conn = self.sftp(session_id)?;
        self.rt.block_on(async {
            let lost = |c: &SftpConnection| c.is_closed();
            let target = match path.as_deref() {
                None | Some("") => conn.home.clone(),
                Some(p) => p.to_string(),
            };
            let canon = conn.canonicalize(&target).await.map_err(|m| {
                if lost(&conn) {
                    api_err("connection_lost", "Connection to the server lost.")
                } else {
                    api_err("sftp", m)
                }
            })?;
            match conn.list_dir(&canon).await {
                Ok(entries) => Ok(json!({ "path": canon, "entries": entries })),
                Err(_) if lost(&conn) => {
                    Err(api_err("connection_lost", "Connection to the server lost."))
                }
                Err(m) => Err(ApiError {
                    code: "sftp",
                    message: m,
                    details: Some(json!({ "path": canon })),
                }),
            }
        })
    }

    /// Explorer write operations. `paths` operations continue past failures and report them per item.
    pub fn fs_op(&self, session_id: &str, op: &str, p: &Value) -> Result<Value, ApiError> {
        let conn = self.sftp(session_id)?;
        let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_owned);
        let need = |k: &str| {
            s(k).ok_or_else(|| api_err("invalid", format!("A parameter is missing {k}.")))
        };
        let paths: Vec<String> = p
            .get("paths")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let fail = |m: String| {
            if conn.is_closed() {
                api_err("connection_lost", "Connection to the server lost.")
            } else {
                api_err("sftp", m)
            }
        };
        self.rt.block_on(async {
            match op {
                "mkdir" => conn
                    .mkdir(&need("parent")?, &need("name")?)
                    .await
                    .map(|p| json!({ "path": p }))
                    .map_err(fail),
                "touch" => conn
                    .create_file(&need("parent")?, &need("name")?)
                    .await
                    .map(|p| json!({ "path": p }))
                    .map_err(fail),
                "rename" => conn
                    .rename_entry(&need("path")?, &need("name")?)
                    .await
                    .map(|p| json!({ "path": p }))
                    .map_err(fail),
                "chmod" => {
                    let mode = p.get("mode").and_then(Value::as_u64).unwrap_or(u64::MAX);
                    let mode = u32::try_from(mode)
                        .map_err(|_| api_err("invalid", "Invalid permissions."))?;
                    let flag = |k: &str, d: bool| p.get(k).and_then(Value::as_bool).unwrap_or(d);
                    conn.chmod_tree(
                        &need("path")?,
                        mode,
                        flag("recursive", false),
                        flag("files", true),
                        flag("dirs", true),
                    )
                    .await
                    .map(|n| json!({ "changed": n }))
                    .map_err(fail)
                }
                "count" => {
                    let mut files = 0;
                    let mut dirs = 0;
                    for path in &paths {
                        let (f, d) = conn.count_tree(path).await.map_err(fail)?;
                        files += f;
                        dirs += d;
                    }
                    Ok(json!({ "files": files, "dirs": dirs }))
                }
                "move" | "copy" | "delete" => {
                    let dest = if op == "delete" {
                        String::new()
                    } else {
                        need("dest")?
                    };
                    let mut done = Vec::new();
                    let mut errors = Vec::new();
                    for path in &paths {
                        let r = match op {
                            "move" => conn.move_entry(path, &dest).await,
                            "copy" => conn.copy_entry(path, &dest).await,
                            _ => conn.delete_entry(path).await.map(|_| path.clone()),
                        };
                        match r {
                            Ok(n) => done.push(n),
                            Err(m) => {
                                if conn.is_closed() {
                                    return Err(api_err(
                                        "connection_lost",
                                        "Connection to the server lost.",
                                    ));
                                }
                                errors.push(json!({ "path": path, "message": m }));
                            }
                        }
                    }
                    Ok(json!({ "done": done, "errors": errors }))
                }
                _ => Err(api_err(
                    "unknown_method",
                    format!("Unknown operation: {op}"),
                )),
            }
        })
    }

    /// Which of `paths` would clash with something at the destination (upload: remote, download: local).
    pub fn transfer_plan(
        &self,
        session_id: &str,
        kind: &str,
        paths: &[String],
        dest: &str,
    ) -> Result<Value, ApiError> {
        let conn = self.sftp(session_id)?;
        self.rt.block_on(async {
            let mut out = Vec::new();
            for p in paths {
                let name = if kind == "upload" {
                    std::path::Path::new(p)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string()
                } else {
                    remote_path::file_name(p).to_string()
                };
                let (exists, is_dir) = if kind == "upload" {
                    let is_dir = std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false);
                    let ex = conn
                        .exists_lstat(&remote_path::join(dest, &name))
                        .await
                        .map_err(|e| api_err("sftp", e.to_string()))?;
                    (ex.is_some(), is_dir)
                } else {
                    let is_dir = conn
                        .sftp
                        .metadata(p.clone())
                        .await
                        .map(|m| m.file_type().is_dir())
                        .unwrap_or(false);
                    (
                        std::path::Path::new(dest)
                            .join(&name)
                            .symlink_metadata()
                            .is_ok(),
                        is_dir,
                    )
                };
                out.push(json!({ "path": p, "name": name, "exists": exists, "is_dir": is_dir }));
            }
            Ok(Value::Array(out))
        })
    }

    pub fn transfer_start(
        &self,
        session_id: &str,
        params: crate::transfers::JobParams,
    ) -> Result<String, ApiError> {
        let conn = self.sftp(session_id)?;
        Ok(self.transfers.start(conn, params))
    }

    /// Editor: open a file (`new` = not yet existing, created on the first save).
    pub fn edit_open(
        &self,
        session_id: &str,
        path: &str,
        limit_mb: u64,
        new: bool,
        encoding: Option<&str>,
    ) -> Result<Value, ApiError> {
        let conn = self.sftp(session_id)?;
        self.rt
            .block_on(conn.edit_open_enc(path, limit_mb * 1024 * 1024, new, encoding))
            .map_err(edit_err)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_save(
        &self,
        session_id: &str,
        path: &str,
        text: &str,
        bom: bool,
        eol: crate::sftp::edit::Eol,
        encoding: &str,
        base: Option<crate::sftp::edit::Base>,
        limit_mb: u64,
        force: bool,
        allow_foreign: bool,
        in_place: bool,
    ) -> Result<Value, ApiError> {
        let conn = self.sftp(session_id)?;
        self.rt
            .block_on(conn.edit_save_enc(path, text, bom, eol, encoding, base.as_ref(), limit_mb * 1024 * 1024, force, allow_foreign, in_place))
            .map_err(|e| {
                if conn.is_closed() && e.code == "sftp" {
                    api_err("connection_lost", "Connection to the server lost. The editor content is kept in the window; reconnect and press Ctrl+S again.")
                } else {
                    edit_err(e)
                }
            })
    }

    /// Server load sample, read from `/proc` over SFTP (no command is run on the server).
    pub fn monitor_sample(&self, session_id: &str, with_info: bool) -> Result<Value, ApiError> {
        let conn = self.sftp(session_id)?;
        let started = std::time::Instant::now();
        match self.rt.block_on(conn.monitor_sample(with_info)) {
            Some(mut v) => {
                v["ms"] = json!(started.elapsed().as_millis() as u64);
                Ok(v)
            }
            None if conn.is_closed() => {
                Err(api_err("connection_lost", "Connection to the server lost."))
            }
            None => Err(api_err(
                "monitor_unavailable",
                "Monitoring is unavailable: the server does not provide /proc (Linux is required).",
            )),
        }
    }
    /// Close a session together with its terminals.
    pub fn disconnect(&self, session_id: &str) {
        self.transfers.cancel_session(session_id);
        let ids: Vec<String> = self
            .terms
            .lock()
            .expect("lock")
            .iter()
            .filter(|(_, t)| t.ssh_session == session_id)
            .map(|(k, _)| k.clone())
            .collect();
        for t in ids {
            self.term_close(&t);
        }
        let s = self.sessions.lock().expect("lock").remove(session_id);
        if let Some(Session { sftp: Some(c), .. }) = s {
            if let Ok(c) = Arc::try_unwrap(c) {
                self.rt.block_on(c.close());
            }
        }
    }

    /// The user entered a new password for a saved server: open sessions of that server (terminal
    /// reconnects) must use it from now on.
    pub fn update_server_password(&self, server_id: &str, password: &str) {
        for s in self.sessions.lock().expect("lock").values() {
            if s.info.server_id.as_deref() == Some(server_id) {
                *s.info.password.lock().expect("lock") = Some(password.to_string());
            }
        }
    }

    // ---------------------------------------------------------------- terminals

    /// Start `ssh` in a PTY for an open session. Returns the terminal id.
    pub fn term_open(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
        ctx: &TermContext,
    ) -> Result<String, ApiError> {
        let info = self
            .sessions
            .lock()
            .expect("lock")
            .get(session_id)
            .map(|s| s.info.clone())
            .ok_or_else(|| api_err("no_session", "The session is closed."))?;
        // key login: check the file first, so that a wrong path is explained here and not as an ssh error in the terminal
        let identity = match &info.key_path {
            Some(p) => Some(crate::keyfile::check(p).map_err(|m| api_err("key_error", m))?),
            None => None,
        };
        let args = build_args(
            &SshTarget {
                host: &info.host,
                port: info.port,
                username: &info.username,
                extra: &info.extra_args,
                identity: identity.as_deref(),
            },
            &ctx.known_hosts,
        )
        .map_err(|e| api_err("invalid_args", e.0))?;

        let term_id = new_id();
        let mut env: Vec<(String, String)> = Vec::new();
        let mut expect_password = None;
        let mut server = None;
        // Filled after spawn; lets the askpass server kill ssh when the user refuses to answer.
        let victim: Arc<Mutex<Option<std::sync::Weak<TermSession>>>> = Arc::new(Mutex::new(None));
        match ctx.mode {
            PasswordMode::Askpass => {
                let v = victim.clone();
                let on_refuse: askpass::OnRefuse = Arc::new(move || {
                    let s = v
                        .lock()
                        .expect("lock")
                        .as_ref()
                        .and_then(std::sync::Weak::upgrade);
                    if let Some(s) = s {
                        s.close();
                    }
                });
                let s = AskpassServer::start(
                    self.askpass_handler(&term_id, info.password(), info.key_passphrase.clone()),
                    Some(on_refuse),
                )
                .map_err(|e| api_err("io", format!("Could not start askpass: {e}")))?;
                env.push((
                    "SSH_ASKPASS".into(),
                    askpass::askpass_program_path(&ctx.askpass_exe),
                ));
                env.push(("SSH_ASKPASS_REQUIRE".into(), "force".into()));
                env.push((askpass::ENV_FLAG.into(), "1".into()));
                env.push((askpass::ENV_ADDR.into(), s.addr.to_string()));
                env.push((askpass::ENV_TOKEN.into(), s.token.clone()));
                server = Some(s);
            }
            PasswordMode::Expect => expect_password = info.password(),
            PasswordMode::Off => {}
        }
        let session = TermSession::spawn(TermOptions {
            program: ctx.ssh_program.clone(),
            args,
            env,
            cols,
            rows,
            expect_password,
        })
        .map_err(|m| api_err("spawn", m))?;
        *victim.lock().expect("lock") = Some(Arc::downgrade(&session));
        self.terms.lock().expect("lock").insert(
            term_id.clone(),
            Arc::new(TermEntry {
                session,
                ssh_session: session_id.to_string(),
                _askpass: server,
            }),
        );
        Ok(term_id)
    }

    /// Answers ssh's askpass requests: the saved password exactly once for an ordinary password prompt,
    /// everything else (2FA codes, a second password request after a wrong one) goes to the user.
    fn askpass_handler(
        &self,
        term_id: &str,
        saved: Option<String>,
        passphrase: Option<String>,
    ) -> askpass::Handler {
        let prompts = self.prompts.clone();
        let handle = self.rt.handle().clone();
        let tid = term_id.to_string();
        let policy = SavedPasswordPolicy::new(saved).with_passphrase(passphrase);
        Arc::new(move |prompt: &str| -> Option<String> {
            policy.decide(prompt, &|i: AskInfo<'_>| {
                let view = json!({
                    "kind": "input", "text": i.prompt, "echo": false,
                    "is_password": i.is_password,
                    "saved_password_rejected": i.saved_password_rejected,
                });
                match handle.block_on(prompts.ask(&tid, view)) {
                    Answer::Text(t) => t,
                    Answer::Accept(_) => None,
                }
            })
        })
    }

    fn term(&self, id: &str) -> Result<Arc<TermEntry>, ApiError> {
        self.terms
            .lock()
            .expect("lock")
            .get(id)
            .cloned()
            .ok_or_else(|| api_err("no_terminal", "The terminal is closed."))
    }

    pub fn term_read(&self, id: &str, timeout: Duration) -> Result<ReadResult, ApiError> {
        Ok(self.term(id)?.session.read(timeout))
    }

    pub fn term_write(&self, id: &str, data: &[u8]) -> Result<(), ApiError> {
        self.term(id)?
            .session
            .write(data)
            .map_err(|e| api_err("io", format!("Could not write to the terminal: {e}")))
    }

    pub fn term_resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), ApiError> {
        self.term(id)?
            .session
            .resize(cols, rows)
            .map_err(|e| api_err("io", e))
    }

    pub fn term_session(&self, id: &str) -> Option<String> {
        self.term(id).ok().map(|t| t.ssh_session.clone())
    }

    pub fn term_pid(&self, id: &str) -> Option<u32> {
        self.term(id).ok().and_then(|t| t.session.pid())
    }

    /// What the terminal's shell is doing (folder, foreground program, virtualenv), read from `/proc` over SFTP.
    pub fn term_context(&self, term_id: &str) -> Result<Value, ApiError> {
        let t = self.term(term_id)?;
        let conn = self.sftp(&t.ssh_session)?;
        let (port, pid) = {
            let g = self.sessions.lock().expect("lock");
            let port = g.get(&t.ssh_session).map(|s| s.info.port).unwrap_or(22);
            (port, t.session.pid())
        };
        let pid = pid.ok_or_else(|| api_err("no_terminal", "The terminal is not running."))?;
        let Some(local) = crate::terminal::localport::local_port(pid, port) else {
            return Ok(
                json!({ "found": false, "reason": "Could not determine the connection port of the terminal." }),
            );
        };
        self.rt
            .block_on(conn.shell_context(local))
            .map_err(|m| api_err("sftp", m))
    }
    /// Kill ssh, close the PTY and forget the terminal.
    pub fn term_close(&self, id: &str) {
        let t = self.terms.lock().expect("lock").remove(id);
        self.prompts.cancel_attempt(id);
        if let Some(t) = t {
            t.session.close();
        }
    }
}

fn edit_err(e: crate::sftp::edit::EditError) -> ApiError {
    ApiError {
        code: e.code,
        message: e.message,
        details: e.details,
    }
}
