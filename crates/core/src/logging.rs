//! Minimal file logger with size-based rotation. Never pass secrets to it.
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_BYTES: u64 = 1_000_000;
const KEEP: usize = 3;

pub struct Logger {
    path: PathBuf,
    inner: Mutex<(File, u64)>,
}

impl Logger {
    pub fn open(dir: &Path) -> std::io::Result<Logger> {
        let path = dir.join("d256shell.log");
        let (file, size) = Self::open_append(&path)?;
        Ok(Logger {
            path,
            inner: Mutex::new((file, size)),
        })
    }

    fn open_append(path: &Path) -> std::io::Result<(File, u64)> {
        let f = OpenOptions::new().create(true).append(true).open(path)?;
        let size = f.metadata()?.len();
        Ok((f, size))
    }

    fn rotated(&self, i: usize) -> PathBuf {
        let mut s = self.path.clone().into_os_string();
        s.push(format!(".{i}"));
        PathBuf::from(s)
    }

    fn rotate(&self) -> std::io::Result<(File, u64)> {
        for i in (1..KEEP).rev() {
            let from = self.rotated(i);
            if from.exists() {
                let _ = fs::rename(from, self.rotated(i + 1));
            }
        }
        let _ = fs::rename(&self.path, self.rotated(1));
        Self::open_append(&self.path)
    }

    pub fn log(&self, level: &str, msg: &str) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("{secs} [{level}] {msg}\n");
        let Ok(mut g) = self.inner.lock() else { return };
        if g.1 + line.len() as u64 > MAX_BYTES {
            if let Ok(n) = self.rotate() {
                *g = n;
            }
        }
        if g.0.write_all(line.as_bytes()).is_ok() {
            g.1 += line.len() as u64;
        }
    }

    pub fn info(&self, msg: &str) {
        self.log("INFO", msg)
    }
    pub fn error(&self, msg: &str) {
        self.log("ERROR", msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_rotates() {
        let dir = std::env::temp_dir().join(format!("d256shell-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let l = Logger::open(&dir).unwrap();
        l.info("hello");
        assert!(fs::read_to_string(dir.join("d256shell.log"))
            .unwrap()
            .contains("[INFO] hello"));
        let big = "x".repeat(100_000);
        for _ in 0..25 {
            l.info(&big);
        }
        assert!(dir.join("d256shell.log.1").exists());
        assert!(fs::metadata(dir.join("d256shell.log")).unwrap().len() <= MAX_BYTES);
        fs::remove_dir_all(dir).unwrap();
    }
}
