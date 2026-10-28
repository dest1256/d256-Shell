//! Write operations of the file explorer. Every operation refuses to overwrite anything: a name clash
//! is an error (or, for copies, a new "name (копия)" name), so nothing is lost by a misclick.
use super::conn::{describe, SftpConnection};
use crate::remote_path::{self, is_same_or_inside, is_valid_name, join};
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type R<T> = Result<T, String>;

const CHUNK: usize = 128 * 1024;
const MAX_DEPTH: usize = 64;

fn bad(path: &str) -> String {
    format!("Недопустимый путь: {path}")
}

fn check_abs(path: &str) -> R<()> {
    if !path.starts_with('/') || path.contains('\0') {
        return Err(bad(path));
    }
    Ok(())
}

/// Never delete or move the root, or something so high up that it is clearly a mistake.
fn check_not_top(path: &str) -> R<()> {
    check_abs(path)?;
    let n = remote_path::normalize(path);
    if n == "/"
        || n.matches('/').count() < 2
            && matches!(
                n.as_str(),
                "/home"
                    | "/opt"
                    | "/srv"
                    | "/mnt"
                    | "/media"
                    | "/run"
                    | "/lib64"
                    | "/tmp"
                    | "/etc"
                    | "/usr"
                    | "/var"
                    | "/bin"
                    | "/sbin"
                    | "/lib"
                    | "/boot"
                    | "/root"
                    | "/dev"
                    | "/proc"
                    | "/sys"
            )
    {
        return Err(format!("Операция с «{n}» запрещена."));
    }
    Ok(())
}

impl SftpConnection {
    async fn exists(&self, path: &str) -> R<bool> {
        match self.sftp.symlink_metadata(path).await {
            Ok(_) => Ok(true),
            Err(russh_sftp::client::error::Error::Status(s))
                if s.status_code == russh_sftp::protocol::StatusCode::NoSuchFile =>
            {
                Ok(false)
            }
            Err(e) => Err(describe(e)),
        }
    }

    pub async fn mkdir(&self, parent: &str, name: &str) -> R<String> {
        check_abs(parent)?;
        if !is_valid_name(name) {
            return Err("Недопустимое имя.".into());
        }
        let p = join(parent, name);
        if self.exists(&p).await? {
            return Err(format!("«{name}» уже существует."));
        }
        self.sftp.create_dir(p.clone()).await.map_err(describe)?;
        Ok(p)
    }

    /// New empty file; fails if the name is taken (O_EXCL).
    pub async fn create_file(&self, parent: &str, name: &str) -> R<String> {
        check_abs(parent)?;
        if !is_valid_name(name) {
            return Err("Недопустимое имя.".into());
        }
        let p = join(parent, name);
        if self.exists(&p).await? {
            return Err(format!("«{name}» уже существует."));
        }
        let mut f = self
            .sftp
            .open_with_flags(
                p.clone(),
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .map_err(describe)?;
        f.shutdown().await.map_err(|e| e.to_string())?;
        Ok(p)
    }

    pub async fn rename_entry(&self, path: &str, new_name: &str) -> R<String> {
        check_not_top(path)?;
        if !is_valid_name(new_name) {
            return Err("Недопустимое имя.".into());
        }
        let to = join(&remote_path::parent(path), new_name);
        if to == path {
            return Ok(to);
        }
        if self.exists(&to).await? {
            return Err(format!("«{new_name}» уже существует."));
        }
        self.sftp.rename(path, to.clone()).await.map_err(describe)?;
        Ok(to)
    }

    /// Move into `dest_dir` (rename on the same server). Name clashes and moves into itself are refused.
    pub async fn move_entry(&self, path: &str, dest_dir: &str) -> R<String> {
        check_not_top(path)?;
        check_abs(dest_dir)?;
        if is_same_or_inside(dest_dir, path) {
            return Err("Нельзя переместить папку в саму себя.".into());
        }
        let name = remote_path::file_name(path);
        let to = join(dest_dir, name);
        if to == path {
            return Ok(to);
        }
        if self.exists(&to).await? {
            return Err(format!("«{name}» уже есть в папке назначения."));
        }
        self.sftp.rename(path, to.clone()).await.map_err(describe)?;
        Ok(to)
    }

    /// Free name in `dir`: `name`, `name (копия)`, `name (копия 2)` ... (extension is kept).
    async fn free_name(&self, dir: &str, name: &str) -> R<String> {
        if !self.exists(&join(dir, name)).await? {
            return Ok(join(dir, name));
        }
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        for n in 1..1000 {
            let cand = if n == 1 {
                format!("{stem} (копия){ext}")
            } else {
                format!("{stem} (копия {n}){ext}")
            };
            if !self.exists(&join(dir, &cand)).await? {
                return Ok(join(dir, &cand));
            }
        }
        Err("Не удалось подобрать свободное имя.".into())
    }

    /// Copy a file or folder tree into `dest_dir` (through this machine: SFTP has no server-side copy).
    /// Symlinks are recreated as symlinks, never followed. Returns the new path.
    pub async fn copy_entry(&self, path: &str, dest_dir: &str) -> R<String> {
        check_abs(path)?;
        check_abs(dest_dir)?;
        let name = remote_path::file_name(path);
        if is_same_or_inside(dest_dir, path)
            && remote_path::normalize(dest_dir) != remote_path::parent(path)
        {
            return Err("Нельзя скопировать папку внутрь самой себя.".into());
        }
        let to = self.free_name(dest_dir, name).await?;
        self.copy_tree(path, &to, 0).await?;
        Ok(to)
    }

    fn copy_tree<'a>(
        &'a self,
        from: &'a str,
        to: &'a str,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = R<()>> + Send + 'a>> {
        Box::pin(async move {
            if depth > MAX_DEPTH {
                return Err("Слишком глубокая вложенность.".into());
            }
            let m = self.sftp.symlink_metadata(from).await.map_err(describe)?;
            let ft = m.file_type();
            if ft.is_symlink() {
                let target = self.sftp.read_link(from).await.map_err(describe)?;
                self.sftp
                    // OpenSSH sftp-server takes (target, link) on the wire, russh-sftp passes them through
                    .symlink(target, to.to_string())
                    .await
                    .map_err(describe)?;
            } else if ft.is_dir() {
                self.sftp.create_dir(to).await.map_err(describe)?;
                for e in self.sftp.read_dir(from).await.map_err(describe)? {
                    let n = e.file_name();
                    if n.contains('\u{FFFD}') {
                        continue; // cannot be addressed safely
                    }
                    self.copy_tree(&join(from, &n), &join(to, &n), depth + 1)
                        .await?;
                }
                if let Some(mode) = m.permissions {
                    let mut a = FileAttributes::empty();
                    a.permissions = Some(mode & 0o7777);
                    let _ = self.sftp.set_metadata(to, a).await;
                }
            } else if ft.is_file() {
                let mut src = self.sftp.open(from).await.map_err(describe)?;
                let mut dst = self
                    .sftp
                    .open_with_flags(
                        to.to_string(),
                        OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                    )
                    .await
                    .map_err(describe)?;
                let mut buf = vec![0u8; CHUNK];
                loop {
                    let n = src.read(&mut buf).await.map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    dst.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
                }
                dst.shutdown().await.map_err(|e| e.to_string())?;
                if let Some(mode) = m.permissions {
                    let mut a = FileAttributes::empty();
                    a.permissions = Some(mode & 0o7777);
                    let _ = self.sftp.set_metadata(to, a).await;
                }
            } else {
                return Err("Специальные файлы копировать нельзя.".into());
            }
            Ok(())
        })
    }

    /// Delete a file, symlink (the link only) or a whole folder tree.
    pub async fn delete_entry(&self, path: &str) -> R<()> {
        check_not_top(path)?;
        // the home folder itself, and anything that contains it, can only be removed by hand in a terminal
        if is_same_or_inside(&self.home, path) {
            return Err(format!(
                "«{path}» — домашняя папка или её родитель: удалять такое можно только вручную в терминале."
            ));
        }
        self.delete_tree(path, 0).await
    }

    fn delete_tree<'a>(
        &'a self,
        path: &'a str,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = R<()>> + Send + 'a>> {
        Box::pin(async move {
            if depth > MAX_DEPTH {
                return Err("Слишком глубокая вложенность.".into());
            }
            let m = self.sftp.symlink_metadata(path).await.map_err(describe)?;
            if m.file_type().is_dir() {
                for e in self.sftp.read_dir(path).await.map_err(describe)? {
                    let n = e.file_name();
                    if n.contains('\u{FFFD}') {
                        return Err(format!(
                            "В «{path}» есть файл с не-UTF-8 именем; удалите его в терминале."
                        ));
                    }
                    self.delete_tree(&join(path, &n), depth + 1).await?;
                }
                self.sftp.remove_dir(path).await.map_err(describe)
            } else {
                self.sftp.remove_file(path).await.map_err(describe)
            }
        })
    }

    /// Count what a recursive delete would remove (by `lstat`: links are counted, never entered).
    pub async fn count_tree(&self, path: &str) -> R<(u64, u64)> {
        check_not_top(path)?;
        let mut acc = (0, 0);
        self.count_inner(path, 0, &mut acc).await?;
        Ok(acc)
    }

    fn count_inner<'a>(
        &'a self,
        path: &'a str,
        depth: usize,
        acc: &'a mut (u64, u64),
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = R<()>> + Send + 'a>> {
        Box::pin(async move {
            if depth > MAX_DEPTH {
                return Err("Слишком глубокая вложенность.".into());
            }
            let m = self.sftp.symlink_metadata(path).await.map_err(describe)?;
            if m.file_type().is_dir() {
                acc.1 += 1;
                for e in self.sftp.read_dir(path).await.map_err(describe)? {
                    self.count_inner(&join(path, &e.file_name()), depth + 1, acc)
                        .await?;
                }
            } else {
                acc.0 += 1;
            }
            Ok(())
        })
    }

    /// chmod, optionally recursive. `files` / `dirs` choose which kinds get `mode`; symlinks are never touched.
    pub async fn chmod_tree(
        &self,
        path: &str,
        mode: u32,
        recursive: bool,
        files: bool,
        dirs: bool,
    ) -> R<u64> {
        check_abs(path)?;
        if mode > 0o7777 {
            return Err("Недопустимые права.".into());
        }
        let mut n = 0;
        self.chmod_inner(path, mode, recursive, files, dirs, 0, &mut n)
            .await?;
        Ok(n)
    }

    #[allow(clippy::too_many_arguments)]
    fn chmod_inner<'a>(
        &'a self,
        path: &'a str,
        mode: u32,
        recursive: bool,
        files: bool,
        dirs: bool,
        depth: usize,
        n: &'a mut u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = R<()>> + Send + 'a>> {
        Box::pin(async move {
            if depth > MAX_DEPTH {
                return Err("Слишком глубокая вложенность.".into());
            }
            let m = self.sftp.symlink_metadata(path).await.map_err(describe)?;
            let ft = m.file_type();
            if ft.is_symlink() {
                return Ok(());
            }
            if (ft.is_dir() && dirs) || (ft.is_file() && files) {
                let mut a = FileAttributes::empty();
                a.permissions = Some(mode);
                self.sftp.set_metadata(path, a).await.map_err(describe)?;
                *n += 1;
            }
            if ft.is_dir() && recursive {
                for e in self.sftp.read_dir(path).await.map_err(describe)? {
                    let name = e.file_name();
                    if name.contains('\u{FFFD}') {
                        continue;
                    }
                    self.chmod_inner(
                        &join(path, &name),
                        mode,
                        recursive,
                        files,
                        dirs,
                        depth + 1,
                        n,
                    )
                    .await?;
                }
            }
            Ok(())
        })
    }
}
