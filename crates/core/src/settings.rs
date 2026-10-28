//! Unencrypted user settings (`settings.json`). Unknown/missing fields fall back to defaults, so older
//! and newer files both load.
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoPasswordMode {
    /// askpass when ssh >= 8.4, expect otherwise.
    Auto,
    Askpass,
    Expect,
    /// Never type or supply the password automatically.
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Explicit path to ssh.exe; empty = auto-detect.
    pub ssh_path: String,
    pub auto_password: AutoPasswordMode,
    pub font_family: String,
    pub font_size: u32,
    pub scheme: String,
    pub cursor_style: String,
    pub scrollback: u32,
    /// "menu" | "copy-paste"
    pub right_click: String,
    /// Mouse wheel in full-screen programs that track the mouse (btop, htop, mc): arrows or mouse.
    pub wheel_mode: String,
    pub paste_warn_lines: u32,
    pub paste_warn_bytes: u32,
    /// Simultaneous transfers (1..8).
    pub parallel_transfers: u32,
    /// Keep the modification time of transferred files.
    pub keep_mtime: bool,
    /// Largest drag-out (panel to Explorer) in megabytes.
    pub drag_out_limit_mb: u32,
    /// Largest file the editor opens, in megabytes.
    pub editor_limit_mb: u32,
    /// Lock the vault after this many idle minutes (0 = never).
    pub auto_lock_minutes: u32,
    /// Show the server load bar under the terminal.
    /// Duplicating a terminal: "ask" (show what will be typed into the copy first), "auto" (just do it), "view" (output only).
    pub clone_mode: String,
    pub monitor_enabled: bool,
    /// Milliseconds between load samples (100..60000).
    pub monitor_interval_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ssh_path: String::new(),
            auto_password: AutoPasswordMode::Auto,
            font_family: "'Cascadia Mono', Consolas, 'Courier New', monospace".into(),
            font_size: 14,
            scheme: "campbell".into(),
            cursor_style: "bar".into(),
            scrollback: 10_000,
            right_click: "copy-paste".into(),
            wheel_mode: "arrows".into(),
            paste_warn_lines: 2,
            paste_warn_bytes: 5 * 1024,
            parallel_transfers: 2,
            keep_mtime: true,
            drag_out_limit_mb: 200,
            editor_limit_mb: 5,
            auto_lock_minutes: 0,
            clone_mode: "ask".into(),
            monitor_enabled: true,
            monitor_interval_ms: 2000,
        }
    }
}

impl Settings {
    /// Clamp values to sane ranges (settings.json can be edited by hand).
    pub fn sanitized(mut self) -> Self {
        self.font_size = self.font_size.clamp(6, 48);
        self.parallel_transfers = self.parallel_transfers.clamp(1, 8);
        self.drag_out_limit_mb = self.drag_out_limit_mb.clamp(1, 4096);
        self.editor_limit_mb = self.editor_limit_mb.clamp(1, 100);
        self.auto_lock_minutes = self.auto_lock_minutes.min(1440);
        self.monitor_interval_ms = self.monitor_interval_ms.clamp(100, 60_000);
        if !["ask", "auto", "view"].contains(&self.clone_mode.as_str()) {
            self.clone_mode = "ask".into();
        }
        self.scrollback = self.scrollback.clamp(100, 1_000_000);
        if !["bar", "block", "underline"].contains(&self.cursor_style.as_str()) {
            self.cursor_style = "bar".into();
        }
        if !["menu", "copy-paste"].contains(&self.right_click.as_str()) {
            self.right_click = "copy-paste".into();
        }
        if !["arrows", "mouse"].contains(&self.wheel_mode.as_str()) {
            self.wheel_mode = "arrows".into();
        }
        self
    }

    pub fn load(path: &Path) -> Settings {
        match fs::read(path) {
            Ok(b) => {
                let mut s = serde_json::from_slice::<Settings>(&b).unwrap_or_default();
                // files written before the interval was given in milliseconds keep their value (seconds -> ms)
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                    if v.get("monitor_interval_ms").is_none() {
                        if let Some(sec) = v
                            .get("monitor_interval_sec")
                            .and_then(serde_json::Value::as_u64)
                        {
                            s.monitor_interval_ms = (sec as u32).saturating_mul(1000);
                        }
                    }
                }
                s.sanitized()
            }
            Err(_) => Settings::default(),
        }
    }

    /// Atomic save (tmp + rename).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("d256shell-set-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.join("settings.json")
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(Settings::load(&tmp("missing")), Settings::default());
    }

    #[test]
    fn roundtrip_and_partial_files() {
        let p = tmp("rt");
        let s = Settings {
            font_size: 18,
            scheme: "onehalfdark".into(),
            ..Settings::default()
        };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
        // unknown and missing fields are tolerated
        fs::write(&p, r#"{"font_size": 20, "future_option": true}"#).unwrap();
        let l = Settings::load(&p);
        assert_eq!(l.font_size, 20);
        assert_eq!(l.scheme, "campbell");
    }

    #[test]
    fn broken_json_and_absurd_values_are_sanitized() {
        let p = tmp("bad");
        fs::write(&p, "{not json").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
        fs::write(
            &p,
            r#"{"font_size": 9999, "scrollback": 1, "cursor_style": "x", "right_click": "y"}"#,
        )
        .unwrap();
        let l = Settings::load(&p);
        assert_eq!((l.font_size, l.scrollback), (48, 100));
        assert_eq!(
            (l.cursor_style.as_str(), l.right_click.as_str()),
            ("bar", "copy-paste")
        );
    }
}

#[cfg(test)]
mod interval_tests {
    use super::*;

    #[test]
    fn interval_is_in_milliseconds_clamped_and_old_seconds_are_migrated() {
        let mut s = Settings {
            monitor_interval_ms: 5,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.monitor_interval_ms, 100, "never faster than 100 ms");
        s.monitor_interval_ms = 10_000_000;
        assert_eq!(s.sanitized().monitor_interval_ms, 60_000);
        let dir = std::env::temp_dir().join(format!("d256shell-set-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        std::fs::write(&p, br#"{"monitor_interval_sec": 5}"#).unwrap();
        assert_eq!(Settings::load(&p).monitor_interval_ms, 5000);
        std::fs::write(
            &p,
            br#"{"monitor_interval_sec": 5, "monitor_interval_ms": 250}"#,
        )
        .unwrap();
        assert_eq!(
            Settings::load(&p).monitor_interval_ms,
            250,
            "the new field wins"
        );
        std::fs::write(&p, b"{}").unwrap();
        assert_eq!(Settings::load(&p).monitor_interval_ms, 2000);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
