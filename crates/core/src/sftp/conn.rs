//! Connection: TCP -> SSH handshake (host key check against `known_hosts`) -> authentication -> SFTP subsystem.
//!
//! Authentication policy (production safety): at most ONE automatic attempt with the saved password.
//! A wrong password is never retried automatically (fail2ban / account lockout); any further or unusual
//! prompt (2FA, second password request) goes to the user through [`Interaction::prompt`].
use super::model::{DirEntry, EntryKind};
use crate::known_hosts::{self, Verdict};
use crate::prompt::{classify, PromptKind};
use crate::remote_path;
use russh::client::{self, Handle, Handler, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PublicKey};
use russh::MethodKind;
use russh_sftp::client::SftpSession;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

#[derive(Debug, Clone)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    /// `SHA256:...` as printed by OpenSSH.
    pub fingerprint: String,
    pub key_b64: String,
    /// Key types already on file for this host (non-empty only for a changed key).
    pub known_types: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AuthPrompt {
    pub text: String,
    pub echo: bool,
    pub kind: PromptKind,
    /// A password was already sent automatically in this attempt, so this request means it was wrong.
    pub saved_password_rejected: bool,
}

/// Callbacks into the user interface.
pub struct Interaction {
    /// Unknown host: show the fingerprint, return `true` to trust and remember.
    pub confirm_host_key: Box<dyn Fn(HostKeyInfo) -> BoxFut<bool> + Send + Sync>,
    /// Any prompt that must not be answered with the saved password. `None` = user cancelled.
    pub prompt: Box<dyn Fn(AuthPrompt) -> BoxFut<Option<String>> + Send + Sync>,
}

pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Saved (or user-supplied) password for the single automatic attempt.
    pub password: Option<String>,
    pub known_hosts: PathBuf,
    pub connect_timeout: Duration,
    /// Private key login (tried first; the password policy below applies to what comes after it).
    pub key: Option<KeyAuth>,
}

/// A private key file and, when the user saved it, its passphrase.
#[derive(Clone)]
pub struct KeyAuth {
    pub path: String,
    pub passphrase: Option<String>,
}

impl std::fmt::Debug for KeyAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // never print the passphrase
        write!(f, "KeyAuth({})", self.path)
    }
}

#[derive(Debug)]
pub enum ConnectError {
    Timeout,
    Network(String),
    /// Presented key differs from the one on file. The connection is NOT made.
    HostKeyChanged(Box<HostKeyInfo>),
    HostKeyRevoked(Box<HostKeyInfo>),
    /// The user declined to trust an unknown host key.
    HostKeyRejected,
    /// Wrong credentials. `saved_password_used`: the automatic attempt was made (and must not be repeated).
    AuthFailed {
        saved_password_used: bool,
    },
    AuthCancelled,
    /// The key file cannot be used (missing, wrong format, wrong passphrase ...): a message for the user.
    KeyError(String),
    /// The server did not accept the private key.
    KeyRejected,
    NoAuthMethod(Vec<String>),
    Protocol(String),
    /// Login succeeded but the SFTP subsystem is not available (disabled or forbidden on the server).
    SftpUnavailable(String),
}

impl ConnectError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Network(_) => "network",
            Self::HostKeyChanged(_) => "host_key_changed",
            Self::HostKeyRevoked(_) => "host_key_revoked",
            Self::HostKeyRejected => "host_key_rejected",
            Self::AuthFailed { .. } => "auth_failed",
            Self::AuthCancelled => "auth_cancelled",
            Self::KeyError(_) => "key_error",
            Self::KeyRejected => "key_rejected",
            Self::NoAuthMethod(_) => "no_auth_method",
            Self::Protocol(_) => "protocol",
            Self::SftpUnavailable(_) => "sftp_unavailable",
        }
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "The server does not respond (timeout exceeded)."),
            Self::Network(e) => write!(f, "Could not connect: {e}"),
            Self::HostKeyChanged(i) => write!(
                f,
                "WARNING: the server key {} changed! A man-in-the-middle attack is possible. The connection was not made.",
                i.host
            ),
            Self::HostKeyRevoked(i) => write!(f, "Server key {} revoked (@revoked). The connection was not made.", i.host),
            Self::HostKeyRejected => write!(f, "The server key was not accepted. The connection was cancelled."),
            Self::AuthFailed { saved_password_used: true } => write!(f, "Wrong password (or login is not allowed for this user)."),
            Self::AuthFailed { .. } => write!(f, "Login failed: the credentials were rejected."),
            Self::AuthCancelled => write!(f, "Login cancelled."),
            Self::KeyError(m) => f.write_str(m),
            Self::KeyRejected => write!(f, "The server did not accept the private key. Check that the matching public key is added to ~/.ssh/authorized_keys of this user on the server."),
            Self::NoAuthMethod(m) => write!(f, "The server does not support password login (available: {}).", m.join(", ")),
            Self::Protocol(e) => write!(f, "SSH protocol error: {e}"),
            Self::SftpUnavailable(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ConnectError {}

enum HostKeyFailure {
    Changed(HostKeyInfo),
    Revoked(HostKeyInfo),
    Rejected,
    Io(String),
}

struct Client {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    interaction: Arc<Interaction>,
    failure: Arc<Mutex<Option<HostKeyFailure>>>,
    waiting_for_user: Arc<AtomicUsize>,
}

struct WaitGuard(Arc<AtomicUsize>);
impl WaitGuard {
    fn new(c: &Arc<AtomicUsize>) -> Self {
        c.fetch_add(1, Ordering::SeqCst);
        Self(c.clone())
    }
}
impl Drop for WaitGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn key_parts(key: &PublicKey) -> Option<(String, String)> {
    let line = key.to_openssh().ok()?;
    let mut it = line.split_whitespace();
    Some((it.next()?.to_string(), it.next()?.to_string()))
}

impl Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let set = |f: HostKeyFailure| *self.failure.lock().expect("lock") = Some(f);
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            set(HostKeyFailure::Io(
                "host certificates are not supported".into(),
            ));
            return Ok(false);
        };
        let Some((key_type, key_b64)) = key_parts(key) else {
            set(HostKeyFailure::Io("cannot encode host key".into()));
            return Ok(false);
        };
        let mut info = HostKeyInfo {
            host: self.host.clone(),
            port: self.port,
            key_type: key_type.clone(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            key_b64: key_b64.clone(),
            known_types: Vec::new(),
        };
        let verdict = match known_hosts::check_file(
            &self.known_hosts,
            &self.host,
            self.port,
            &key_type,
            &key_b64,
        ) {
            Ok(v) => v,
            Err(e) => {
                set(HostKeyFailure::Io(format!("known_hosts: {e}")));
                return Ok(false);
            }
        };
        match verdict {
            Verdict::Match => Ok(true),
            Verdict::Revoked => {
                set(HostKeyFailure::Revoked(info));
                Ok(false)
            }
            Verdict::Changed { known_types } => {
                info.known_types = known_types;
                set(HostKeyFailure::Changed(info));
                Ok(false)
            }
            Verdict::Unknown => {
                let _g = WaitGuard::new(&self.waiting_for_user);
                if !(self.interaction.confirm_host_key)(info).await {
                    set(HostKeyFailure::Rejected);
                    return Ok(false);
                }
                if let Err(e) = known_hosts::add(
                    &self.known_hosts,
                    &self.host,
                    self.port,
                    &key_type,
                    &key_b64,
                ) {
                    set(HostKeyFailure::Io(format!(
                        "could not write known_hosts: {e}"
                    )));
                    return Ok(false);
                }
                Ok(true)
            }
        }
    }
}

/// Order the host key algorithms so that those already on file for this host come first
/// (like OpenSSH does), otherwise a server offering several key types could be negotiated to a
/// different one than we know and be reported as "changed".
fn preferred_for(known_types: &[String]) -> russh::Preferred {
    use russh::keys::Algorithm;
    fn family(a: &Algorithm) -> String {
        match a {
            Algorithm::Ed25519 => "ssh-ed25519".into(),
            Algorithm::Rsa { .. } => "ssh-rsa".into(),
            Algorithm::Ecdsa { curve } => format!("ecdsa-sha2-{}", curve.as_str()),
            other => other.to_string(),
        }
    }
    let mut p = russh::Preferred::default();
    let mut keys: Vec<Algorithm> = p.key.iter().cloned().collect();
    keys.sort_by_key(|a| !known_types.contains(&family(a))); // stable: known first
    p.key = std::borrow::Cow::Owned(keys);
    p
}

pub struct SftpConnection {
    handle: Handle<Client>,
    pub sftp: SftpSession,
    /// Server-side canonical home directory (`realpath(".")`).
    pub home: String,
    /// Owner of the home directory: the best available guess of "the current user" over SFTP.
    pub home_uid: Option<u32>,
    /// Login name (for readable permission errors).
    pub user: String,
    /// Temporary files that could not be removed (lost link); retried by `cleanup_stale`.
    pub stale: Mutex<Vec<String>>,
}

impl SftpConnection {
    pub async fn connect(
        p: ConnectParams,
        interaction: Arc<Interaction>,
    ) -> Result<SftpConnection, ConnectError> {
        let tcp = tokio::time::timeout(
            p.connect_timeout,
            tokio::net::TcpStream::connect((p.host.as_str(), p.port)),
        )
        .await
        .map_err(|_| ConnectError::Timeout)?
        .map_err(|e| ConnectError::Network(e.to_string()))?;
        let _ = tcp.set_nodelay(true);

        let known = known_hosts::known_key_types(&p.known_hosts, &p.host, p.port);
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 4,
            preferred: preferred_for(&known),
            ..Default::default()
        });
        let failure = Arc::new(Mutex::new(None));
        let waiting = Arc::new(AtomicUsize::new(0));
        let handler = Client {
            host: p.host.clone(),
            port: p.port,
            known_hosts: p.known_hosts.clone(),
            interaction: interaction.clone(),
            failure: failure.clone(),
            waiting_for_user: waiting.clone(),
        };

        // Handshake with a watchdog that does not count time spent waiting for the user.
        let fut = client::connect_stream(config, tcp, handler);
        tokio::pin!(fut);
        let res = loop {
            tokio::select! {
                r = &mut fut => break r,
                _ = tokio::time::sleep(p.connect_timeout) => {
                    if waiting.load(Ordering::SeqCst) == 0 {
                        return Err(ConnectError::Timeout);
                    }
                }
            }
        };
        let mut handle = match res {
            Ok(h) => h,
            Err(e) => {
                return Err(match failure.lock().expect("lock").take() {
                    Some(HostKeyFailure::Changed(i)) => ConnectError::HostKeyChanged(Box::new(i)),
                    Some(HostKeyFailure::Revoked(i)) => ConnectError::HostKeyRevoked(Box::new(i)),
                    Some(HostKeyFailure::Rejected) => ConnectError::HostKeyRejected,
                    Some(HostKeyFailure::Io(m)) => ConnectError::Protocol(m),
                    None => ConnectError::Protocol(e.to_string()),
                })
            }
        };

        authenticate(&mut handle, &p, &interaction, &waiting).await?;

        let proto = |e: &dyn std::fmt::Display| ConnectError::Protocol(e.to_string());
        let mut ch = handle.channel_open_session().await.map_err(|e| proto(&e))?;
        // From here on the login worked: a missing SFTP subsystem is reported separately so that the
        // terminal (a plain ssh shell) can still be used.
        ch.request_subsystem(true, "sftp")
            .await
            .map_err(|e| sftp_unavailable(&e))?;
        // Wait for the server's verdict: otherwise a server without SFTP is only noticed after the
        // SFTP init timeout (10 s).
        loop {
            match tokio::time::timeout(Duration::from_secs(15), ch.wait()).await {
                Ok(Some(russh::ChannelMsg::Success)) => break,
                Ok(Some(russh::ChannelMsg::Failure)) => {
                    return Err(sftp_unavailable(
                        &"the server rejected the sftp subsystem request",
                    ))
                }
                Ok(Some(_)) => continue, // window adjust, banner data, ...
                Ok(None) => return Err(sftp_unavailable(&"the server closed the channel")),
                Err(_) => return Err(sftp_unavailable(&"no reply to the sftp subsystem request")),
            }
        }
        let sftp = SftpSession::new(ch.into_stream())
            .await
            .map_err(|e| sftp_unavailable(&e))?;
        let home = sftp
            .canonicalize(".")
            .await
            .map_err(|e| sftp_unavailable(&e))?;
        let home_uid = sftp.metadata(home.clone()).await.ok().and_then(|m| m.uid);
        Ok(SftpConnection {
            handle,
            sftp,
            home,
            home_uid,
            user: p.username.clone(),
            stale: Mutex::new(Vec::new()),
        })
    }

    /// Drop the link without consuming the connection (used when the user cancels everything, and in tests).
    pub async fn disconnect(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Server-side `realpath` (resolves symlinks and `..`).
    pub async fn canonicalize(&self, path: &str) -> Result<String, String> {
        self.sftp.canonicalize(path).await.map_err(describe)
    }

    /// List a directory. Entry types come from `lstat` data; for symlinks the target type is resolved with `stat`.
    pub async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        let rd = self.sftp.read_dir(path).await.map_err(describe)?;
        let mut out = Vec::new();
        for e in rd {
            let m = e.metadata();
            let ft = m.file_type();
            let kind = if ft.is_symlink() {
                EntryKind::Symlink
            } else if ft.is_dir() {
                EntryKind::Dir
            } else if ft.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            let name = e.file_name();
            let target_is_dir = if kind == EntryKind::Symlink {
                self.sftp
                    .metadata(remote_path::join(path, &name))
                    .await
                    .ok()
                    .map(|t| t.file_type().is_dir())
            } else {
                None
            };
            out.push(DirEntry {
                name_lossy: name.contains('\u{FFFD}'),
                name,
                kind,
                target_is_dir,
                size: m.size.unwrap_or(0),
                mtime: m.mtime,
                mode: m.permissions,
                uid: m.uid,
                gid: m.gid,
                owner: m.user.clone(),
                group: m.group.clone(),
            });
        }
        Ok(out)
    }

    pub async fn close(self) {
        let _ = self.sftp.close().await;
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
    }
}

fn sftp_unavailable(e: &dyn std::fmt::Display) -> ConnectError {
    ConnectError::SftpUnavailable(format!(
        "The server did not provide SFTP: the subsystem is disabled or forbidden for this user ({e}). The terminal works, the file panel is unavailable. SFTP is not available on OpenWrt (dropbear) by default: run “opkg update && opkg install openssh-sftp-server” in the terminal and reconnect."
    ))
}

/// Human readable SFTP error.
pub fn describe(e: russh_sftp::client::error::Error) -> String {
    use russh_sftp::client::error::Error;
    use russh_sftp::protocol::StatusCode;
    match &e {
        Error::Status(s) => match s.status_code {
            StatusCode::NoSuchFile => "No such file or folder.".to_string(),
            StatusCode::PermissionDenied => "No access rights.".to_string(),
            StatusCode::ConnectionLost | StatusCode::NoConnection => {
                "Connection to the server lost.".to_string()
            }
            _ => format!("Server error: {}", s.error_message),
        },
        other => format!("SFTP error: {other}"),
    }
}

async fn authenticate(
    handle: &mut Handle<Client>,
    p: &ConnectParams,
    interaction: &Arc<Interaction>,
    waiting: &Arc<AtomicUsize>,
) -> Result<(), ConnectError> {
    let proto = |e: russh::Error| ConnectError::Protocol(e.to_string());
    let user = p.username.clone();
    let none = handle
        .authenticate_none(user.clone())
        .await
        .map_err(proto)?;
    let mut methods = match none {
        russh::client::AuthResult::Success => return Ok(()),
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods,
    };

    // Private key first (when configured). The password policy below is unchanged: it applies only to what the
    // server still offers after the key (a key rejected for good, with no saved password, is reported as such).
    if let Some(k) = &p.key {
        if !methods.contains(&MethodKind::PublicKey) {
            return Err(ConnectError::KeyError(
                "The server does not accept key login (the publickey method is disabled).".into(),
            ));
        }
        let key = load_key(k, interaction, waiting).await?;
        let hash = handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten();
        let res = handle
            .authenticate_publickey(
                user.clone(),
                russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await
            .map_err(proto)?;
        match res {
            russh::client::AuthResult::Success => return Ok(()),
            russh::client::AuthResult::Failure {
                remaining_methods,
                partial_success,
            } => {
                // key + second factor ("partial success") or a saved password as fallback: go on, otherwise stop here
                if !(partial_success || p.password.is_some()) {
                    return Err(ConnectError::KeyRejected);
                }
                methods = remaining_methods;
            }
        }
    }
    let has = |k: MethodKind| methods.contains(&k);

    if has(MethodKind::Password) {
        let pw = match &p.password {
            Some(pw) => pw.clone(),
            None => {
                let _g = WaitGuard::new(waiting);
                (interaction.prompt)(AuthPrompt {
                    text: format!("Password for {user}@{}:", p.host),
                    echo: false,
                    kind: PromptKind::Password,
                    saved_password_rejected: false,
                })
                .await
                .ok_or(ConnectError::AuthCancelled)?
            }
        };
        // Exactly one attempt. No retry, no fallback to keyboard-interactive with the same password.
        return match handle
            .authenticate_password(user, pw)
            .await
            .map_err(proto)?
        {
            russh::client::AuthResult::Success => Ok(()),
            russh::client::AuthResult::Failure { .. } => Err(ConnectError::AuthFailed {
                saved_password_used: p.password.is_some(),
            }),
        };
    }

    if has(MethodKind::KeyboardInteractive) {
        let mut saved = p.password.clone();
        let mut used_saved = false;
        let mut resp = handle
            .authenticate_keyboard_interactive_start(user.clone(), None)
            .await
            .map_err(proto)?;
        for _round in 0..8 {
            match resp {
                KeyboardInteractiveAuthResponse::Success => return Ok(()),
                KeyboardInteractiveAuthResponse::Failure { .. } => {
                    return Err(ConnectError::AuthFailed {
                        saved_password_used: used_saved,
                    })
                }
                KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    let mut answers = Vec::with_capacity(prompts.len());
                    for pr in prompts {
                        let kind = classify(&pr.prompt);
                        if kind == PromptKind::Password && saved.is_some() {
                            answers.push(saved.take().expect("checked"));
                            used_saved = true;
                        } else {
                            let _g = WaitGuard::new(waiting);
                            let a = (interaction.prompt)(AuthPrompt {
                                text: pr.prompt.clone(),
                                echo: pr.echo,
                                kind,
                                saved_password_rejected: kind == PromptKind::Password && used_saved,
                            })
                            .await
                            .ok_or(ConnectError::AuthCancelled)?;
                            answers.push(a);
                        }
                    }
                    resp = handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(proto)?;
                }
            }
        }
        return Err(ConnectError::Protocol(
            "too many authentication requests".into(),
        ));
    }

    Err(ConnectError::NoAuthMethod(
        methods
            .iter()
            .map(|m| <&str>::from(m).to_string())
            .collect(),
    ))
}

/// Load the private key. An encrypted key needs its passphrase: the saved one first, then the user is asked
/// (up to three times: decrypting a file is local, no server login attempt is spent).
async fn load_key(
    k: &KeyAuth,
    interaction: &Arc<Interaction>,
    waiting: &Arc<AtomicUsize>,
) -> Result<russh::keys::PrivateKey, ConnectError> {
    use russh::keys::{load_secret_key, Error as KeyError};
    let path = crate::keyfile::check(&k.path).map_err(ConnectError::KeyError)?;
    let encrypted = |e: &KeyError| {
        matches!(e, KeyError::KeyIsEncrypted) || e.to_string().to_lowercase().contains("encrypt")
    };
    match load_secret_key(&path, None) {
        Ok(key) => return Ok(key),
        Err(e) if !encrypted(&e) => {
            return Err(ConnectError::KeyError(format!(
                "Could not read the key {}: {e}. OpenSSH and PEM keys are supported (RSA, ECDSA, Ed25519).",
                path.display()
            )))
        }
        Err(_) => {}
    }
    let mut candidate = k.passphrase.clone().filter(|p| !p.is_empty());
    for round in 0..4 {
        if let Some(pass) = candidate.take() {
            if let Ok(key) = load_secret_key(&path, Some(&pass)) {
                return Ok(key);
            }
        }
        if round == 3 {
            break;
        }
        let _g = WaitGuard::new(waiting);
        let ans = (interaction.prompt)(AuthPrompt {
            text: format!("Passphrase for the key {}:", path.display()),
            echo: false,
            kind: PromptKind::Passphrase,
            saved_password_rejected: round > 0
                || k.passphrase.as_deref().is_some_and(|p| !p.is_empty()),
        })
        .await
        .ok_or(ConnectError::AuthCancelled)?;
        candidate = Some(ans);
    }
    Err(ConnectError::KeyError(format!(
        "Wrong passphrase for the key {}.",
        path.display()
    )))
}
