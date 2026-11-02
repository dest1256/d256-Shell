//! File operations on THIS computer for the local explorer (the SFTP explorer's twin).
//! Paths cross the API in one form on every platform: `/`-separated. On Windows a path starts with the drive:
//! `/C:/Users/me`, and `/` itself is the list of drives. Nothing here ever overwrites anything: a clash is an
//! error, or for copies a new "name (copy)" name.
use crate::remote_path::{is_same_or_inside, is_valid_name, normalize};
use crate::sftp::model::{DirEntry, EntryKind};
use crate::sftp::xfer::windows_name_ok;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

type R<T> = Result<T, String>;
const MAX_DEPTH: usize = 64;

// ---------------------------------------------------------------- path forms

/// API path -> real path (Windows: `/C:/x` -> `C:/x`, `/C:` -> `C:/`). `/` alone (the drive list) has no real path.
pub fn real_path(ui: &str, windows: bool) -> R<PathBuf> {
    let p = ui.replace('\\', "/");
    if !windows {
        return if p.starts_with('/') {
            Ok(PathBuf::from(p))
        } else {
            Err(format!("Invalid path: {ui}"))
        };
    }
    let rest = p.strip_prefix('/').unwrap_or(&p);
    let b = rest.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'/')
    {
        let tail = &rest[2..];
        return Ok(PathBuf::from(format!(
            "{}:/{}",
            rest[..1].to_ascii_uppercase(),
            tail.trim_start_matches('/')
        )));
    }
    Err(format!("Invalid path: {ui}"))
}

/// Real path -> API path.
pub fn ui_path(real: &Path, windows: bool) -> String {
    let s = real.to_string_lossy().replace('\\', "/");
    if windows {
        let s = s.strip_prefix("//?/").unwrap_or(&s);
        let s = s.trim_end_matches('/');
        format!("/{s}")
    } else {
        s
    }
}

fn win() -> bool {
    cfg!(windows)
}

/// The user's home folder in API form.
pub fn home() -> String {
    let var = if win() { "USERPROFILE" } else { "HOME" };
    match std::env::var(var).ok().filter(|v| !v.is_empty()) {
        Some(h) => ui_path(Path::new(&h), win()),
        None => "/".into(),
    }
}

// ---------------------------------------------------------------- listing

fn entry_of(name: String, lossy: bool, path: &Path) -> DirEntry {
    let meta = fs::symlink_metadata(path).ok();
    let is_link = meta.as_ref().is_some_and(|m| m.file_type().is_symlink());
    let target = if is_link {
        fs::metadata(path).ok()
    } else {
        None
    };
    let m = if is_link {
        target.as_ref().or(meta.as_ref())
    } else {
        meta.as_ref()
    };
    let kind = match &meta {
        Some(m) if m.file_type().is_symlink() => EntryKind::Symlink,
        Some(m) if m.is_dir() => EntryKind::Dir,
        Some(m) if m.is_file() => EntryKind::File,
        _ => EntryKind::Other,
    };
    #[cfg(unix)]
    let (mode, uid, gid) = {
        use std::os::unix::fs::MetadataExt;
        (m.map(|m| m.mode()), m.map(|m| m.uid()), m.map(|m| m.gid()))
    };
    #[cfg(not(unix))]
    let (mode, uid, gid) = (None, None, None);
    DirEntry {
        name,
        kind,
        target_is_dir: if is_link {
            Some(target.as_ref().is_some_and(|t| t.is_dir()))
        } else {
            None
        },
        size: m.map(|m| if m.is_dir() { 0 } else { m.len() }).unwrap_or(0),
        mtime: m
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs().min(u64::from(u32::MAX)) as u32),
        mode,
        uid,
        gid,
        owner: None,
        group: None,
        name_lossy: lossy,
    }
}

/// List a folder: returns the canonical API path and its entries. `None` / "" = the home folder.
pub fn list(path: Option<&str>) -> R<(String, Vec<DirEntry>)> {
    let ui = match path {
        None | Some("") => home(),
        Some(p) => p.replace('\\', "/"),
    };
    // a drive path typed by hand (C:/Users) gets the leading slash of the API form
    let ui = if win() && !ui.starts_with('/') {
        format!("/{ui}")
    } else {
        ui
    };
    let ui = normalize(&ui);
    if win() && ui == "/" {
        let mut v = Vec::new();
        for l in b'A'..=b'Z' {
            let root = format!("{}:/", l as char);
            if Path::new(&root).exists() {
                v.push(DirEntry {
                    name: format!("{}:", l as char),
                    kind: EntryKind::Dir,
                    target_is_dir: None,
                    size: 0,
                    mtime: None,
                    mode: None,
                    uid: None,
                    gid: None,
                    owner: None,
                    group: None,
                    name_lossy: false,
                });
            }
        }
        return Ok(("/".into(), v));
    }
    let real = real_path(&ui, win())?;
    let canon = fs::canonicalize(&real).map_err(|e| describe(&e))?;
    let rd = fs::read_dir(&canon).map_err(|e| describe(&e))?;
    let mut out = Vec::new();
    for e in rd.flatten() {
        let (name, lossy) = match e.file_name().into_string() {
            Ok(n) => (n, false),
            Err(os) => (os.to_string_lossy().into_owned(), true),
        };
        out.push(entry_of(name, lossy, &e.path()));
    }
    Ok((ui_path(&canon, win()), out))
}

fn describe(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => "No such file or folder.".into(),
        PermissionDenied => "No access rights.".into(),
        _ => format!("Error: {e}"),
    }
}

// ---------------------------------------------------------------- operations

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_of(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => path[..i].to_string(),
    }
}

fn name_ok(name: &str) -> R<()> {
    if !is_valid_name(name) || (win() && !windows_name_ok(name)) {
        return Err("Invalid name.".into());
    }
    Ok(())
}

fn exists(real: &Path) -> bool {
    fs::symlink_metadata(real).is_ok()
}

fn is_dir_nofollow(real: &Path) -> bool {
    fs::symlink_metadata(real)
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

pub fn mkdir(parent: &str, name: &str) -> R<String> {
    name_ok(name)?;
    let p = join(&normalize(parent), name);
    fs::create_dir(real_path(&p, win())?).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!("“{name}” already exists.")
        } else {
            describe(&e)
        }
    })?;
    Ok(p)
}

pub fn create_file(parent: &str, name: &str) -> R<String> {
    name_ok(name)?;
    let p = join(&normalize(parent), name);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(real_path(&p, win())?)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                format!("“{name}” already exists.")
            } else {
                describe(&e)
            }
        })?;
    Ok(p)
}

pub fn rename(path: &str, new_name: &str) -> R<String> {
    name_ok(new_name)?;
    let path = normalize(path);
    let to = join(&parent_of(&path), new_name);
    let (from_r, to_r) = (real_path(&path, win())?, real_path(&to, win())?);
    if exists(&to_r) && !(win() && path.eq_ignore_ascii_case(&to)) {
        return Err(format!("“{new_name}” already exists."));
    }
    fs::rename(from_r, to_r).map_err(|e| describe(&e))?;
    Ok(to)
}

/// `name`, `name (copy)`, `name (copy 2)` ... in `dir` (extension kept).
fn free_name(dir: &str, name: &str) -> R<String> {
    let taken = |n: &str| real_path(&join(dir, n), win()).map(|p| exists(&p));
    if !taken(name)? {
        return Ok(join(dir, name));
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..1000 {
        let cand = if n == 1 {
            format!("{stem} (copy){ext}")
        } else {
            format!("{stem} (copy {n}){ext}")
        };
        if !taken(&cand)? {
            return Ok(join(dir, &cand));
        }
    }
    Err("Could not find a free name.".into())
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn move_entry(path: &str, dest_dir: &str) -> R<String> {
    let (path, dest) = (normalize(path), normalize(dest_dir));
    if path == "/" || real_path(&path, win()).is_err() {
        return Err(format!("Invalid path: {path}"));
    }
    if is_same_or_inside(&dest, &path) {
        return Err("A folder cannot be moved into itself.".into());
    }
    let to = join(&dest, file_name(&path));
    let (from_r, to_r) = (real_path(&path, win())?, real_path(&to, win())?);
    if exists(&to_r) {
        return Err(format!(
            "“{}” already exists in the destination folder.",
            file_name(&path)
        ));
    }
    if fs::rename(&from_r, &to_r).is_err() {
        // another volume: copy, then remove the original
        copy_tree(&from_r, &to_r, 0)?;
        remove_tree(&from_r)?;
    }
    Ok(to)
}

fn copy_tree(from: &Path, to: &Path, depth: usize) -> R<()> {
    if depth > MAX_DEPTH {
        return Err("Nesting is too deep.".into());
    }
    let meta = fs::symlink_metadata(from).map_err(|e| describe(&e))?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        #[cfg(unix)]
        {
            let target = fs::read_link(from).map_err(|e| describe(&e))?;
            std::os::unix::fs::symlink(target, to).map_err(|e| describe(&e))?;
            return Ok(());
        }
        #[cfg(not(unix))]
        {
            // follow a link to a file; a link to a folder is skipped (no privilege to create links)
            if fs::metadata(from).map(|m| m.is_file()).unwrap_or(false) {
                fs::copy(from, to).map_err(|e| describe(&e))?;
            }
            return Ok(());
        }
    }
    if ft.is_dir() {
        fs::create_dir(to).map_err(|e| describe(&e))?;
        for e in fs::read_dir(from).map_err(|e| describe(&e))?.flatten() {
            copy_tree(&e.path(), &to.join(e.file_name()), depth + 1)?;
        }
        return Ok(());
    }
    if ft.is_file() {
        fs::copy(from, to).map_err(|e| describe(&e))?;
        return Ok(());
    }
    Err("Special files cannot be copied.".into())
}

pub fn copy_entry(path: &str, dest_dir: &str) -> R<String> {
    let (path, dest) = (normalize(path), normalize(dest_dir));
    let from = real_path(&path, win())?;
    if is_dir_nofollow(&from) && is_same_or_inside(&dest, &path) {
        return Err("A folder cannot be copied into itself.".into());
    }
    let to_ui = free_name(&dest, file_name(&path))?;
    copy_tree(&from, &real_path(&to_ui, win())?, 0)?;
    Ok(to_ui)
}

fn remove_tree(real: &Path) -> R<()> {
    let meta = fs::symlink_metadata(real).map_err(|e| describe(&e))?;
    if meta.is_dir() {
        fs::remove_dir_all(real).map_err(|e| describe(&e))
    } else {
        // files and links (a link is removed itself, never its target)
        fs::remove_file(real)
            .or_else(|_| fs::remove_dir(real))
            .map_err(|e| describe(&e))
    }
}

/// Things that are never deleted from here: the drive list, drive roots, the home folder and everything containing it,
/// and the system folders.
fn forbidden(path: &str, home: &str, windows: bool) -> bool {
    let p = normalize(path);
    if p == "/" || is_same_or_inside(home, &p) {
        return true;
    }
    if windows {
        let rest = p.trim_start_matches('/');
        let (drive, tail) = rest.split_once('/').unwrap_or((rest, ""));
        let top = tail.split('/').next().unwrap_or("").to_ascii_lowercase();
        let single = !tail.contains('/');
        return drive.len() == 2
            && (tail.is_empty()
                || (single
                    && [
                        "windows",
                        "program files",
                        "program files (x86)",
                        "programdata",
                        "users",
                    ]
                    .contains(&top.as_str())));
    }
    [
        "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib32", "/lib64", "/opt", "/proc",
        "/root", "/run", "/sbin", "/srv", "/sys", "/usr", "/var",
    ]
    .contains(&p.as_str())
}

pub fn delete_entry(path: &str) -> R<()> {
    let p = normalize(path);
    if forbidden(&p, &home(), win()) {
        return Err(format!("“{}” is a home folder or its parent: such things can only be deleted by hand in a terminal.", file_name(&p)));
    }
    remove_tree(&real_path(&p, win())?)
}

/// (files, folders) in the tree, links not followed.
pub fn count_tree(path: &str) -> R<(u64, u64)> {
    fn walk(p: &Path, depth: usize, files: &mut u64, dirs: &mut u64) -> R<()> {
        if depth > MAX_DEPTH {
            return Err("Nesting is too deep.".into());
        }
        let m = fs::symlink_metadata(p).map_err(|e| describe(&e))?;
        if m.is_dir() {
            *dirs += 1;
            for e in fs::read_dir(p).map_err(|e| describe(&e))?.flatten() {
                walk(&e.path(), depth + 1, files, dirs)?;
            }
        } else {
            *files += 1;
        }
        Ok(())
    }
    let (mut f, mut d) = (0, 0);
    walk(&real_path(&normalize(path), win())?, 0, &mut f, &mut d)?;
    Ok((f, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_cross_the_api_with_a_leading_slash() {
        assert_eq!(
            real_path("/C:/Users/me", true).unwrap(),
            PathBuf::from("C:/Users/me")
        );
        assert_eq!(real_path("/c:", true).unwrap(), PathBuf::from("C:/"));
        assert_eq!(
            real_path("C:\\a\\b", true).unwrap(),
            PathBuf::from("C:/a/b")
        );
        assert!(
            real_path("/", true).is_err(),
            "the drive list has no real path"
        );
        assert!(real_path("/Users", true).is_err());
        assert_eq!(ui_path(Path::new("C:\\Users\\me"), true), "/C:/Users/me");
        assert_eq!(ui_path(Path::new("\\\\?\\D:\\x\\"), true), "/D:/x");
        assert_eq!(ui_path(Path::new("/home/u"), false), "/home/u");
        assert!(real_path("relative", false).is_err());
    }

    #[test]
    fn dangerous_things_are_never_deleted() {
        let home = "/home/u";
        for p in ["/", "/home", "/home/u", "/etc", "/usr"] {
            assert!(forbidden(p, home, false), "{p}");
        }
        assert!(!forbidden("/home/u/docs", home, false));
        let whome = "/C:/Users/me";
        for p in [
            "/",
            "/C:",
            "/C:/",
            "/C:/Windows",
            "/C:/Program Files",
            "/C:/Users",
            "/C:/Users/me",
        ] {
            assert!(forbidden(p, whome, true), "{p}");
        }
        assert!(!forbidden("/C:/Users/me/Documents", whome, true));
        assert!(
            !forbidden("/D:/Windows/foo", whome, true),
            "only top-level system folders"
        );
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("d256shell-lfs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(unix)]
    #[test]
    fn operations_never_overwrite_and_copies_get_new_names() {
        let d = tmp("ops");
        let u = |n: &str| ui_path(&d.join(n), false);
        let root = ui_path(&d, false);
        assert_eq!(mkdir(&root, "a").unwrap(), u("a"));
        assert!(mkdir(&root, "a").is_err(), "exists");
        for bad in ["", ".", "..", "x/y", "a\0b"] {
            assert!(mkdir(&root, bad).is_err(), "{bad:?}");
        }
        create_file(&u("a"), "f.txt").unwrap();
        fs::write(d.join("a/f.txt"), "data").unwrap();
        assert!(create_file(&u("a"), "f.txt").is_err());
        // copy next to itself -> "(copy)", again -> "(copy 2)"
        assert_eq!(
            copy_entry(&u("a/f.txt"), &u("a")).unwrap(),
            u("a/f (copy).txt")
        );
        assert_eq!(
            copy_entry(&u("a/f.txt"), &u("a")).unwrap(),
            u("a/f (copy 2).txt")
        );
        assert_eq!(
            fs::read_to_string(d.join("a/f (copy).txt")).unwrap(),
            "data"
        );
        // folder tree copy; never into itself
        mkdir(&root, "b").unwrap();
        assert_eq!(copy_entry(&u("a"), &u("b")).unwrap(), u("b/a"));
        assert!(d.join("b/a/f.txt").exists());
        assert!(copy_entry(&u("a"), &u("a")).is_err(), "folder into itself");
        assert!(move_entry(&u("a"), &u("a/x")).is_err());
        // move refuses a clash, rename too
        assert!(
            move_entry(&u("b/a"), &root).is_err(),
            "a already exists in the destination"
        );
        fs::write(d.join("g.txt"), "g").unwrap();
        assert_eq!(rename(&u("g.txt"), "h.txt").unwrap(), u("h.txt"));
        assert!(rename(&u("h.txt"), "b").is_err());
        assert_eq!(move_entry(&u("h.txt"), &u("b")).unwrap(), u("b/h.txt"));
        // counts and delete (a symlink is removed, not its target)
        std::os::unix::fs::symlink(d.join("a"), d.join("link")).unwrap();
        assert_eq!(count_tree(&u("b")).unwrap(), (4, 2)); // b/a with 3 files, h.txt
        delete_entry(&u("link")).unwrap();
        assert!(d.join("a/f.txt").exists(), "the link's target survives");
        delete_entry(&u("b")).unwrap();
        assert!(!d.join("b").exists());
        assert!(delete_entry("/").is_err() && delete_entry("/etc").is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn listing_reports_kinds_links_and_hidden_files() {
        let d = tmp("list");
        fs::write(d.join(".hidden"), "x").unwrap();
        fs::write(d.join("file.txt"), "12345").unwrap();
        fs::create_dir(d.join("dir")).unwrap();
        std::os::unix::fs::symlink(d.join("dir"), d.join("tolink")).unwrap();
        std::os::unix::fs::symlink(d.join("nowhere"), d.join("broken")).unwrap();
        let (path, mut v) = list(Some(&ui_path(&d, false))).unwrap();
        assert_eq!(path, ui_path(&fs::canonicalize(&d).unwrap(), false));
        v.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [".hidden", "broken", "dir", "file.txt", "tolink"]);
        let by = |n: &str| v.iter().find(|e| e.name == n).unwrap();
        assert_eq!(by("file.txt").size, 5);
        assert_eq!(by("dir").kind, EntryKind::Dir);
        assert_eq!(
            (by("tolink").kind, by("tolink").target_is_dir),
            (EntryKind::Symlink, Some(true))
        );
        assert_eq!(by("broken").target_is_dir, Some(false));
        assert!(list(Some("/definitely/not/here")).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
