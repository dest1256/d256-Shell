//! SSH_ASKPASS support. The same executable acts as the askpass helper when `D256SHELL_ASKPASS=1`:
//! it sends `token\nprompt\n` to the IPC server of the main process (loopback only) and prints the reply.
//!
//! Wire format (one connection per prompt):
//!   client -> `<token>\n<prompt with newlines replaced by spaces>\n`
//!   server -> `OK <base64(password)>\n`  or  `NO\n`
//! The token is 32 random bytes (hex), unique per terminal session, compared in constant time, and
//! invalid as soon as the server is stopped.
use base64ct::{Base64, Encoding};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const ENV_FLAG: &str = "D256SHELL_ASKPASS";
pub const ENV_ADDR: &str = "D256SHELL_ASKPASS_ADDR";
pub const ENV_TOKEN: &str = "D256SHELL_ASKPASS_TOKEN";

/// How long the helper (and a pending prompt) may wait for the user.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(300);

/// Decides what to answer: `Some(password)` or `None` (refuse). May block (e.g. waiting for the user).
pub type Handler = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Context for a question that must go to the user.
pub struct AskInfo<'a> {
    pub prompt: &'a str,
    /// An ordinary account password request.
    pub is_password: bool,
    /// The saved password was already supplied in this session, so this request means it was wrong.
    pub saved_password_rejected: bool,
}

/// "Saved password exactly once": the first ordinary password prompt gets the saved password,
/// everything else (second password request after a wrong one, 2FA codes, passphrases, yes/no) is
/// handed to `ask`. Never retries a rejected password on its own (fail2ban / account lockout).
pub struct SavedPasswordPolicy {
    saved: std::sync::Mutex<Option<String>>,
    used: AtomicBool,
    /// Saved passphrase of the private key (answers "Enter passphrase for key ..." once).
    passphrase: std::sync::Mutex<Option<String>>,
    passphrase_used: AtomicBool,
}

impl SavedPasswordPolicy {
    pub fn new(saved: Option<String>) -> Self {
        Self {
            saved: std::sync::Mutex::new(saved.filter(|p| !p.is_empty())),
            used: AtomicBool::new(false),
            passphrase: std::sync::Mutex::new(None),
            passphrase_used: AtomicBool::new(false),
        }
    }

    pub fn with_passphrase(self, passphrase: Option<String>) -> Self {
        *self.passphrase.lock().expect("lock") = passphrase.filter(|p| !p.is_empty());
        self
    }

    pub fn decide(
        &self,
        prompt: &str,
        ask: &dyn Fn(AskInfo<'_>) -> Option<String>,
    ) -> Option<String> {
        // key passphrase: the saved one once, then the user (masked); it is never confused with the account password
        if prompt.to_lowercase().contains("passphrase") {
            if let Some(pp) = self.passphrase.lock().expect("lock").take() {
                self.passphrase_used.store(true, Ordering::SeqCst);
                return Some(pp);
            }
            return ask(AskInfo {
                prompt,
                is_password: true,
                saved_password_rejected: self.passphrase_used.load(Ordering::SeqCst),
            });
        }
        let is_password = crate::prompt::classify(prompt) == crate::prompt::PromptKind::Password;
        if is_password {
            if let Some(pw) = self.saved.lock().expect("lock").take() {
                self.used.store(true, Ordering::SeqCst);
                return Some(pw);
            }
        }
        ask(AskInfo {
            prompt,
            is_password,
            saved_password_rejected: is_password && self.used.load(Ordering::SeqCst),
        })
    }
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

pub fn new_token() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("rng");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Called when a valid request is refused (user cancelled / timeout), BEFORE the helper is told.
/// OpenSSH goes on to the next `NumberOfPasswordPrompts` attempt after a failed askpass and would send an
/// EMPTY password to the server (counted by fail2ban), so the owner must kill the ssh process here.
pub type OnRefuse = Arc<dyn Fn() + Send + Sync>;

pub struct AskpassServer {
    pub addr: SocketAddr,
    pub token: String,
    stop: Arc<AtomicBool>,
}

impl AskpassServer {
    pub fn start(handler: Handler, on_refuse: Option<OnRefuse>) -> std::io::Result<AskpassServer> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let token = new_token();
        let stop = Arc::new(AtomicBool::new(false));
        let (t, s) = (token.clone(), stop.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((conn, peer)) => {
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let (h, t, s, r) =
                            (handler.clone(), t.clone(), s.clone(), on_refuse.clone());
                        std::thread::spawn(move || serve_one(conn, &t, &s, &h, r.as_ref()));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(AskpassServer { addr, token, stop })
    }

    /// Invalidate the token and stop listening.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for AskpassServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve_one(
    conn: TcpStream,
    token: &str,
    stop: &AtomicBool,
    handler: &Handler,
    on_refuse: Option<&OnRefuse>,
) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(clone) = conn.try_clone() else { return };
    let mut r = BufReader::new(clone).take(16 * 1024);
    let (mut tok, mut prompt) = (String::new(), String::new());
    if r.read_line(&mut tok).is_err() || r.read_line(&mut prompt).is_err() {
        return;
    }
    let mut w = conn;
    let ok = constant_time_eq(tok.trim_end().as_bytes(), token.as_bytes())
        && !stop.load(Ordering::SeqCst);
    if !ok {
        let _ = w.write_all(b"NO\n");
        return;
    }
    // The handler may take minutes (user dialog); the helper waits with REPLY_TIMEOUT.
    let reply = match handler(prompt.trim_end()) {
        Some(pw) if !stop.load(Ordering::SeqCst) => {
            format!("OK {}\n", Base64::encode_string(pw.as_bytes()))
        }
        _ => {
            if let Some(f) = on_refuse {
                f(); // kill ssh first, then tell the helper
            }
            "NO\n".to_string()
        }
    };
    let _ = w.write_all(reply.as_bytes());
}

/// Client side: returns the password, or `None` when refused or on any error.
pub fn request(addr: &str, token: &str, prompt: &str) -> Option<String> {
    let sa: SocketAddr = addr.parse().ok()?;
    if !sa.ip().is_loopback() {
        return None; // never talk to anything but the local main process
    }
    let mut s = TcpStream::connect_timeout(&sa, Duration::from_secs(5)).ok()?;
    s.set_read_timeout(Some(REPLY_TIMEOUT)).ok()?;
    let one_line = prompt.replace(['\r', '\n'], " ");
    s.write_all(format!("{token}\n{one_line}\n").as_bytes())
        .ok()?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).ok()?;
    let b64 = line.trim_end().strip_prefix("OK ")?;
    String::from_utf8(Base64::decode_vec(b64).ok()?).ok()
}

/// If this process was started as the askpass helper, run it and exit; otherwise return immediately.
/// Must be the very first thing in `main()` (before any GUI/runtime initialisation).
pub fn run_helper_if_requested() {
    if std::env::var(ENV_FLAG).as_deref() != Ok("1") {
        return;
    }
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let addr = std::env::var(ENV_ADDR).unwrap_or_default();
    let token = std::env::var(ENV_TOKEN).unwrap_or_default();
    let code = match request(&addr, &token, &prompt) {
        Some(pw) => {
            let mut out = std::io::stdout();
            if out
                .write_all(pw.as_bytes())
                .and_then(|_| out.write_all(b"\n"))
                .and_then(|_| out.flush())
                .is_ok()
            {
                0
            } else {
                1
            }
        }
        None => 1,
    };
    std::process::exit(code);
}

/// Windows: OpenSSH starts the askpass program through a command line that may not survive spaces in
/// the path, so use the 8.3 short path there. Elsewhere (and on failure) the path is returned unchanged.
pub fn askpass_program_path(exe: &std::path::Path) -> String {
    #[cfg(windows)]
    {
        if let Some(s) = short_path(exe) {
            return s;
        }
    }
    exe.to_string_lossy().into_owned()
}

#[cfg(windows)]
fn short_path(p: &std::path::Path) -> Option<String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated and `buf` is a valid writable buffer of the stated length.
    let n =
        unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    Some(
        std::ffi::OsString::from_wide(&buf[..n])
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn server(f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> AskpassServer {
        AskpassServer::start(Arc::new(f), None).unwrap()
    }

    #[test]
    fn policy_supplies_the_saved_password_once_and_asks_the_user_otherwise() {
        let p = SavedPasswordPolicy::new(Some("saved".into()));
        let asked = std::cell::RefCell::new(Vec::new());
        let ask = |i: AskInfo<'_>| {
            asked.borrow_mut().push((
                i.prompt.to_string(),
                i.is_password,
                i.saved_password_rejected,
            ));
            Some("typed".to_string())
        };
        // 2FA first: not a password prompt, the saved password stays unused
        assert_eq!(
            p.decide("Verification code:", &ask).as_deref(),
            Some("typed")
        );
        assert_eq!(
            p.decide("user@h's password:", &ask).as_deref(),
            Some("saved")
        );
        // the server asks again => the saved one was wrong: user decides, flagged as rejected
        assert_eq!(
            p.decide("user@h's password:", &ask).as_deref(),
            Some("typed")
        );
        assert_eq!(
            p.decide("user@h's password:", &ask).as_deref(),
            Some("typed")
        );
        let a = asked.borrow();
        assert_eq!(a.len(), 3);
        assert_eq!(a[0], ("Verification code:".to_string(), false, false));
        assert_eq!(a[1], ("user@h's password:".to_string(), true, true));
    }

    #[test]
    fn policy_without_saved_password_always_asks() {
        let p = SavedPasswordPolicy::new(None);
        let ask = |i: AskInfo<'_>| {
            (i.is_password && !i.saved_password_rejected).then(|| "typed".to_string())
        };
        assert_eq!(p.decide("Password:", &ask).as_deref(), Some("typed"));
        let p = SavedPasswordPolicy::new(Some(String::new()));
        assert_eq!(p.decide("Password:", &ask).as_deref(), Some("typed"));
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn roundtrip_with_special_characters() {
        let s = server(|p| (p == "tester@h's password:").then(|| "pä$$ wörd\"'\\ key".to_string()));
        let pw = request(&s.addr.to_string(), &s.token, "tester@h's password:").unwrap();
        assert_eq!(pw, "pä$$ wörd\"'\\ key");
    }

    #[test]
    fn wrong_token_is_refused_and_handler_not_called() {
        let called = Arc::new(Mutex::new(0));
        let c = called.clone();
        let s = server(move |_| {
            *c.lock().unwrap() += 1;
            Some("secret".into())
        });
        assert_eq!(
            request(&s.addr.to_string(), "wrong-token", "Password:"),
            None
        );
        assert_eq!(
            request(&s.addr.to_string(), &s.token[..63], "Password:"),
            None
        );
        assert_eq!(*called.lock().unwrap(), 0);
    }

    #[test]
    fn stopped_server_invalidates_the_token() {
        let s = server(|_| Some("secret".into()));
        assert_eq!(
            request(&s.addr.to_string(), &s.token, "Password:").as_deref(),
            Some("secret")
        );
        s.stop();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(request(&s.addr.to_string(), &s.token, "Password:"), None);
    }

    #[test]
    fn refusal_and_multiline_prompts() {
        let s = server(|p| {
            assert!(!p.contains('\n'));
            None
        });
        assert_eq!(
            request(&s.addr.to_string(), &s.token, "line1\nline2:"),
            None
        );
    }

    #[test]
    fn client_never_talks_to_non_loopback_addresses() {
        assert_eq!(request("192.0.2.1:9", "t", "p"), None);
        assert_eq!(request("not an address", "t", "p"), None);
    }

    #[test]
    fn server_listens_on_loopback_only() {
        let s = server(|_| None);
        assert!(s.addr.ip().is_loopback());
    }
}
