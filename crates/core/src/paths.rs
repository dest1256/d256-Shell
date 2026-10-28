//! Portable data directory layout.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const ENV_DATA_DIR: &str = "D256SHELL_DATA_DIR";

#[derive(Debug)]
pub enum DataDirError {
    NoExeDir,
    NotWritable { dir: PathBuf, source: io::Error },
}

impl std::fmt::Display for DataDirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoExeDir => write!(f, "Не удалось определить папку программы."),
            Self::NotWritable { dir, source } => write!(
                f,
                "Папка данных недоступна для записи: {}\nПереместите программу в папку, где у вас есть права записи (не в Program Files), либо задайте папку данных через D256SHELL_DATA_DIR.\nПодробности: {}",
                dir.display(),
                source
            ),
        }
    }
}

impl std::error::Error for DataDirError {}

/// All paths inside the portable `data` folder.
#[derive(Debug, Clone)]
pub struct DataDirs {
    pub root: PathBuf,
}

impl DataDirs {
    pub fn vault(&self) -> PathBuf {
        self.root.join("vault.bin")
    }
    pub fn vault_bak(&self) -> PathBuf {
        self.root.join("vault.bin.bak")
    }
    pub fn known_hosts(&self) -> PathBuf {
        self.root.join("known_hosts")
    }
    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }
    pub fn webview(&self) -> PathBuf {
        self.root.join("webview")
    }
    pub fn staging(&self) -> PathBuf {
        self.root.join("staging")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn openssh(&self) -> PathBuf {
        self.root.join("OpenSSH")
    }
}

/// `D256SHELL_DATA_DIR` if set and non-empty, otherwise `<exe dir>/data`.
/// Never falls back to %APPDATA% or similar. On Linux an installed package lives in `/usr/bin` (not writable),
/// so there the portable `data` folder next to the binary is used only if it exists (the tar.gz ships one),
/// otherwise the XDG data folder (`$XDG_DATA_HOME/d256shell`, `~/.local/share/d256shell`).
pub fn resolve_root(
    env_override: Option<&str>,
    exe_dir: Option<&Path>,
) -> Result<PathBuf, DataDirError> {
    match env_override.filter(|s| !s.is_empty()) {
        Some(p) => Ok(PathBuf::from(p)),
        None => {
            let portable = exe_dir.map(|d| d.join("data"));
            if cfg!(windows) {
                return portable.ok_or(DataDirError::NoExeDir);
            }
            match portable {
                Some(p) if p.is_dir() => Ok(p),
                other => xdg_data_root(
                    std::env::var("XDG_DATA_HOME").ok().as_deref(),
                    std::env::var("HOME").ok().as_deref(),
                )
                .or(other)
                .ok_or(DataDirError::NoExeDir),
            }
        }
    }
}

/// `$XDG_DATA_HOME/d256shell`, else `$HOME/.local/share/d256shell` (relative XDG values are ignored per the spec).
pub fn xdg_data_root(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    match xdg.filter(|x| Path::new(x).is_absolute()) {
        Some(x) => Some(PathBuf::from(x).join("d256shell")),
        None => home
            .filter(|h| !h.is_empty())
            .map(|h| PathBuf::from(h).join(".local/share/d256shell")),
    }
}

/// Create the data directory and subfolders, check it is writable (probe file is created and removed).
pub fn prepare(root: PathBuf) -> Result<DataDirs, DataDirError> {
    let not_writable = |source| DataDirError::NotWritable {
        dir: root.clone(),
        source,
    };
    fs::create_dir_all(&root).map_err(not_writable)?;
    let probe = root.join(format!(".write-test-{}", std::process::id()));
    fs::write(&probe, b"x").map_err(not_writable)?;
    fs::remove_file(&probe).map_err(not_writable)?;
    let dirs = DataDirs { root };
    for d in [dirs.webview(), dirs.staging(), dirs.logs()] {
        fs::create_dir_all(&d).map_err(|source| DataDirError::NotWritable {
            dir: d.clone(),
            source,
        })?;
    }
    Ok(dirs)
}

/// Resolve from the real environment and current executable.
pub fn init() -> Result<DataDirs, DataDirError> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let env = std::env::var(ENV_DATA_DIR).ok();
    prepare(resolve_root(env.as_deref(), exe_dir.as_deref())?)
}

/// Remove everything inside `staging` (called at startup and exit).
pub fn clear_staging(dirs: &DataDirs) -> io::Result<()> {
    let s = dirs.staging();
    if s.exists() {
        fs::remove_dir_all(&s)?;
    }
    fs::create_dir_all(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("d256shell-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn env_override_wins() {
        let r = resolve_root(Some("/x/y"), Some(Path::new("/exe"))).unwrap();
        assert_eq!(r, PathBuf::from("/x/y"));
    }

    #[test]
    fn empty_env_falls_back_to_the_portable_data_folder_next_to_the_exe() {
        let exe = tmp("portable");
        fs::create_dir_all(exe.join("data")).unwrap();
        let r = resolve_root(Some(""), Some(&exe)).unwrap();
        assert_eq!(r, exe.join("data"));
        fs::remove_dir_all(exe).unwrap();
    }

    #[test]
    fn xdg_locations() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            xdg_data_root(Some("/x/share"), Some("/home/u")),
            p("/x/share/d256shell")
        );
        // a relative XDG_DATA_HOME is invalid by the spec: ignored
        assert_eq!(
            xdg_data_root(Some("rel"), Some("/home/u")),
            p("/home/u/.local/share/d256shell")
        );
        assert_eq!(
            xdg_data_root(None, Some("/home/u")),
            p("/home/u/.local/share/d256shell")
        );
        assert_eq!(xdg_data_root(None, Some("")), None);
        assert_eq!(xdg_data_root(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn without_a_portable_folder_linux_uses_xdg_not_the_install_dir() {
        let exe = tmp("installed"); // like /usr/bin: no data folder, maybe not writable
        fs::create_dir_all(&exe).unwrap();
        let r = resolve_root(None, Some(&exe)).unwrap();
        assert!(!r.starts_with(&exe), "{r:?}");
        assert!(r.ends_with("d256shell"), "{r:?}");
        fs::remove_dir_all(exe).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn no_exe_dir_is_error() {
        assert!(matches!(
            resolve_root(None, None),
            Err(DataDirError::NoExeDir)
        ));
    }

    #[test]
    fn prepare_creates_layout_and_leaves_no_probe() {
        let root = tmp("layout");
        let d = prepare(root.clone()).unwrap();
        assert!(d.webview().is_dir() && d.staging().is_dir() && d.logs().is_dir());
        let leftovers = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".write-test"))
            .count();
        assert_eq!(leftovers, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unwritable_location_is_clear_error() {
        // A path under a regular file can never be created.
        let root = tmp("file");
        fs::write(&root, b"f").unwrap();
        let err = prepare(root.join("data")).unwrap_err();
        assert!(matches!(err, DataDirError::NotWritable { .. }));
        assert!(err.to_string().contains("недоступна для записи"));
        fs::remove_file(root).unwrap();
    }

    #[test]
    fn clear_staging_empties_folder() {
        let root = tmp("staging");
        let d = prepare(root.clone()).unwrap();
        fs::create_dir_all(d.staging().join("a/b")).unwrap();
        fs::write(d.staging().join("a/b/f"), b"1").unwrap();
        clear_staging(&d).unwrap();
        assert_eq!(fs::read_dir(d.staging()).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
