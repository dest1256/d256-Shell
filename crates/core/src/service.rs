//! Transport-independent application API. Tauri and devbridge both just forward
//! `call(method, params)` here, so the UI can be tested against the real core.
use crate::hub::{ConnectArgs, Hub, PasswordMode, TermContext};
use crate::paths::DataDirs;
use crate::settings::{AutoPasswordMode, Settings};
use crate::terminal::session::ReadResult;
use crate::terminal::ssh;
use crate::vault::{KdfParams, Server, Vault, VaultError};
use base64ct::{Base64, Encoding};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    /// Machine readable extras (e.g. host key fingerprint for a "key changed" refusal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ApiError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }
}

impl From<VaultError> for ApiError {
    fn from(e: VaultError) -> Self {
        let code = match &e {
            VaultError::Missing { .. } => "vault_missing",
            VaultError::Corrupt(_) => "vault_corrupt",
            VaultError::UnsupportedVersion(_) => "vault_version",
            VaultError::AuthFailed => "auth_failed",
            VaultError::Io(_) => "io",
            VaultError::Crypto(_) => "crypto",
        };
        Self::new(code, e.to_string())
    }
}

/// What the UI is allowed to see about a server: never the password.
#[derive(Debug, Serialize)]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub initial_dir: Option<String>,
    pub ssh_extra_args: Option<String>,
    pub has_password: bool,
    pub key_path: Option<String>,
    pub has_key_passphrase: bool,
}

impl From<&Server> for ServerView {
    fn from(s: &Server) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            host: s.host.clone(),
            port: s.port,
            username: s.username.clone(),
            initial_dir: s.initial_dir.clone(),
            ssh_extra_args: s.ssh_extra_args.clone(),
            has_password: !s.password.is_empty(),
            key_path: s.key_path.clone(),
            has_key_passphrase: !s.key_passphrase.is_empty(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ServerInput {
    id: Option<String>,
    name: String,
    host: String,
    port: u16,
    username: String,
    /// `None` keeps the stored password (edit), `Some("")` clears it.
    password: Option<String>,
    initial_dir: Option<String>,
    ssh_extra_args: Option<String>,
    /// Private key file for key login; empty / None = no key (and the saved passphrase is dropped).
    key_path: Option<String>,
    /// `None` keeps the stored passphrase (edit), `Some("")` clears it.
    key_passphrase: Option<String>,
}

pub struct Service {
    dirs: DataDirs,
    kdf: KdfParams,
    vault: Mutex<Option<Vault>>,
    hub: Hub,
    settings: Mutex<Settings>,
    /// Program that ssh starts as SSH_ASKPASS: this executable (it switches to helper mode by env var).
    askpass_exe: std::path::PathBuf,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> Result<String, ApiError> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| ApiError::new("crypto", e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn norm(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Validate user-supplied connection fields. Host and user end up in ssh argv, so a leading `-`
/// (option injection) and whitespace/control characters are rejected.
fn validate_target(host: &str, username: &str, port: u16) -> Result<(), ApiError> {
    let bad = |c: char| c.is_whitespace() || c.is_control();
    if host.is_empty() || host.starts_with('-') || host.contains(bad) || host.contains('@') {
        return Err(ApiError::new("invalid", "Некорректный адрес сервера."));
    }
    if username.is_empty() || username.starts_with('-') || username.contains(bad) {
        return Err(ApiError::new("invalid", "Некорректное имя пользователя."));
    }
    if port == 0 {
        return Err(ApiError::new("invalid", "Порт должен быть от 1 до 65535."));
    }
    Ok(())
}

fn validate(i: &ServerInput) -> Result<(), ApiError> {
    if i.name.trim().is_empty() {
        return Err(ApiError::new("invalid", "Укажите название сервера."));
    }
    validate_target(&i.host, &i.username, i.port)
}

impl Service {
    pub fn new(dirs: DataDirs) -> Self {
        Self::with_kdf(dirs, KdfParams::default())
    }

    /// Cheaper KDF parameters are for tests only.
    pub fn with_kdf(dirs: DataDirs, kdf: KdfParams) -> Self {
        let hub = Hub::new(dirs.known_hosts());
        let settings = Settings::load(&dirs.settings());
        hub.transfers
            .set_parallel(settings.parallel_transfers as usize);
        Self {
            dirs,
            kdf,
            vault: Mutex::new(None),
            hub,
            settings: Mutex::new(settings),
            askpass_exe: std::env::current_exe().unwrap_or_default(),
        }
    }

    /// Use another executable as the askpass helper (tests: the test binary itself has no helper mode).
    pub fn with_askpass_exe(mut self, exe: std::path::PathBuf) -> Self {
        self.askpass_exe = exe;
        self
    }

    fn settings_snapshot(&self) -> Settings {
        self.settings.lock().expect("lock").clone()
    }

    /// Locate ssh and read its version.
    fn resolve_ssh(&self) -> Result<(std::path::PathBuf, ssh::SshVersion), ApiError> {
        let s = self.settings_snapshot();
        let path = ssh::find_ssh(&s.ssh_path, &self.dirs.openssh()).ok_or_else(|| {
            ApiError::new(
                "ssh_not_found",
                if cfg!(windows) {
                    "Не найден ssh.exe. Установите OpenSSH-клиент Windows или укажите путь в настройках (или положите портативный OpenSSH в папку data\\OpenSSH)."
                } else {
                    "Не найден ssh. Установите клиент OpenSSH (Ubuntu: sudo apt install openssh-client; Arch: sudo pacman -S openssh) или укажите путь в настройках."
                },
            )
        })?;
        let v = ssh::detect(&path).map_err(|m| ApiError::new("ssh_failed", m))?;
        Ok((path, v))
    }

    /// Effective automatic password mode for this ssh version.
    fn effective_mode(&self, v: &ssh::SshVersion) -> PasswordMode {
        match self.settings_snapshot().auto_password {
            AutoPasswordMode::Off => PasswordMode::Off,
            AutoPasswordMode::Expect => PasswordMode::Expect,
            // Old ssh ignores SSH_ASKPASS_REQUIRE: fall back to the guarded expect mode.
            AutoPasswordMode::Askpass | AutoPasswordMode::Auto => {
                if v.supports_askpass_force() {
                    PasswordMode::Askpass
                } else {
                    PasswordMode::Expect
                }
            }
        }
    }

    /// Process id of the terminal's ssh (diagnostics and tests).
    pub fn term_session_id(&self, term_id: &str) -> Option<String> {
        self.hub.term_session(term_id)
    }

    pub fn term_pid(&self, term_id: &str) -> Option<u32> {
        self.hub.term_pid(term_id)
    }

    /// Raw terminal output for streaming transports (Tauri channel).
    pub fn term_read_raw(
        &self,
        term_id: &str,
        timeout: std::time::Duration,
    ) -> Result<ReadResult, ApiError> {
        self.hub.term_read(term_id, timeout)
    }

    fn term_open(&self, params: &Value) -> Result<Value, ApiError> {
        let sid = params
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new("invalid", "Нет параметра session_id"))?;
        let dim = |k: &str, d: u64| {
            params
                .get(k)
                .and_then(Value::as_u64)
                .unwrap_or(d)
                .clamp(1, 1000) as u16
        };
        let (program, version) = self.resolve_ssh()?;
        let mode = self.effective_mode(&version);
        let ctx = TermContext {
            ssh_program: program,
            mode,
            known_hosts: self.dirs.known_hosts(),
            askpass_exe: self.askpass_exe.clone(),
        };
        let term_id = self
            .hub
            .term_open(sid, dim("cols", 80), dim("rows", 24), &ctx)?;
        Ok(json!({ "term_id": term_id, "mode": mode.name(), "ssh_version": version.raw }))
    }

    fn ssh_info(&self) -> Value {
        match self.resolve_ssh() {
            Ok((path, v)) => json!({
                "found": true, "path": path.to_string_lossy(), "version": v.raw,
                "askpass_supported": v.supports_askpass_force(),
                "mode": self.effective_mode(&v).name(),
            }),
            Err(e) => json!({ "found": false, "error": e.message }),
        }
    }

    fn set_settings(&self, patch: &Value) -> Result<Value, ApiError> {
        let mut cur = serde_json::to_value(self.settings_snapshot()).expect("serialize");
        if let (Some(c), Some(p)) = (cur.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                c.insert(k.clone(), v.clone());
            }
        }
        let new: Settings = serde_json::from_value(cur)
            .map_err(|e| ApiError::new("invalid", format!("Неверные настройки: {e}")))?;
        let new = new.sanitized();
        new.save(&self.dirs.settings())
            .map_err(|e| ApiError::new("io", format!("Не удалось сохранить настройки: {e}")))?;
        self.hub
            .transfers
            .set_parallel(new.parallel_transfers as usize);
        *self.settings.lock().expect("lock") = new.clone();
        Ok(serde_json::to_value(new).expect("serialize"))
    }

    /// Path of the `known_hosts` file in use (shown in diagnostics and used by tests).
    pub fn staging_dir(&self) -> std::path::PathBuf {
        self.dirs.staging()
    }

    pub fn known_hosts_path(&self) -> std::path::PathBuf {
        self.dirs.known_hosts()
    }

    fn status(&self) -> Value {
        let unlocked = self.vault.lock().map(|g| g.is_some()).unwrap_or(false);
        let state = if unlocked {
            "unlocked"
        } else if self.dirs.vault().exists() || self.dirs.vault_bak().exists() {
            // A lone .bak means vault.bin was lost mid-write: unlock will report Missing{backup_exists}.
            "locked"
        } else {
            "no_vault"
        };
        json!({ "state": state, "backup_exists": self.dirs.vault_bak().exists() })
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        let p = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_owned);
        let need =
            |k: &str| p(k).ok_or_else(|| ApiError::new("invalid", format!("Нет параметра {k}")));
        match method {
            "status" => Ok(self.status()),
            "create_vault" => {
                let master = need("master")?;
                if master.chars().count() < 8 {
                    return Err(ApiError::new(
                        "invalid",
                        "Мастер-пароль должен быть не короче 8 символов.",
                    ));
                }
                let v = Vault::create(&self.dirs.vault(), &master, self.kdf)?;
                *self.lock()? = Some(v);
                Ok(self.status())
            }
            "unlock" => {
                let v = Vault::unlock(&self.dirs.vault(), &need("master")?)?;
                *self.lock()? = Some(v);
                Ok(self.status())
            }
            "restore_backup" => {
                let v = Vault::restore_from_backup(&self.dirs.vault(), &need("master")?)?;
                *self.lock()? = Some(v);
                Ok(self.status())
            }
            "lock" => {
                *self.lock()? = None;
                Ok(self.status())
            }
            "change_master" => {
                let (old, new) = (need("old")?, need("new")?);
                if new.chars().count() < 8 {
                    return Err(ApiError::new(
                        "invalid",
                        "Мастер-пароль должен быть не короче 8 символов.",
                    ));
                }
                Vault::unlock(&self.dirs.vault(), &old)?; // verify the old password against the file
                self.with_vault(|v| v.change_master(&new).map(|_| json!({})))
            }
            "list_servers" => self.with_vault(|v| {
                let list: Vec<ServerView> = v.data.servers.iter().map(ServerView::from).collect();
                Ok(json!(list))
            }),
            "get_password" => {
                let id = need("id")?;
                self.with_vault(|v| {
                    Ok(json!(v
                        .data
                        .servers
                        .iter()
                        .find(|s| s.id == id)
                        .map(|s| s.password.clone())))
                })
            }
            "save_server" => {
                let input: ServerInput = serde_json::from_value(params.clone())
                    .map_err(|e| ApiError::new("invalid", e.to_string()))?;
                validate(&input)?;
                let id_new = new_id()?;
                self.with_vault(|v| {
                    let t = now();
                    let mut input = input;
                    let idx = input
                        .id
                        .as_ref()
                        .and_then(|id| v.data.servers.iter().position(|s| &s.id == id));
                    let key_path = norm(input.key_path.take());
                    let (id, created, password) = match idx {
                        Some(i) => {
                            let old = &v.data.servers[i];
                            (
                                old.id.clone(),
                                old.created,
                                input
                                    .password
                                    .take()
                                    .unwrap_or_else(|| old.password.clone()),
                            )
                        }
                        None => (id_new, t, input.password.take().unwrap_or_default()),
                    };
                    let old_pp = idx
                        .map(|i| v.data.servers[i].key_passphrase.clone())
                        .unwrap_or_default();
                    let key_passphrase = match (&key_path, input.key_passphrase.take()) {
                        (None, _) => String::new(),
                        (Some(_), Some(p)) => p,
                        (Some(_), None) => old_pp,
                    };
                    let s = Server {
                        id: id.clone(),
                        name: input.name.trim().to_string(),
                        host: input.host,
                        port: input.port,
                        username: input.username,
                        password,
                        initial_dir: norm(input.initial_dir),
                        ssh_extra_args: norm(input.ssh_extra_args),
                        key_path,
                        key_passphrase,
                        created,
                        updated: t,
                    };
                    match idx {
                        Some(i) => v.data.servers[i] = s,
                        None => v.data.servers.push(s),
                    }
                    v.save().map(|_| json!({ "id": id }))
                })
            }
            "duplicate_server" => {
                let id = need("id")?;
                let id_new = new_id()?;
                self.with_vault(|v| {
                    let Some(src) = v.data.servers.iter().find(|s| s.id == id).cloned() else {
                        return Ok(json!(null));
                    };
                    let mut copy = src.clone();
                    copy.id = id_new.clone();
                    copy.name = format!("{} (копия)", src.name);
                    copy.created = now();
                    copy.updated = copy.created;
                    v.data.servers.push(copy);
                    v.save().map(|_| json!({ "id": id_new }))
                })
            }
            "connect" => self.connect(&params),
            "get_settings" => {
                Ok(serde_json::to_value(self.settings_snapshot()).expect("serialize"))
            }
            "set_settings" => self.set_settings(&params),
            "ssh_info" => Ok(self.ssh_info()),
            "term_open" => self.term_open(&params),
            "term_read" => {
                let wait = params
                    .get("wait_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(200)
                    .min(5000);
                let r = self
                    .hub
                    .term_read(&need("term_id")?, std::time::Duration::from_millis(wait))?;
                Ok(match r {
                    ReadResult::Data(d) => {
                        json!({ "data": Base64::encode_string(&d), "exit": null })
                    }
                    ReadResult::Timeout => json!({ "data": null, "exit": null }),
                    ReadResult::Exited(c) => json!({ "data": null, "exit": c }),
                })
            }
            "term_write" => {
                let data = Base64::decode_vec(&need("data_b64")?)
                    .map_err(|_| ApiError::new("invalid", "Некорректные данные ввода."))?;
                self.hub.term_write(&need("term_id")?, &data)?;
                Ok(json!({}))
            }
            "term_resize" => {
                let g = |k: &str| {
                    params
                        .get(k)
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        .clamp(1, 1000) as u16
                };
                self.hub
                    .term_resize(&need("term_id")?, g("cols"), g("rows"))?;
                Ok(json!({}))
            }
            "term_close" => {
                self.hub.term_close(&need("term_id")?);
                Ok(json!({}))
            }
            "set_server_password" => {
                let (id, pw) = (need("id")?, need("password")?);
                self.hub.update_server_password(&id, &pw);
                self.with_vault(|v| {
                    if let Some(x) = v.data.servers.iter_mut().find(|x| x.id == id) {
                        x.password = pw.clone();
                        x.updated = now();
                    }
                    v.save().map(|_| json!({}))
                })
            }
            "poll_prompts" => Ok(self.hub.poll_prompts(&need("attempt")?)),
            "answer_prompt" => {
                let id = params
                    .get("id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| ApiError::new("invalid", "Нет параметра id"))?;
                self.hub.answer_prompt(
                    id,
                    params.get("accept").and_then(Value::as_bool),
                    p("value"),
                )?;
                Ok(json!({}))
            }
            "cancel_connect" => {
                self.hub.cancel(&need("attempt")?);
                Ok(json!({}))
            }
            "replace_host_key" => {
                self.hub.replace_host_key(&need("attempt")?)?;
                Ok(json!({}))
            }
            "list_dir" => self.hub.list_dir(&need("session_id")?, p("path")),
            "fs" => self.hub.fs_op(&need("session_id")?, &need("op")?, &params),
            "term_context" => self.hub.term_context(&need("term_id")?),
            "monitor_sample" => {
                let info = params.get("info").and_then(Value::as_bool).unwrap_or(false);
                self.hub.monitor_sample(&need("session_id")?, info)
            }
            m if m.starts_with("transfer_") || m == "stage_clear" => self.transfer_call(m, &params),
            m if m.starts_with("edit_") => self.edit_call(m, &params),
            "disconnect" => {
                self.hub.disconnect(&need("session_id")?);
                Ok(json!({}))
            }
            "delete_server" => {
                let id = need("id")?;
                self.with_vault(|v| {
                    v.data.servers.retain(|s| s.id != id);
                    v.save().map(|_| json!({}))
                })
            }
            _ => Err(ApiError::new(
                "unknown_method",
                format!("Неизвестный метод: {method}"),
            )),
        }
    }

    /// Editor API (`edit_open`, `edit_save`).
    fn edit_call(&self, method: &str, p: &Value) -> Result<Value, ApiError> {
        use crate::sftp::edit::{Base, Eol};
        let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_owned);
        let need =
            |k: &str| s(k).ok_or_else(|| ApiError::new("invalid", format!("Нет параметра {k}")));
        let flag = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
        let limit = u64::from(self.settings_snapshot().editor_limit_mb);
        match method {
            "edit_open" => self.hub.edit_open(
                &need("session_id")?,
                &need("path")?,
                limit,
                flag("new"),
                s("encoding").as_deref(),
            ),
            "edit_save" => {
                let eol = match s("eol").as_deref() {
                    Some("crlf") => Eol::Crlf,
                    _ => Eol::Lf,
                };
                let base = p.get("base").and_then(Base::from_json);
                self.hub.edit_save(
                    &need("session_id")?,
                    &need("path")?,
                    &need("text")?,
                    flag("bom"),
                    eol,
                    &s("encoding").unwrap_or_else(|| "UTF-8".into()),
                    base,
                    limit,
                    flag("force"),
                    flag("allow_foreign"),
                    flag("in_place"),
                )
            }
            _ => Err(ApiError::new(
                "unknown_method",
                format!("Неизвестный метод: {method}"),
            )),
        }
    }
    /// Transfer queue API (`transfer_*`, `stage_clear`).
    fn transfer_call(&self, method: &str, p: &Value) -> Result<Value, ApiError> {
        use crate::sftp::xfer::Action;
        use crate::transfers::{Item, JobParams, Kind};
        let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_owned);
        let need =
            |k: &str| s(k).ok_or_else(|| ApiError::new("invalid", format!("Нет параметра {k}")));
        let kind = |v: &str| match v {
            "upload" => Ok(Kind::Upload),
            "download" => Ok(Kind::Download),
            _ => Err(ApiError::new("invalid", "Неизвестный вид передачи.")),
        };
        match method {
            "transfer_plan" => {
                let paths: Vec<String> = p
                    .get("paths")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                self.hub
                    .transfer_plan(&need("session_id")?, &need("kind")?, &paths, &need("dest")?)
            }
            "transfer_start" => {
                let st = self.settings_snapshot();
                let kind = kind(&need("kind")?)?;
                let items = p
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or_else(|| ApiError::new("invalid", "Нет параметра items"))?
                    .iter()
                    .map(|i| {
                        let path = i
                            .get("path")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let action = match i.get("action").and_then(Value::as_str).unwrap_or("skip")
                        {
                            "replace" => Action::Replace,
                            "rename" => Action::Rename,
                            _ => Action::Skip,
                        };
                        Item { path, action }
                    })
                    .collect::<Vec<_>>();
                if items.is_empty() || items.iter().any(|i| i.path.is_empty()) {
                    return Err(ApiError::new("invalid", "Нечего передавать."));
                }
                let stage = p.get("stage").and_then(Value::as_bool).unwrap_or(false);
                let (dest, max_bytes) = if stage {
                    let mut b = [0u8; 6];
                    getrandom::fill(&mut b).expect("rng");
                    let name: String = b.iter().map(|x| format!("{x:02x}")).collect();
                    let d = self.dirs.staging().join(name);
                    (
                        d.to_string_lossy().into_owned(),
                        Some(u64::from(st.drag_out_limit_mb) * 1024 * 1024),
                    )
                } else {
                    (need("dest")?, None)
                };
                let params = JobParams {
                    session_id: need("session_id")?,
                    kind,
                    items,
                    dest: dest.clone(),
                    keep_mtime: st.keep_mtime,
                    max_bytes,
                    risk: (
                        p.get("allow_foreign")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        p.get("in_place").and_then(Value::as_bool).unwrap_or(false),
                    ),
                };
                let sid = params.session_id.clone();
                let id = self.hub.transfer_start(&sid, params)?;
                Ok(json!({ "id": id, "dest": dest }))
            }
            "transfer_list" => Ok(self.hub.transfers.list()),
            "transfer_cancel" => {
                self.hub.transfers.cancel(&need("id")?);
                Ok(json!({}))
            }
            "transfer_retry" => {
                let risk = match (
                    p.get("allow_foreign").and_then(Value::as_bool),
                    p.get("in_place").and_then(Value::as_bool),
                ) {
                    (None, None) => None,
                    (a, b) => Some((a.unwrap_or(false), b.unwrap_or(false))),
                };
                let mut params = self
                    .hub
                    .transfers
                    .retry_params(&need("id")?, risk)
                    .ok_or_else(|| ApiError::new("invalid", "Эту передачу нельзя повторить."))?;
                // after a reconnect the transfer must go through the live session
                if let Some(s) = p.get("session_id").and_then(Value::as_str) {
                    params.session_id = s.to_string();
                }
                let sid = params.session_id.clone();
                let dest = params.dest.clone();
                let id = self.hub.transfer_start(&sid, params)?;
                Ok(json!({ "id": id, "dest": dest }))
            }
            "transfer_clear" => {
                self.hub.transfers.clear_finished();
                Ok(json!({}))
            }
            "stage_clear" => {
                // only ever inside <data>/staging
                let path = std::path::PathBuf::from(need("path")?);
                let root = self.dirs.staging();
                if path.starts_with(&root)
                    && path != root
                    && !path.components().any(|c| c.as_os_str() == "..")
                {
                    let _ = std::fs::remove_dir_all(&path);
                }
                Ok(json!({}))
            }
            _ => Err(ApiError::new(
                "unknown_method",
                format!("Неизвестный метод: {method}"),
            )),
        }
    }
    /// `connect`: either `server_id` (saved server) or `host`/`port`/`username` (quick connect).
    /// Optional `password` overrides the saved one (and `save_password` stores it after a successful login);
    /// `test_only` closes the session right after a successful login ("check connection").
    fn connect(&self, params: &Value) -> Result<Value, ApiError> {
        let s = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_owned);
        let flag = |k: &str| params.get(k).and_then(Value::as_bool).unwrap_or(false);
        let attempt =
            s("attempt").ok_or_else(|| ApiError::new("invalid", "Нет параметра attempt"))?;
        let server_id = s("server_id");
        let (name, host, port, user, saved, extra_args, saved_key, saved_key_pp) = match &server_id
        {
            Some(id) => self
                .with_vault(|v| {
                    Ok(v.data.servers.iter().find(|x| &x.id == id).map(|x| {
                        (
                            x.name.clone(),
                            x.host.clone(),
                            x.port,
                            x.username.clone(),
                            x.password.clone(),
                            x.ssh_extra_args.clone().unwrap_or_default(),
                            x.key_path.clone(),
                            x.key_passphrase.clone(),
                        )
                    }))
                })?
                .ok_or_else(|| ApiError::new("not_found", "Сервер не найден."))?,
            None => {
                let host = s("host").unwrap_or_default();
                let user = s("username").unwrap_or_default();
                let port = params.get("port").and_then(Value::as_u64).unwrap_or(22) as u16;
                (
                    format!("{user}@{host}"),
                    host,
                    port,
                    user,
                    String::new(),
                    String::new(),
                    None,
                    String::new(),
                )
            }
        };
        // a key given with the request (quick connect, or testing a key before saving it) wins over the saved one
        let key_path = s("key_path").filter(|k| !k.trim().is_empty()).or(saved_key);
        let key_passphrase = s("key_passphrase").or(if saved_key_pp.is_empty() {
            None
        } else {
            Some(saved_key_pp)
        });
        validate_target(&host, &user, port)?;
        let override_pw = s("password");
        let password = override_pw
            .clone()
            .or(if saved.is_empty() { None } else { Some(saved) });
        let ok = self.hub.connect(ConnectArgs {
            attempt,
            server_id: server_id.clone(),
            name: name.clone(),
            host: host.clone(),
            port,
            username: user.clone(),
            password,
            extra_args,
            key_path,
            key_passphrase,
        })?;
        let (session_id, home) = (ok.session_id.clone(), ok.home.clone());
        if flag("save_password") {
            if let (Some(id), Some(pw)) = (&server_id, &override_pw) {
                // The new password worked: remember it (best effort; connection stays usable on failure).
                let _ = self.with_vault(|v| {
                    if let Some(x) = v.data.servers.iter_mut().find(|x| &x.id == id) {
                        x.password = pw.clone();
                        x.updated = now();
                    }
                    v.save()
                });
            }
        }
        if flag("test_only") {
            self.hub.disconnect(&session_id);
        }
        Ok(json!({
            "session_id": session_id, "home": home, "sftp_error": ok.sftp_error,
            "name": name, "host": host, "port": port, "username": user,
        }))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<Vault>>, ApiError> {
        self.vault
            .lock()
            .map_err(|_| ApiError::new("internal", "Внутренняя ошибка (mutex)."))
    }

    fn with_vault<T>(
        &self,
        f: impl FnOnce(&mut Vault) -> Result<T, VaultError>,
    ) -> Result<T, ApiError> {
        let mut g = self.lock()?;
        let v = g
            .as_mut()
            .ok_or_else(|| ApiError::new("locked", "Хранилище заблокировано."))?;
        f(v).map_err(ApiError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths;

    fn svc(name: &str) -> (Service, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("d256shell-svc-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = paths::prepare(root.clone()).unwrap();
        (
            Service::with_kdf(
                dirs,
                KdfParams {
                    m_cost_kib: 64,
                    t_cost: 1,
                    p_cost: 1,
                },
            ),
            root,
        )
    }

    fn state(s: &Service) -> String {
        s.call("status", json!({})).unwrap()["state"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn srv(name: &str) -> Value {
        json!({"name": name, "host": "example.org", "port": 22, "username": "root", "password": "pw1"})
    }

    #[test]
    fn full_flow() {
        let (s, root) = svc("flow");
        assert_eq!(state(&s), "no_vault");
        assert!(s.call("create_vault", json!({"master": "short"})).is_err());
        s.call("create_vault", json!({"master": "long-enough-pw"}))
            .unwrap();
        assert_eq!(state(&s), "unlocked");
        let id = s.call("save_server", srv("one")).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let list = s.call("list_servers", json!({})).unwrap();
        assert_eq!(list[0]["name"], "one");
        assert!(
            list[0].get("password").is_none(),
            "password must not be listed"
        );
        assert_eq!(
            s.call("get_password", json!({"id": id})).unwrap(),
            json!("pw1")
        );

        // edit keeps the password when none is supplied
        let mut edit = srv("one-renamed");
        edit["id"] = json!(id);
        edit.as_object_mut().unwrap().remove("password");
        s.call("save_server", edit).unwrap();
        assert_eq!(
            s.call("get_password", json!({"id": id})).unwrap(),
            json!("pw1")
        );

        s.call("duplicate_server", json!({"id": id})).unwrap();
        assert_eq!(
            s.call("list_servers", json!({}))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );

        s.call("lock", json!({})).unwrap();
        assert_eq!(state(&s), "locked");
        assert_eq!(
            s.call("list_servers", json!({})).unwrap_err().code,
            "locked"
        );
        assert_eq!(
            s.call("unlock", json!({"master": "wrong-password"}))
                .unwrap_err()
                .code,
            "auth_failed"
        );
        s.call("unlock", json!({"master": "long-enough-pw"}))
            .unwrap();
        s.call("delete_server", json!({"id": id})).unwrap();
        assert_eq!(
            s.call("list_servers", json!({}))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );

        assert_eq!(
            s.call(
                "change_master",
                json!({"old": "bad-old-pass", "new": "new-long-pw1"})
            )
            .unwrap_err()
            .code,
            "auth_failed"
        );
        s.call(
            "change_master",
            json!({"old": "long-enough-pw", "new": "new-long-pw1"}),
        )
        .unwrap();
        s.call("lock", json!({})).unwrap();
        s.call("unlock", json!({"master": "new-long-pw1"})).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_option_injection_in_host_and_user() {
        let (s, root) = svc("inj");
        s.call("create_vault", json!({"master": "long-enough-pw"}))
            .unwrap();
        for (host, user) in [
            ("-oProxyCommand=x", "u"),
            ("a b", "u"),
            ("h", "-l"),
            ("h", "u v"),
            ("u@h", "u"),
            ("", "u"),
        ] {
            let mut v = srv("x");
            v["host"] = json!(host);
            v["username"] = json!(user);
            assert_eq!(
                s.call("save_server", v).unwrap_err().code,
                "invalid",
                "{host:?}/{user:?}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_method_and_locked_access() {
        let (s, root) = svc("unk");
        assert_eq!(
            s.call("nope", json!({})).unwrap_err().code,
            "unknown_method"
        );
        assert_eq!(s.call("save_server", srv("a")).unwrap_err().code, "locked");
        std::fs::remove_dir_all(root).unwrap();
    }
}
