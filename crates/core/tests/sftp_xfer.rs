//! Upload / download / delete rules (section 6.3) against a real sshd. After each test the work folder
//! must contain no `.d256shell-*` leftovers.
mod common;
use common::*;
use d256shell_core::sftp::xfer::{Action, Ctl, Outcome, WriteOpts, XferError};
use d256shell_core::sftp::SftpConnection;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

macro_rules! need_servers {
    () => {
        if !enabled() {
            return;
        }
    };
}

async fn connect(name: &str) -> Arc<SftpConnection> {
    let d = tmp(name);
    let (i, _) = interaction(true, |_| None);
    Arc::new(
        SftpConnection::connect(params("sshd-password", Some(PASS), &d.join("kh")), i)
            .await
            .expect("connect"),
    )
}

async fn workdir(c: &SftpConnection, tag: &str) -> String {
    let name = format!("xfer-{tag}-{}", std::process::id());
    let p = format!("{}/{name}", c.home);
    let _ = c.delete_entry(&p).await;
    c.mkdir(&c.home, &name).await.unwrap()
}

async fn assert_clean(c: &SftpConnection, dir: &str) {
    fn walk<'a>(
        c: &'a SftpConnection,
        d: &'a str,
        bad: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            for e in c.list_dir(d).await.unwrap() {
                if e.name.contains(".d256shell-") {
                    bad.push(format!("{d}/{}", e.name));
                }
                if e.kind == d256shell_core::sftp::EntryKind::Dir {
                    walk(c, &format!("{d}/{}", e.name), bad).await;
                }
            }
        })
    }
    let mut bad = Vec::new();
    walk(c, dir, &mut bad).await;
    assert!(bad.is_empty(), "leftovers: {bad:?}");
}

async fn remote_bytes(c: &SftpConnection, p: &str) -> Vec<u8> {
    c.sftp.read(p.to_string()).await.unwrap()
}

fn local(name: &str) -> PathBuf {
    let d = tmp(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 31 + i / 7) % 251) as u8).collect()
}

async fn put(
    c: &SftpConnection,
    l: &Path,
    rdir: &str,
    name: &str,
    action: Action,
) -> Result<bool, XferError> {
    c.upload_file(l, rdir, name, action, true, &Ctl::default())
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_new_replace_keeps_mode_rename_and_skip() {
    need_servers!();
    let c = connect("x1").await;
    let w = workdir(&c, "up").await;
    let l = local("x1-local");
    let data = pattern(300_000);
    std::fs::write(l.join("f.bin"), &data).unwrap();

    assert!(put(&c, &l.join("f.bin"), &w, "f.bin", Action::Skip)
        .await
        .unwrap());
    assert_eq!(remote_bytes(&c, &format!("{w}/f.bin")).await, data);
    // mtime of the local file is preserved
    let lm = std::fs::metadata(l.join("f.bin"))
        .unwrap()
        .modified()
        .unwrap();
    let rm = c
        .sftp
        .metadata(format!("{w}/f.bin"))
        .await
        .unwrap()
        .mtime
        .unwrap();
    assert_eq!(
        u64::from(rm),
        lm.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
    );

    // replace: content changes, mode of the old file survives
    c.chmod_tree(&format!("{w}/f.bin"), 0o640, false, true, true)
        .await
        .unwrap();
    std::fs::write(l.join("f.bin"), b"new content").unwrap();
    assert!(put(&c, &l.join("f.bin"), &w, "f.bin", Action::Replace)
        .await
        .unwrap());
    assert_eq!(
        remote_bytes(&c, &format!("{w}/f.bin")).await,
        b"new content"
    );
    let m = c.sftp.metadata(format!("{w}/f.bin")).await.unwrap();
    assert_eq!(m.permissions.unwrap() & 0o7777, 0o640);

    // skip leaves it alone, rename adds " (1)"
    std::fs::write(l.join("f.bin"), b"third").unwrap();
    assert!(!put(&c, &l.join("f.bin"), &w, "f.bin", Action::Skip)
        .await
        .unwrap());
    assert_eq!(
        remote_bytes(&c, &format!("{w}/f.bin")).await,
        b"new content"
    );
    assert!(put(&c, &l.join("f.bin"), &w, "f.bin", Action::Rename)
        .await
        .unwrap());
    assert_eq!(remote_bytes(&c, &format!("{w}/f (1).bin")).await, b"third");
    // empty file and a name with odd characters
    std::fs::write(l.join("e"), b"").unwrap();
    put(
        &c,
        &l.join("e"),
        &w,
        "it's $weird *name — юникод.txt",
        Action::Skip,
    )
    .await
    .unwrap();
    assert_eq!(
        remote_bytes(&c, &format!("{w}/it's $weird *name — юникод.txt")).await,
        b""
    );

    assert_clean(&c, &w).await;
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_through_symlink_keeps_the_link_and_refuses_folders() {
    need_servers!();
    let c = connect("x2").await;
    let w = workdir(&c, "link").await;
    let l = local("x2-local");
    std::fs::write(l.join("n.txt"), b"through the link").unwrap();

    mk(&c, &format!("{w}/real.txt"), b"old").await;
    c.sftp
        .symlink("real.txt".to_string(), format!("{w}/lnk.txt"))
        .await
        .unwrap();
    put(&c, &l.join("n.txt"), &w, "lnk.txt", Action::Replace)
        .await
        .unwrap();
    let m = c
        .sftp
        .symlink_metadata(format!("{w}/lnk.txt"))
        .await
        .unwrap();
    assert!(m.file_type().is_symlink(), "the link is still a link");
    assert_eq!(
        remote_bytes(&c, &format!("{w}/real.txt")).await,
        b"through the link"
    );

    // a folder (or a link to one) as the target: refused, nothing changed
    c.mkdir(&w, "adir").await.unwrap();
    let e = put(&c, &l.join("n.txt"), &w, "adir", Action::Replace)
        .await
        .unwrap_err();
    assert!(matches!(e, XferError::Msg(_)), "{e}");
    c.sftp
        .symlink("/protected".to_string(), format!("{w}/dirlink"))
        .await
        .unwrap();
    assert!(put(&c, &l.join("n.txt"), &w, "dirlink", Action::Replace)
        .await
        .is_err());
    // broken link
    c.sftp
        .symlink("nowhere".to_string(), format!("{w}/broken"))
        .await
        .unwrap();
    assert!(put(&c, &l.join("n.txt"), &w, "broken", Action::Replace)
        .await
        .is_err());

    assert_clean(&c, &w).await;
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_into_readonly_folder_fails_and_creates_nothing() {
    need_servers!();
    let c = connect("x3").await;
    let ro = format!("{}/zone/ro_dir", c.home);
    let l = local("x3-local");
    std::fs::write(l.join("n.txt"), b"x").unwrap();
    let e = put(&c, &l.join("n.txt"), &ro, "n.txt", Action::Skip)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("access rights"), "{e}");
    assert!(c.list_dir(&ro).await.unwrap().is_empty());
    assert_clean(&c, &format!("{}/zone", c.home)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn foreign_owner_needs_a_decision() {
    need_servers!();
    let c = connect("x4").await;
    let zone = format!("{}/zone", c.home);
    let orig = remote_bytes(&c, &format!("{zone}/other_owner.txt")).await;
    let mut src = std::io::Cursor::new(b"mine".to_vec());
    let o = WriteOpts {
        overwrite: true,
        ..Default::default()
    };
    let e = c
        .write_safe(&format!("{zone}/other_owner.txt"), &mut src, 4, &o, None)
        .await
        .unwrap_err();
    assert!(matches!(e, XferError::ForeignOwner), "{e:?}");
    assert_eq!(
        remote_bytes(&c, &format!("{zone}/other_owner.txt")).await,
        orig,
        "untouched"
    );
    // not allowed to write in place either (file belongs to otheruser, mode 644): clear error, still untouched
    let mut src = std::io::Cursor::new(b"mine".to_vec());
    let o = WriteOpts {
        overwrite: true,
        in_place: true,
        ..Default::default()
    };
    let e = c
        .write_safe(&format!("{zone}/other_owner.txt"), &mut src, 4, &o, None)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("access rights"), "{e}");
    assert_eq!(
        remote_bytes(&c, &format!("{zone}/other_owner.txt")).await,
        orig
    );
    assert_clean(&c, &zone).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_at_half_leaves_the_original_and_no_part_file() {
    need_servers!();
    let c = connect("x5").await;
    let w = workdir(&c, "cancel").await;
    let l = local("x5-local");
    let big = pattern(24 * 1024 * 1024);
    std::fs::write(l.join("big.bin"), &big).unwrap();
    mk(&c, &format!("{w}/big.bin"), b"ORIGINAL").await;

    let ctl = Arc::new(Ctl::default());
    ctl.total.store(big.len() as u64, Ordering::Relaxed);
    let watcher = {
        let ctl = ctl.clone();
        tokio::spawn(async move {
            while ctl.done.load(Ordering::Relaxed) < (12 * 1024 * 1024) {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            ctl.cancel.store(true, Ordering::Relaxed);
        })
    };
    let r = c
        .upload_file(
            &l.join("big.bin"),
            &w,
            "big.bin",
            Action::Replace,
            true,
            &ctl,
        )
        .await;
    watcher.await.unwrap();
    assert!(matches!(r, Err(XferError::Cancelled)), "{r:?}");
    assert_eq!(remote_bytes(&c, &format!("{w}/big.bin")).await, b"ORIGINAL");
    assert_clean(&c, &w).await;
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn connection_loss_at_half_keeps_the_original_and_the_part_is_removed_after_reconnect() {
    need_servers!();
    let c = connect("x6").await;
    let w = workdir(&c, "drop").await;
    let l = local("x6-local");
    let big = pattern(24 * 1024 * 1024);
    std::fs::write(l.join("big.bin"), &big).unwrap();
    mk(&c, &format!("{w}/big.bin"), b"ORIGINAL").await;

    let ctl = Arc::new(Ctl::default());
    let killer = {
        let (ctl, c) = (ctl.clone(), c.clone());
        tokio::spawn(async move {
            while ctl.done.load(Ordering::Relaxed) < (12 * 1024 * 1024) {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            c.disconnect().await;
        })
    };
    let r = c
        .upload_file(
            &l.join("big.bin"),
            &w,
            "big.bin",
            Action::Replace,
            true,
            &ctl,
        )
        .await;
    killer.await.unwrap();
    assert!(r.is_err(), "the transfer must fail when the link drops");

    // new connection: the original is intact; the .part is still there until the stale list is cleaned
    let c2 = connect("x6b").await;
    assert_eq!(
        remote_bytes(&c2, &format!("{w}/big.bin")).await,
        b"ORIGINAL"
    );
    let parts: Vec<_> = c2
        .list_dir(&w)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.name.ends_with(".part"))
        .collect();
    assert_eq!(
        parts.len(),
        1,
        "an orphaned .part is expected after a dead link"
    );
    let stale: Vec<String> = std::mem::take(&mut *c.stale.lock().unwrap());
    assert_eq!(stale.len(), 1, "the dead session remembered the leftover");
    c2.stale.lock().unwrap().extend(stale);
    c2.cleanup_stale().await;
    assert_clean(&c2, &w).await;
    c2.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn large_upload_and_download_roundtrip_is_exact() {
    need_servers!();
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let c = connect("x7").await;
    let w = workdir(&c, "big").await;
    let l = local("x7-local");
    // 96 MB by default; scripts/test-big.sh sets D256SHELL_BIG=1 for the 512 MB case of the spec
    let size: u64 = if std::env::var("D256SHELL_BIG").is_ok() {
        512 * 1024 * 1024
    } else {
        96 * 1024 * 1024
    };
    // write the local file in chunks while hashing it (nothing big is kept in memory)
    let chunk = pattern(1024 * 1024);
    let mut want = Sha256::new();
    {
        let mut f = std::fs::File::create(l.join("big.bin")).unwrap();
        let mut left = size;
        while left > 0 {
            let n = left.min(chunk.len() as u64) as usize;
            f.write_all(&chunk[..n]).unwrap();
            want.update(&chunk[..n]);
            left -= n as u64;
        }
    }
    let want = want.finalize();
    put(&c, &l.join("big.bin"), &w, "big.bin", Action::Skip)
        .await
        .unwrap();
    assert_eq!(
        c.sftp.metadata(format!("{w}/big.bin")).await.unwrap().size,
        Some(size)
    );
    let out = local("x7-out");
    let dl = |overwrite: bool| {
        let (c, w, out) = (c.clone(), w.clone(), out.clone());
        async move {
            c.download_file(
                &format!("{w}/big.bin"),
                &out,
                "big.bin",
                overwrite,
                true,
                &Ctl::default(),
            )
            .await
        }
    };
    dl(false).await.unwrap();
    let mut got = Sha256::new();
    let mut f = std::fs::File::open(out.join("big.bin")).unwrap();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        got.update(&buf[..n]);
    }
    assert_eq!(got.finalize(), want, "SHA-256 of the round trip");
    // conflict: refused without overwrite, allowed with it; no leftovers locally
    assert!(matches!(dl(false).await.unwrap_err(), XferError::Exists));
    dl(true).await.unwrap();
    let names: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["big.bin"]);
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn download_cancel_removes_the_part_and_keeps_the_old_file() {
    need_servers!();
    let c = connect("x8").await;
    let w = workdir(&c, "dcancel").await;
    mk(&c, &format!("{w}/f.bin"), &pattern(20 * 1024 * 1024)).await;
    let out = local("x8-out");
    std::fs::write(out.join("f.bin"), b"LOCAL OLD").unwrap();
    let ctl = Arc::new(Ctl::default());
    let watcher = {
        let ctl = ctl.clone();
        tokio::spawn(async move {
            while ctl.done.load(Ordering::Relaxed) < (8 * 1024 * 1024) {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            ctl.cancel.store(true, Ordering::Relaxed);
        })
    };
    let r = c
        .download_file(&format!("{w}/f.bin"), &out, "f.bin", true, true, &ctl)
        .await;
    watcher.await.unwrap();
    assert!(matches!(r, Err(XferError::Cancelled)), "{r:?}");
    assert_eq!(std::fs::read(out.join("f.bin")).unwrap(), b"LOCAL OLD");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1, "no .part left");
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn trees_deep_empty_and_cyclic_links() {
    need_servers!();
    let c = connect("x9").await;
    let w = workdir(&c, "tree").await;
    let l = local("x9-local");
    let root = l.join("proj");
    let mut deep = root.clone();
    for i in 0..20 {
        deep = deep.join(format!("level{i}"));
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("leaf.txt"), b"leaf").unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&root, root.join("loop")).unwrap(); // cycle: proj/loop -> proj

    let ctl = Ctl::default();
    let mut out = Outcome::default();
    let t0 = std::time::Instant::now();
    c.upload_tree(
        &root,
        &w,
        "proj",
        Action::Replace,
        true,
        &ctl,
        &mut HashSet::new(),
        0,
        &mut out,
    )
    .await
    .unwrap();
    assert!(t0.elapsed().as_secs() < 30, "no hang on the cycle");
    assert_eq!(out.files, 2);
    let mut rdeep = format!("{w}/proj");
    for i in 0..20 {
        rdeep = format!("{rdeep}/level{i}");
    }
    assert_eq!(
        remote_bytes(&c, &format!("{rdeep}/leaf.txt")).await,
        b"leaf"
    );
    assert!(c
        .list_dir(&format!("{w}/proj/empty"))
        .await
        .unwrap()
        .is_empty());
    #[cfg(unix)]
    assert!(
        out.skipped.iter().any(|s| s.contains("loop")),
        "{:?}",
        out.skipped
    );

    // a remote symlink to a folder is not walked on download (and a link to a file is read)
    c.sftp
        .symlink("/protected".to_string(), format!("{w}/proj/to_protected"))
        .await
        .unwrap();
    c.sftp
        .symlink("top.txt".to_string(), format!("{w}/proj/top_link.txt"))
        .await
        .unwrap();
    let dl = local("x9-dl");
    let mut out = Outcome::default();
    c.download_tree(
        &format!("{w}/proj"),
        &dl,
        "proj",
        Action::Replace,
        true,
        &ctl,
        0,
        &mut out,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(dl.join("proj/top_link.txt")).unwrap(), b"top");
    assert!(!dl.join("proj/to_protected").exists());
    assert!(out.skipped.iter().any(|s| s.contains("link to a folder")));
    assert!(dl.join("proj/empty").is_dir());
    let mut p = dl.join("proj");
    for i in 0..20 {
        p = p.join(format!("level{i}"));
    }
    assert_eq!(std::fs::read(p.join("leaf.txt")).unwrap(), b"leaf");

    // merging: uploading again with Skip changes nothing and reports skips
    let mut out = Outcome::default();
    c.upload_tree(
        &root,
        &w,
        "proj",
        Action::Skip,
        true,
        &ctl,
        &mut HashSet::new(),
        0,
        &mut out,
    )
    .await
    .unwrap();
    assert_eq!(out.files, 0);
    assert_clean(&c, &w).await;
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_does_not_follow_links_and_refuses_dangerous_paths() {
    need_servers!();
    let c = connect("x10").await;
    let w = workdir(&c, "del").await;
    let before_a = remote_bytes(&c, "/protected/a.txt").await;
    let before_b = remote_bytes(&c, "/protected/b.txt").await;
    c.mkdir(&w, "tree").await.unwrap();
    c.sftp
        .symlink("/protected".to_string(), format!("{w}/tree/p"))
        .await
        .unwrap();
    mk(&c, &format!("{w}/tree/f"), b"x").await;
    assert_eq!(
        c.count_tree(&format!("{w}/tree")).await.unwrap(),
        (2, 1),
        "link counted as one object"
    );
    c.delete_entry(&format!("{w}/tree")).await.unwrap();
    assert_eq!(remote_bytes(&c, "/protected/a.txt").await, before_a);
    assert_eq!(remote_bytes(&c, "/protected/b.txt").await, before_b);
    assert_eq!(c.list_dir("/protected").await.unwrap().len(), 2);

    for bad in [
        "/", "", "relative", "/etc", "/usr", "/var", "/home", "/root", "/boot", "/bin", &c.home,
        "/home/",
    ] {
        assert!(
            c.delete_entry(bad).await.is_err(),
            "{bad:?} must be refused"
        );
    }
    // permissions: stops with an error, the read-only folder's content stays
    let e = c.delete_entry(&format!("{}/zone/ro_dir", c.home)).await;
    assert!(e.is_err());
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn chmod_recursive_by_kind_leaves_symlinks_alone() {
    need_servers!();
    let c = connect("x11").await;
    let w = workdir(&c, "chmod").await;
    c.mkdir(&w, "d").await.unwrap();
    c.mkdir(&format!("{w}/d"), "sub").await.unwrap();
    mk(&c, &format!("{w}/d/f"), b"x").await;
    mk(&c, &format!("{w}/d/sub/g"), b"x").await;
    c.sftp
        .symlink("/protected".to_string(), format!("{w}/d/p"))
        .await
        .unwrap();
    let mode = |p: String, c: Arc<SftpConnection>| async move {
        c.sftp.metadata(p).await.unwrap().permissions.unwrap() & 0o7777
    };
    let prot_before = mode("/protected".into(), c.clone()).await;

    // only files, recursive
    let n = c
        .chmod_tree(&format!("{w}/d"), 0o600, true, true, false)
        .await
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(mode(format!("{w}/d/f"), c.clone()).await, 0o600);
    assert_eq!(mode(format!("{w}/d/sub/g"), c.clone()).await, 0o600);
    assert_ne!(mode(format!("{w}/d/sub"), c.clone()).await, 0o600);
    // only folders
    c.chmod_tree(&format!("{w}/d"), 0o750, true, false, true)
        .await
        .unwrap();
    assert_eq!(mode(format!("{w}/d/sub"), c.clone()).await, 0o750);
    assert_eq!(mode(format!("{w}/d/f"), c.clone()).await, 0o600);
    assert_eq!(
        mode("/protected".into(), c.clone()).await,
        prot_before,
        "the link target was not touched"
    );
    // non recursive: only the item itself
    assert_eq!(
        c.chmod_tree(&format!("{w}/d/f"), 0o644, false, true, true)
            .await
            .unwrap(),
        1
    );
    c.chmod_tree(&format!("{w}/d"), 0o755, true, true, true)
        .await
        .unwrap(); // restore so cleanup works
    c.delete_entry(&w).await.unwrap();
}

/// Create (or truncate) a remote file with the given content. `SftpSession::write` only writes to existing files.
async fn mk<D: AsRef<[u8]>>(c: &SftpConnection, path: &str, data: D) {
    use tokio::io::AsyncWriteExt;
    let mut f = c.sftp.create(path.to_string()).await.unwrap();
    f.write_all(data.as_ref()).await.unwrap();
    f.shutdown().await.unwrap();
}
