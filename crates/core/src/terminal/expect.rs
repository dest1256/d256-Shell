//! Fallback for ssh without `SSH_ASKPASS_REQUIRE` (OpenSSH < 8.4): watch the PTY output and type the saved
//! password once, only when it is safe:
//!  1. at most 60 s since ssh started;
//!  2. no automatic input yet in this connection attempt;
//!  3. the user has not typed anything into this session;
//!  4. the output (ANSI stripped, last ~256 chars) ends with a password prompt that has been quiet for ~150 ms.
//!
//! Any doubt means: type nothing, the user enters the password themselves.
use std::time::{Duration, Instant};

const TAIL_BYTES: usize = 1024;
const TAIL_CHARS: usize = 256;

/// Remove ANSI escape sequences (CSI, OSC, two-byte escapes) and BEL.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            i += 1;
            match bytes.get(i) {
                Some(b'[') => {
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1; // final byte
                }
                Some(b']') => {
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                Some(b'(' | b')' | b'*' | b'+') => i += 2, // charset selection
                Some(_) => i += 1,
                None => {}
            }
        } else if b == 0x07 {
            i += 1;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const NOT_ACCOUNT_PASSWORD: [&str; 8] = [
    "passphrase",
    "new password",
    "retype",
    "verification",
    "code",
    "otp",
    "token",
    "новый пароль",
];

/// Does the (ANSI-stripped) text end with an account password prompt?
pub fn prompt_at_end(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let tail: String = chars[chars.len().saturating_sub(TAIL_CHARS)..]
        .iter()
        .collect();
    let last_line = tail.rsplit('\n').next().unwrap_or("");
    let last_line = last_line.rsplit('\r').next().unwrap_or("");
    let line = last_line.trim_end();
    if line.is_empty() || line.chars().count() > 160 || !line.ends_with(':') {
        return false;
    }
    let lower = line.to_lowercase();
    if !(lower.contains("password") || lower.contains("пароль")) {
        return false;
    }
    !NOT_ACCOUNT_PASSWORD.iter().any(|w| lower.contains(w)) || lower.ends_with("'s password:")
    // "user@code-host's password:" is still the account password
}

pub struct Expect {
    start: Instant,
    injected: bool,
    typed: bool,
    last_data: Option<Instant>,
    tail: Vec<u8>,
    window: Duration,
    settle: Duration,
}

impl Expect {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            injected: false,
            typed: false,
            last_data: None,
            tail: Vec::new(),
            window: Duration::from_secs(60),
            settle: Duration::from_millis(150),
        }
    }

    pub fn on_output(&mut self, data: &[u8], now: Instant) {
        self.tail.extend_from_slice(data);
        if self.tail.len() > TAIL_BYTES {
            let cut = self.tail.len() - TAIL_BYTES;
            self.tail.drain(..cut);
        }
        self.last_data = Some(now);
    }

    /// The user typed into the session: automatic input is off for good.
    pub fn on_user_input(&mut self) {
        self.typed = true;
    }

    pub fn mark_injected(&mut self) {
        self.injected = true;
    }

    /// Can the watcher stop (nothing will ever be typed automatically)?
    pub fn is_finished(&self, now: Instant) -> bool {
        self.injected || self.typed || now.duration_since(self.start) > self.window
    }

    pub fn should_inject(&self, now: Instant) -> bool {
        if self.is_finished(now) {
            return false;
        }
        let Some(last) = self.last_data else {
            return false;
        };
        now.duration_since(last) >= self.settle && prompt_at_end(&strip_ansi(&self.tail))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn strips_ansi() {
        assert_eq!(strip_ansi(b"\x1b[31mred\x1b[0m text"), "red text");
        assert_eq!(strip_ansi(b"\x1b[?25lhide\x1b[?25h"), "hide");
        assert_eq!(strip_ansi(b"\x1b]0;title\x07after"), "after");
        assert_eq!(strip_ansi(b"\x1b]0;title\x1b\\after"), "after");
        assert_eq!(strip_ansi(b"a\x1b(Bb\x07c"), "abc");
        assert_eq!(strip_ansi("Пароль:".as_bytes()), "Пароль:");
        assert_eq!(strip_ansi(b"trailing\x1b"), "trailing");
    }

    #[test]
    fn recognises_prompts() {
        for p in [
            "root@host's password: ",
            "Password: ",
            "password:",
            "Enter password for user: ",
            "banner line\r\nuser@sshd-password's password: ",
            "Пароль: ",
            "tester@code-host's password: ",
            "x\r\rPassword:",
        ] {
            assert!(prompt_at_end(p), "{p:?}");
        }
    }

    #[test]
    fn rejects_non_prompts() {
        for p in [
            "",
            "Last login: Mon\r\nuser@host:~$ ",
            "Enter passphrase for key '/k': ",
            "Verification code: ",
            "New password: ",
            "Retype new password: ",
            "the word password: appears mid line and more text after it",
            "password: ok\r\nuser@host:~$ ",
            "Password",
            "total 4\r\n-rw-r--r-- 1 u u 0 password.txt\r\n",
        ] {
            assert!(!prompt_at_end(p), "{p:?}");
        }
    }

    fn armed(prompt: &str) -> (Expect, Instant) {
        let t0 = Instant::now();
        let mut e = Expect::new(t0);
        e.on_output(prompt.as_bytes(), t0 + ms(100));
        (e, t0)
    }

    #[test]
    fn injects_only_after_the_prompt_settled() {
        let (e, t0) = armed("user@h's password: ");
        assert!(!e.should_inject(t0 + ms(150)), "only 50 ms of silence");
        assert!(e.should_inject(t0 + ms(260)));
    }

    #[test]
    fn more_output_after_the_prompt_cancels() {
        let t0 = Instant::now();
        let mut e = Expect::new(t0);
        e.on_output(b"Password: ", t0 + ms(100));
        e.on_output(b"\r\nuser@host:~$ ", t0 + ms(120));
        assert!(!e.should_inject(t0 + ms(1000)));
    }

    #[test]
    fn never_after_user_typed() {
        let (mut e, t0) = armed("Password: ");
        e.on_user_input();
        assert!(!e.should_inject(t0 + ms(1000)));
        assert!(e.is_finished(t0 + ms(1000)));
    }

    #[test]
    fn password_text_in_command_output_after_user_input_is_ignored() {
        // the user ran `cat file`, the file ends with "password:" and no newline
        let t0 = Instant::now();
        let mut e = Expect::new(t0);
        e.on_output(b"user@host:~$ ", t0 + ms(500));
        e.on_user_input();
        e.on_output(b"cat notes\r\nmy password: ", t0 + ms(900));
        assert!(!e.should_inject(t0 + ms(2000)));
    }

    #[test]
    fn never_after_60_seconds() {
        let (e, t0) = armed("Password: ");
        assert!(e.should_inject(t0 + ms(1000)));
        assert!(!e.should_inject(t0 + Duration::from_secs(61)));
    }

    #[test]
    fn only_once() {
        let (mut e, t0) = armed("Password: ");
        assert!(e.should_inject(t0 + ms(1000)));
        e.mark_injected();
        // the server asks again (wrong password): the user must decide
        e.on_output(
            b"\r\nPermission denied, please try again.\r\nuser@h's password: ",
            t0 + ms(2000),
        );
        assert!(!e.should_inject(t0 + ms(3000)));
    }

    #[test]
    fn works_through_ansi_decoration() {
        let (e, t0) = armed("\x1b[1m\x1b[33muser@h's password: \x1b[0m");
        assert!(e.should_inject(t0 + ms(1000)));
    }

    #[test]
    fn silence_without_any_output_does_nothing() {
        let e = Expect::new(Instant::now());
        assert!(!e.should_inject(Instant::now() + Duration::from_secs(5)));
    }

    #[test]
    fn tail_is_bounded() {
        let t0 = Instant::now();
        let mut e = Expect::new(t0);
        for _ in 0..1000 {
            e.on_output(&[b'x'; 100], t0);
        }
        assert!(e.tail.len() <= TAIL_BYTES);
    }
}
