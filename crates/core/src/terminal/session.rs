//! A PTY session running one process (system `ssh`). Output is queued with a byte limit so a runaway
//! `cat` back-pressures the PTY instead of exhausting memory; the UI drains it with [`TermSession::read`].
use super::expect::Expect;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Maximum queued, not yet delivered output.
const MAX_QUEUED: usize = 4 * 1024 * 1024;
/// Output delivered per `read` call.
const MAX_CHUNK: usize = 256 * 1024;
/// After the process exited, how long to wait for the PTY reader to drain before reporting the exit.
const EXIT_GRACE: Duration = Duration::from_millis(400);

pub struct TermOptions {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
    /// Password for the expect fallback (typed once, only when it is safe).
    pub expect_password: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadResult {
    Data(Vec<u8>),
    /// Nothing arrived within the timeout.
    Timeout,
    /// The process ended and all output was delivered.
    Exited(i32),
}

#[derive(Default)]
struct Queue {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    reader_done: bool,
    exit: Option<(i32, Instant)>,
    closed: bool,
}

struct Shared {
    q: Mutex<Queue>,
    data_cv: Condvar,
    space_cv: Condvar,
}

pub struct TermSession {
    shared: Arc<Shared>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    expect: Arc<Mutex<Option<Expect>>>,
    pid: Option<u32>,
}

impl TermSession {
    pub fn spawn(o: TermOptions) -> Result<Arc<TermSession>, String> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: o.rows.max(1),
                cols: o.cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("could not create the pseudo-terminal: {e}"))?;
        let mut cmd = CommandBuilder::new(&o.program);
        for a in &o.args {
            cmd.arg(a);
        }
        cmd.env("TERM", "xterm-256color");
        for (k, v) in &o.env {
            cmd.env(k, v);
        }
        let mut child: Box<dyn Child + Send + Sync> = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("could not start {}: {e}", o.program.display()))?;
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("PTY: {e}"))?;
        let writer = pair.master.take_writer().map_err(|e| format!("PTY: {e}"))?;

        let shared = Arc::new(Shared {
            q: Mutex::new(Queue::default()),
            data_cv: Condvar::new(),
            space_cv: Condvar::new(),
        });
        let expect = Arc::new(Mutex::new(
            o.expect_password
                .as_ref()
                .map(|_| Expect::new(Instant::now())),
        ));

        let sess = Arc::new(TermSession {
            shared: shared.clone(),
            writer: Mutex::new(writer),
            master: Mutex::new(Some(pair.master)),
            killer: Mutex::new(killer),
            expect: expect.clone(),
            pid,
        });

        // Reader: PTY -> queue (blocks while the queue is full: back-pressure).
        {
            let (shared, expect) = (shared.clone(), expect.clone());
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if let Some(e) = expect.lock().expect("lock").as_mut() {
                        e.on_output(&buf[..n], Instant::now());
                    }
                    let mut q = shared.q.lock().expect("lock");
                    while q.bytes > MAX_QUEUED && !q.closed {
                        q = shared.space_cv.wait(q).expect("wait");
                    }
                    if q.closed {
                        break;
                    }
                    q.bytes += n;
                    q.chunks.push_back(buf[..n].to_vec());
                    shared.data_cv.notify_all();
                }
                let mut q = shared.q.lock().expect("lock");
                q.reader_done = true;
                shared.data_cv.notify_all();
            });
        }
        // Waiter: process exit code.
        {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let code = match child.wait() {
                    Ok(st) => st.exit_code() as i32,
                    Err(_) => -1,
                };
                let mut q = shared.q.lock().expect("lock");
                q.exit = Some((code, Instant::now()));
                shared.data_cv.notify_all();
            });
        }
        // Expect watcher (only for the fallback mode): types the password once when it is safe.
        if let Some(pw) = o.expect_password {
            let weak = Arc::downgrade(&sess);
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(40));
                let Some(s) = weak.upgrade() else { return };
                let mut g = s.expect.lock().expect("lock");
                let Some(e) = g.as_mut() else { return };
                let now = Instant::now();
                if e.is_finished(now) {
                    return;
                }
                if e.should_inject(now) {
                    e.mark_injected();
                    drop(g);
                    let _ = s.write_raw(format!("{pw}\r").as_bytes());
                    return;
                }
            });
        }
        Ok(sess)
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn write_raw(&self, data: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().expect("lock");
        w.write_all(data)?;
        w.flush()
    }

    /// Input typed by the user, forwarded unchanged. Disables the expect fallback for good.
    pub fn write(&self, data: &[u8]) -> std::io::Result<()> {
        if let Some(e) = self.expect.lock().expect("lock").as_mut() {
            e.on_user_input();
        }
        self.write_raw(data)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        match self.master.lock().expect("lock").as_ref() {
            Some(m) => m
                .resize(PtySize {
                    rows: rows.max(1),
                    cols: cols.max(1),
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| e.to_string()),
            None => Err("session closed".into()),
        }
    }

    /// Wait up to `timeout` for output or process exit.
    pub fn read(&self, timeout: Duration) -> ReadResult {
        let deadline = Instant::now() + timeout;
        let mut q = self.shared.q.lock().expect("lock");
        loop {
            if !q.chunks.is_empty() {
                let mut out = Vec::new();
                while let Some(c) = q.chunks.front() {
                    if !out.is_empty() && out.len() + c.len() > MAX_CHUNK {
                        break;
                    }
                    let c = q.chunks.pop_front().expect("front");
                    q.bytes -= c.len();
                    out.extend_from_slice(&c);
                }
                self.shared.space_cv.notify_all();
                return ReadResult::Data(out);
            }
            if let Some((code, at)) = q.exit {
                if q.reader_done || at.elapsed() >= EXIT_GRACE {
                    return ReadResult::Exited(code);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return ReadResult::Timeout;
            }
            let mut wait = deadline - now;
            if let Some((_, at)) = q.exit {
                wait = wait.min(
                    EXIT_GRACE
                        .saturating_sub(at.elapsed())
                        .max(Duration::from_millis(5)),
                );
            }
            q = self.shared.data_cv.wait_timeout(q, wait).expect("wait").0;
        }
    }

    /// Kill the process and close the PTY (ConPTY closing also ends the console process tree).
    pub fn close(&self) {
        let _ = self.killer.lock().expect("lock").kill();
        self.master.lock().expect("lock").take();
        let mut q = self.shared.q.lock().expect("lock");
        q.closed = true;
        self.shared.space_cv.notify_all();
        self.shared.data_cv.notify_all();
    }
}

impl Drop for TermSession {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Arc<TermSession> {
        TermSession::spawn(TermOptions {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            env: vec![],
            cols: 80,
            rows: 24,
            expect_password: None,
        })
        .unwrap()
    }

    fn collect(s: &TermSession, max: Duration) -> (Vec<u8>, Option<i32>) {
        let t = Instant::now();
        let mut out = Vec::new();
        while t.elapsed() < max {
            match s.read(Duration::from_millis(200)) {
                ReadResult::Data(d) => out.extend(d),
                ReadResult::Timeout => {}
                ReadResult::Exited(c) => return (out, Some(c)),
            }
        }
        (out, None)
    }

    #[test]
    fn output_and_exit_code() {
        let s = sh("echo hello; exit 3");
        let (out, code) = collect(&s, Duration::from_secs(5));
        assert!(String::from_utf8_lossy(&out).contains("hello"));
        assert_eq!(code, Some(3));
    }

    #[test]
    fn input_is_forwarded_unchanged() {
        let s = sh("read x; echo got:$x");
        s.write(b"abc\r").unwrap();
        let (out, code) = collect(&s, Duration::from_secs(5));
        assert!(
            String::from_utf8_lossy(&out).contains("got:abc"),
            "{:?}",
            String::from_utf8_lossy(&out)
        );
        assert_eq!(code, Some(0));
    }

    #[test]
    fn resize_reaches_the_process() {
        let s = sh("sleep 0.5; stty size");
        s.resize(100, 30).unwrap();
        let (out, _) = collect(&s, Duration::from_secs(5));
        assert!(
            String::from_utf8_lossy(&out).contains("30 100"),
            "{:?}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn close_kills_the_process() {
        let s = sh("sleep 60");
        let pid = s.pid().unwrap() as i32;
        std::thread::sleep(Duration::from_millis(200));
        assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
        s.close();
        for _ in 0..50 {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("process {pid} survived close()");
    }

    #[test]
    fn utf8_passes_through_untouched() {
        let s = sh("printf 'ключ-✓-🙂\\n'");
        let (out, _) = collect(&s, Duration::from_secs(5));
        assert!(String::from_utf8_lossy(&out).contains("ключ-✓-🙂"));
    }

    #[test]
    fn large_output_is_back_pressured_and_complete() {
        // 20 MB through a 4 MB queue: nothing may be lost
        let s = sh("head -c 20000000 /dev/zero | tr '\\0' 'a'");
        let t = Instant::now();
        let mut total = 0usize;
        loop {
            match s.read(Duration::from_millis(500)) {
                ReadResult::Data(d) => total += d.iter().filter(|b| **b == b'a').count(),
                ReadResult::Exited(_) => break,
                ReadResult::Timeout => {}
            }
            assert!(t.elapsed() < Duration::from_secs(60), "too slow");
        }
        assert_eq!(total, 20_000_000);
    }

    #[test]
    fn expect_fallback_types_password_only_once_and_only_untouched() {
        // A tiny "server" that asks twice; the fallback must answer the first prompt only.
        let s = TermSession::spawn(TermOptions {
            program: "/bin/sh".into(),
            // echo off (like ssh does for passwords); the second prompt waits 1 s for any input
            args: vec!["-c".into(), "stty -echo; printf \"user@h's password: \"; read -r a; echo \"first:$a\"; printf \"user@h's password: \"; read -t 1 b; echo \"second:[$b]\"".into()],
            env: vec![],
            cols: 80,
            rows: 24,
            expect_password: Some("s3cret".into()),
        })
        .unwrap();
        let (out, _) = collect(&s, Duration::from_secs(6));
        let text = String::from_utf8_lossy(&out).to_string();
        assert!(text.contains("first:s3cret"), "{text:?}");
        assert!(
            text.contains("second:[]"),
            "typed again at the second prompt: {text:?}"
        );
        assert_eq!(
            text.matches("s3cret").count(),
            1,
            "typed more than once: {text:?}"
        );
    }

    #[test]
    fn expect_fallback_is_disabled_after_user_input() {
        let s = TermSession::spawn(TermOptions {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "sleep 0.3; printf \"Password: \"; sleep 1.5; echo done".into(),
            ],
            env: vec![],
            cols: 80,
            rows: 24,
            expect_password: Some("s3cret".into()),
        })
        .unwrap();
        s.write(b" ").unwrap(); // the user typed something before the prompt
        let (out, _) = collect(&s, Duration::from_secs(6));
        assert!(!String::from_utf8_lossy(&out).contains("s3cret"));
    }
}
