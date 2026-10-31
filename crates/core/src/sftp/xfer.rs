//! Safe file transfer (upload / download / in-place saves). Rules, in short:
//! * data goes to a temporary `.name.d256shell-<rand>.part` next to the target, its size is verified, and only
//!   then it replaces the target; the original is never removed before the new file is complete;
//! * an existing target is replaced via "original -> .bak, tmp -> target, remove .bak" with a rollback;
//! * symlink targets are written through (the real file is replaced, the link stays), folders / special
//!   files are refused, permissions of the old file are kept, a foreign owner needs an explicit decision;
//! * cancel or an error removes the temporary file (or remembers it in `stale` when the link is gone).
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
use super::conn::{describe, SftpConnection};
use crate::remote_path::{self, join};
use russh_sftp::client::fs::Metadata;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const CHUNK: usize = 128 * 1024;
const MAX_DEPTH: usize = 64;

#[derive(Debug)]
pub enum XferError {
    Msg(String),
    Cancelled,
    /// The target exists and `overwrite` was not requested.
    Exists,
    /// The target belongs to another user: replacing it would change the owner.
    ForeignOwner,
    /// The server refused access (the parent folder or the file is not writable for this user).
    NoPerm,
}

impl std::fmt::Display for XferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Msg(m) => f.write_str(m),
            Self::Cancelled => f.write_str("Cancelled."),
            Self::NoPerm => f.write_str("No access rights."),
            Self::Exists => f.write_str("The file already exists."),
            Self::ForeignOwner => {
                f.write_str("The file belongs to another user: replacing it will change the owner.")
            }
        }
    }
}

impl From<String> for XferError {
    fn from(s: String) -> Self {
        Self::Msg(s)
    }
}

type R<T> = Result<T, XferError>;

fn sftp_err(e: russh_sftp::client::error::Error) -> XferError {
    use russh_sftp::client::error::Error;
    if let Error::Status(s) = &e {
        if s.status_code == russh_sftp::protocol::StatusCode::PermissionDenied {
            return XferError::NoPerm;
        }
    }
    XferError::Msg(describe(e))
}

fn io_err(what: &str, e: std::io::Error) -> XferError {
    XferError::Msg(format!("{what}: {e}"))
}

/// What to do with an existing target (chosen by the user).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Replace,
    Skip,
    Rename,
}

/// Shared progress / cancellation state of one transfer.
#[derive(Default)]
pub struct Ctl {
    pub cancel: AtomicBool,
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub current: Mutex<String>,
}

impl Ctl {
    fn check(&self) -> R<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(XferError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn set_current(&self, s: &str) {
        *self.current.lock().expect("lock") = s.to_string();
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOpts {
    /// Replace an existing target.
    pub overwrite: bool,
    /// Replace a file owned by someone else anyway.
    pub allow_foreign: bool,
    /// Write straight into the existing file (truncate + write, not atomic).
    pub in_place: bool,
    /// Modification time to set on the result (seconds).
    pub mtime: Option<u32>,
}

#[derive(Debug, Default, Clone)]
pub struct Outcome {
    pub files: u64,
    pub skipped: Vec<String>,
}

fn rand_id() -> String {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("rng");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn secs(t: std::time::SystemTime) -> Option<u32> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u32::try_from(d.as_secs()).ok())
}

/// `name (1).ext`, `name (2).ext`, ... (the extension of a dotfile like `.bashrc` is empty).
pub fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

/// Can this remote name be a local file name? Windows is strict (reserved device names, `:`, trailing dot ...);
/// on Linux anything the remote side allows is fine.
pub fn local_name_ok(name: &str) -> bool {
    if cfg!(windows) {
        windows_name_ok(name)
    } else {
        remote_path::is_valid_name(name)
    }
}

/// The Windows rules (kept testable on every platform).
pub fn windows_name_ok(name: &str) -> bool {
    if !remote_path::is_valid_name(name) || name.ends_with([' ', '.']) {
        return false;
    }
    if name.contains(['\\', ':', '*', '?', '"', '<', '>', '|']) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    let com_lpt = |p: &str| {
        stem.len() == 4
            && stem.starts_with(p)
            && stem.as_bytes()[3].is_ascii_digit()
            && stem != format!("{p}0")
    };
    !(reserved.contains(&stem.as_str()) || com_lpt("COM") || com_lpt("LPT"))
}

async fn pump<Rd, Wr>(src: &mut Rd, dst: &mut Wr, ctl: Option<&Ctl>) -> R<u64>
where
    Rd: AsyncRead + Unpin,
    Wr: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if let Some(c) = ctl {
            c.check()?;
        }
        let n = src.read(&mut buf).await.map_err(|e| io_err("Read", e))?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])
            .await
            .map_err(|e| io_err("Write", e))?;
        total += n as u64;
        if let Some(c) = ctl {
            c.done.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    dst.flush().await.map_err(|e| io_err("Write", e))?;
    Ok(total)
}

impl SftpConnection {
    pub async fn exists_lstat(&self, path: &str) -> R<Option<Metadata>> {
        match self.sftp.symlink_metadata(path).await {
            Ok(m) => Ok(Some(m)),
            Err(russh_sftp::client::error::Error::Status(s))
                if s.status_code == russh_sftp::protocol::StatusCode::NoSuchFile =>
            {
                Ok(None)
            }
            Err(e) => Err(sftp_err(e)),
        }
    }

    /// First free `name (n).ext` in `dir`.
    pub async fn free_numbered(&self, dir: &str, name: &str) -> R<String> {
        for n in 1..10_000 {
            let cand = numbered(name, n);
            if self.exists_lstat(&join(dir, &cand)).await?.is_none() {
                return Ok(cand);
            }
        }
        Err(XferError::Msg("Could not find a free name.".into()))
    }

    /// Remove leftover temporary files of earlier failed transfers (the link was gone when they failed).
    pub async fn cleanup_stale(&self) {
        let list: Vec<String> = std::mem::take(&mut *self.stale.lock().expect("lock"));
        for p in list {
            if self.sftp.remove_file(p.clone()).await.is_err()
                && !matches!(self.exists_lstat(&p).await, Ok(None))
            {
                self.stale.lock().expect("lock").push(p);
            }
        }
    }

    async fn drop_remote_tmp(&self, path: &str) {
        match self.sftp.remove_file(path.to_string()).await {
            Ok(()) => {}
            Err(_) => {
                if !matches!(self.exists_lstat(path).await, Ok(None)) {
                    self.stale.lock().expect("lock").push(path.to_string());
                }
            }
        }
    }

    /// Where a write to `target` really lands: through a symlink to the real file.
    async fn resolve_target(&self, target: &str) -> R<(String, Option<Metadata>)> {
        let Some(m) = self.exists_lstat(target).await? else {
            return Ok((target.to_string(), None));
        };
        if !m.file_type().is_symlink() {
            return Ok((target.to_string(), Some(m)));
        }
        let real = self
            .sftp
            .canonicalize(target)
            .await
            .map_err(|_| XferError::Msg("The symbolic link points to nothing.".into()))?;
        match self.exists_lstat(&real).await? {
            Some(m) => Ok((real, Some(m))),
            // some servers resolve a dangling link to its would-be target: never create files through it
            None => Err(XferError::Msg(
                "The symbolic link points to nothing.".into(),
            )),
        }
    }

    /// Write `size` bytes from `src` to `target` following the safety rules above.
    pub async fn write_safe<Rd>(
        &self,
        target: &str,
        src: &mut Rd,
        size: u64,
        o: &WriteOpts,
        ctl: Option<&Ctl>,
    ) -> R<()>
    where
        Rd: AsyncRead + Unpin,
    {
        if !target.starts_with('/') {
            return Err(XferError::Msg(format!("Invalid path: {target}")));
        }
        let (real, old) = self.resolve_target(target).await?;
        if let Some(m) = &old {
            if !o.overwrite {
                return Err(XferError::Exists);
            }
            if !m.file_type().is_file() {
                return Err(XferError::Msg(
                    "The target is not a regular file (a folder or a special file): the write was cancelled.".into(),
                ));
            }
            if !o.allow_foreign
                && !o.in_place
                && m.uid.is_some()
                && self.home_uid.is_some()
                && m.uid != self.home_uid
            {
                return Err(XferError::ForeignOwner);
            }
        }

        if let (true, Some(_)) = (o.in_place, &old) {
            let mut f = self
                .sftp
                .open_with_flags(real.clone(), OpenFlags::WRITE | OpenFlags::TRUNCATE)
                .await
                .map_err(sftp_err)?;
            pump(src, &mut f, ctl).await?;
            f.shutdown().await.map_err(|e| io_err("Write", e))?;
            let got = self
                .sftp
                .metadata(real.clone())
                .await
                .map_err(sftp_err)?
                .size;
            if got != Some(size) {
                return Err(XferError::Msg(
                    "The file size after writing did not match; the file on the server may be damaged, save again.".into(),
                ));
            }
            return Ok(());
        }

        let dir = remote_path::parent(&real);
        let name = remote_path::file_name(&real).to_string();
        let tmp = join(&dir, &format!(".{name}.d256shell-{}.part", rand_id()));
        if let Err(e) = self.fill_tmp(&tmp, src, size, old.as_ref(), o, ctl).await {
            self.drop_remote_tmp(&tmp).await;
            return Err(e);
        }
        if let Some(c) = ctl {
            if let Err(e) = c.check() {
                self.drop_remote_tmp(&tmp).await;
                return Err(e);
            }
        }

        if old.is_none() {
            // plain rename never overwrites: a file created meanwhile is not lost
            if let Err(e) = self.sftp.rename(tmp.clone(), real.clone()).await {
                self.drop_remote_tmp(&tmp).await;
                return Err(sftp_err(e));
            }
            return Ok(());
        }
        let bak = join(&dir, &format!(".{name}.d256shell-{}.bak", rand_id()));
        if let Err(e) = self.sftp.rename(real.clone(), bak.clone()).await {
            self.drop_remote_tmp(&tmp).await;
            return Err(sftp_err(e));
        }
        if let Err(e) = self.sftp.rename(tmp.clone(), real.clone()).await {
            // put the original back
            let restored = self.sftp.rename(bak.clone(), real.clone()).await.is_ok();
            self.drop_remote_tmp(&tmp).await;
            let mut msg = describe(e);
            if !restored {
                msg.push_str(&format!(" The original was saved as “{bak}”."));
            }
            return Err(XferError::Msg(msg));
        }
        if self.sftp.remove_file(bak.clone()).await.is_err() {
            self.stale.lock().expect("lock").push(bak);
        }
        Ok(())
    }

    async fn fill_tmp<Rd>(
        &self,
        tmp: &str,
        src: &mut Rd,
        size: u64,
        old: Option<&Metadata>,
        o: &WriteOpts,
        ctl: Option<&Ctl>,
    ) -> R<()>
    where
        Rd: AsyncRead + Unpin,
    {
        let mut f = self
            .sftp
            .open_with_flags(
                tmp.to_string(),
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .map_err(sftp_err)?;
        pump(src, &mut f, ctl).await?;
        f.shutdown().await.map_err(|e| io_err("Write", e))?;
        let got = self
            .sftp
            .metadata(tmp.to_string())
            .await
            .map_err(sftp_err)?
            .size;
        if got != Some(size) {
            return Err(XferError::Msg(format!(
                "Size on the server ({}) did not match the local one ({size}): the transfer was cancelled, the original is untouched.",
                got.map_or("?".to_string(), |g| g.to_string())
            )));
        }
        let mut a = FileAttributes::empty();
        a.permissions = old.and_then(|m| m.permissions).map(|p| p & 0o7777);
        if let Some(t) = o.mtime {
            a.atime = Some(t);
            a.mtime = Some(t);
        }
        if a.permissions.is_some() || a.mtime.is_some() {
            // best effort: some servers refuse setstat, the content is what matters
            let _ = self.sftp.set_metadata(tmp.to_string(), a).await;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ upload

    /// Upload one local file to `remote_dir/name`, honouring the conflict action. Returns false if skipped.
    pub async fn upload_file(
        &self,
        local: &Path,
        remote_dir: &str,
        name: &str,
        action: Action,
        keep_mtime: bool,
        ctl: &Ctl,
    ) -> R<bool> {
        self.upload_file_with(
            local,
            remote_dir,
            name,
            action,
            keep_mtime,
            ctl,
            (false, false),
        )
        .await
    }

    /// `risk` = (replace a file of another owner, write in place instead of atomically).
    pub async fn upload_file_with(
        &self,
        local: &Path,
        remote_dir: &str,
        name: &str,
        action: Action,
        keep_mtime: bool,
        ctl: &Ctl,
        risk: (bool, bool),
    ) -> R<bool> {
        if !remote_path::is_valid_name(name) {
            return Err(XferError::Msg(format!("Invalid name: {name}")));
        }
        let mut target_name = name.to_string();
        let mut o = WriteOpts {
            allow_foreign: risk.0,
            in_place: risk.1,
            ..WriteOpts::default()
        };
        if self.exists_lstat(&join(remote_dir, name)).await?.is_some() {
            match action {
                Action::Skip => return Ok(false),
                Action::Replace => o.overwrite = true,
                Action::Rename => target_name = self.free_numbered(remote_dir, name).await?,
            }
        }
        ctl.set_current(name);
        let meta = tokio::fs::metadata(local)
            .await
            .map_err(|e| io_err(&local.display().to_string(), e))?;
        if !meta.is_file() {
            return Err(XferError::Msg(format!(
                "{}: not a regular file.",
                local.display()
            )));
        }
        if keep_mtime {
            o.mtime = meta.modified().ok().and_then(secs);
        }
        let mut f = tokio::fs::File::open(local)
            .await
            .map_err(|e| io_err(&local.display().to_string(), e))?;
        self.write_safe(
            &join(remote_dir, &target_name),
            &mut f,
            meta.len(),
            &o,
            Some(ctl),
        )
        .await?;
        Ok(true)
    }

    /// Total size and file count of a local tree (symlinks followed, loops cut).
    pub fn local_totals(path: &Path) -> (u64, u64) {
        fn walk(p: &Path, seen: &mut HashSet<PathBuf>, depth: usize, acc: &mut (u64, u64)) {
            let Ok(m) = std::fs::metadata(p) else { return };
            if m.is_file() {
                acc.0 += m.len();
                acc.1 += 1;
            } else if m.is_dir() && depth < MAX_DEPTH {
                let Ok(real) = std::fs::canonicalize(p) else {
                    return;
                };
                if !seen.insert(real) {
                    return;
                }
                if let Ok(rd) = std::fs::read_dir(p) {
                    for e in rd.flatten() {
                        walk(&e.path(), seen, depth + 1, acc);
                    }
                }
            }
        }
        let mut acc = (0, 0);
        walk(path, &mut HashSet::new(), 0, &mut acc);
        acc
    }

    /// Upload a local folder into `remote_dir` (merging with an existing folder of the same name).
    pub fn upload_tree<'a>(
        &'a self,
        local: &'a Path,
        remote_dir: &'a str,
        name: &'a str,
        action: Action,
        keep_mtime: bool,
        ctl: &'a Ctl,
        seen: &'a mut HashSet<PathBuf>,
        depth: usize,
        out: &'a mut Outcome,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = R<()>> + Send + 'a>> {
        Box::pin(async move {
            ctl.check()?;
            if depth > MAX_DEPTH {
                return Err(XferError::Msg("Nesting is too deep.".into()));
            }
            let real = std::fs::canonicalize(local)
                .map_err(|e| io_err(&local.display().to_string(), e))?;
            if !seen.insert(real.clone()) {
                out.skipped.push(format!("{}: link loop", local.display()));
                return Ok(());
            }
            let dest = join(remote_dir, name);
            match self.exists_lstat(&dest).await? {
                Some(m) if m.file_type().is_dir() => {}
                Some(m) if m.file_type().is_symlink() => {
                    // a link to a folder is a valid merge target; anything else is not
                    let ok = self
                        .sftp
                        .metadata(dest.clone())
                        .await
                        .map(|m| m.file_type().is_dir())
                        .unwrap_or(false);
                    if !ok {
                        return Err(XferError::Msg(format!(
                            "“{name}” on the server is not a folder."
                        )));
                    }
                }
                Some(_) => {
                    return Err(XferError::Msg(format!(
                        "“{name}” on the server is not a folder."
                    )))
                }
                None => self.sftp.create_dir(dest.clone()).await.map_err(sftp_err)?,
            }
            let mut entries: Vec<_> = std::fs::read_dir(local)
                .map_err(|e| io_err(&local.display().to_string(), e))?
                .flatten()
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let n = e.file_name().to_string_lossy().into_owned();
                let p = e.path();
                let Ok(m) = std::fs::metadata(&p) else {
                    out.skipped.push(format!("{}: unavailable", p.display()));
                    continue;
                };
                if !remote_path::is_valid_name(&n) || e.file_name().to_str().is_none() {
                    out.skipped
                        .push(format!("{}: invalid name for a server", p.display()));
                    continue;
                }
                if m.is_dir() {
                    self.upload_tree(&p, &dest, &n, action, keep_mtime, ctl, seen, depth + 1, out)
                        .await?;
                } else if m.is_file() {
                    if self
                        .upload_file(&p, &dest, &n, action, keep_mtime, ctl)
                        .await?
                    {
                        out.files += 1;
                    } else {
                        out.skipped.push(format!("{n}: already exists"));
                    }
                }
            }
            seen.remove(&real);
            Ok(())
        })
    }

    // ---------------------------------------------------------------- download

    /// Download one remote file into the local folder `dir` as `name` (via `.part`).
    pub async fn download_file(
        &self,
        remote: &str,
        dir: &Path,
        name: &str,
        overwrite: bool,
        keep_mtime: bool,
        ctl: &Ctl,
    ) -> R<()> {
        if !local_name_ok(name) {
            return Err(XferError::Msg(format!(
                "Name “{name}” cannot be used as a file name on Windows."
            )));
        }
        let dest = dir.join(name);
        if !overwrite && dest.symlink_metadata().is_ok() {
            return Err(XferError::Exists);
        }
        ctl.set_current(name);
        let rm = self
            .sftp
            .metadata(remote.to_string())
            .await
            .map_err(sftp_err)?;
        if !rm.file_type().is_file() {
            return Err(XferError::Msg(format!("“{name}” is not a regular file.")));
        }
        let size = rm.size.unwrap_or(0);
        let tmp = dir.join(format!(".{name}.d256shell-{}.part", rand_id()));
        let res: R<()> = async {
            let mut src = self.sftp.open(remote.to_string()).await.map_err(sftp_err)?;
            let mut dst = tokio::fs::File::create_new(&tmp)
                .await
                .map_err(|e| io_err(&tmp.display().to_string(), e))?;
            pump(&mut src, &mut dst, Some(ctl)).await?;
            dst.sync_all().await.map_err(|e| io_err("Write", e))?;
            drop(dst);
            let got = std::fs::metadata(&tmp)
                .map_err(|e| io_err("Check", e))?
                .len();
            if got != size {
                return Err(XferError::Msg(format!(
                    "File size ({got}) did not match the server one ({size}): the file was not saved."
                )));
            }
            ctl.check()?;
            if keep_mtime {
                if let Some(t) = rm.mtime {
                    let when = UNIX_EPOCH + std::time::Duration::from_secs(u64::from(t));
                    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&tmp) {
                        let _ = f.set_modified(when);
                    }
                }
            }
            std::fs::rename(&tmp, &dest).map_err(|e| io_err(&dest.display().to_string(), e))
        }
        .await;
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }

    /// (bytes, files) of a remote tree: links to folders are not entered, links to files count as files.
    pub fn remote_totals<'a>(
        &'a self,
        path: &'a str,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = R<(u64, u64)>> + Send + 'a>> {
        Box::pin(async move {
            let m = self
                .sftp
                .metadata(path.to_string())
                .await
                .map_err(sftp_err)?;
            if m.file_type().is_file() {
                return Ok((m.size.unwrap_or(0), 1));
            }
            if !m.file_type().is_dir() || depth > MAX_DEPTH {
                return Ok((0, 0));
            }
            let mut acc = (0u64, 0u64);
            for e in self
                .sftp
                .read_dir(path.to_string())
                .await
                .map_err(sftp_err)?
            {
                let n = e.file_name();
                if n.contains('\u{FFFD}') {
                    continue;
                }
                let child = join(path, &n);
                let ft = e.metadata().file_type();
                if ft.is_symlink() {
                    if let Ok(t) = self.sftp.metadata(child.clone()).await {
                        if t.file_type().is_file() {
                            acc.0 += t.size.unwrap_or(0);
                            acc.1 += 1;
                        }
                    }
                } else {
                    let (b, f) = self.remote_totals(&child, depth + 1).await?;
                    acc.0 += b;
                    acc.1 += f;
                }
            }
            Ok(acc)
        })
    }

    /// Download a remote folder into `local_parent/name`.
    pub fn download_tree<'a>(
        &'a self,
        remote: &'a str,
        local_parent: &'a Path,
        name: &'a str,
        action: Action,
        keep_mtime: bool,
        ctl: &'a Ctl,
        depth: usize,
        out: &'a mut Outcome,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = R<()>> + Send + 'a>> {
        Box::pin(async move {
            ctl.check()?;
            if depth > MAX_DEPTH {
                return Err(XferError::Msg("Nesting is too deep.".into()));
            }
            if !local_name_ok(name) {
                out.skipped.push(format!("{name}: invalid name on Windows"));
                return Ok(());
            }
            let dir = local_parent.join(name);
            std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir.display().to_string(), e))?;
            for e in self
                .sftp
                .read_dir(remote.to_string())
                .await
                .map_err(sftp_err)?
            {
                let n = e.file_name();
                if n.contains('\u{FFFD}') {
                    out.skipped.push(format!("{remote}/{n}: non-UTF-8 name"));
                    continue;
                }
                let child = join(remote, &n);
                let ft = e.metadata().file_type();
                let mut is_dir = ft.is_dir();
                if ft.is_symlink() {
                    match self.sftp.metadata(child.clone()).await {
                        Ok(t) if t.file_type().is_dir() => {
                            out.skipped
                                .push(format!("{child}: a link to a folder (not followed)"));
                            continue;
                        }
                        Ok(t) if t.file_type().is_file() => is_dir = false,
                        _ => {
                            out.skipped.push(format!("{child}: broken link"));
                            continue;
                        }
                    }
                } else if !ft.is_dir() && !ft.is_file() {
                    out.skipped.push(format!("{child}: special file"));
                    continue;
                }
                if is_dir {
                    self.download_tree(&child, &dir, &n, action, keep_mtime, ctl, depth + 1, out)
                        .await?;
                } else {
                    let (dest_name, overwrite) =
                        match (dir.join(&n).symlink_metadata().is_ok(), action) {
                            (false, _) => (n.clone(), false),
                            (true, Action::Skip) => {
                                out.skipped.push(format!("{n}: already exists"));
                                continue;
                            }
                            (true, Action::Replace) => (n.clone(), true),
                            (true, Action::Rename) => {
                                let mut k = 1;
                                loop {
                                    let c = numbered(&n, k);
                                    if dir.join(&c).symlink_metadata().is_err() {
                                        break (c, false);
                                    }
                                    k += 1;
                                }
                            }
                        };
                    self.download_file(&child, &dir, &dest_name, overwrite, keep_mtime, ctl)
                        .await?;
                    out.files += 1;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_names() {
        assert_eq!(numbered("a.txt", 1), "a (1).txt");
        assert_eq!(numbered("a.tar.gz", 2), "a.tar (2).gz");
        assert_eq!(numbered(".bashrc", 1), ".bashrc (1)");
        assert_eq!(numbered("noext", 3), "noext (3)");
    }

    #[test]
    fn windows_unsafe_local_names() {
        for ok in [
            "a.txt",
            "юникод.txt",
            "-dash",
            "with space.txt",
            "it's$weird",
            ".hidden",
        ] {
            assert!(windows_name_ok(ok), "{ok}");
        }
        for bad in [
            "", ".", "..", "a/b", "a\\b", "c:x", "q?.txt", "con", "NUL.txt", "com1", "lpt9.log",
            "trail.", "trail ", "a|b",
        ] {
            assert!(!windows_name_ok(bad), "{bad}");
        }
        assert!(windows_name_ok("com0"));
        // Linux accepts what Windows would refuse (but never a slash or NUL)
        if !cfg!(windows) {
            assert!(local_name_ok("a:b") && local_name_ok("trail.") && !local_name_ok("a/b"));
        }
    }
}
