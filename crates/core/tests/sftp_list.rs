mod common;
use common::*;
use d256shell_core::sftp::{EntryKind, SftpConnection};
use std::time::{Duration, Instant};

async fn connect(name: &str) -> SftpConnection {
    let d = tmp(name);
    let (i, _) = interaction(true, |_| None);
    SftpConnection::connect(params("sshd-password", Some(PASS), &d.join("kh")), i)
        .await
        .expect("connect")
}

macro_rules! need_servers {
    () => {
        if !enabled() {
            return;
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_special_names_types_and_symlinks() {
    need_servers!();
    let c = connect("list").await;
    let zone = format!("{}/zone", c.home);
    let entries = c.list_dir(&zone).await.unwrap();
    let by = |n: &str| {
        entries.iter().find(|e| e.name == n).unwrap_or_else(|| {
            panic!(
                "missing {n:?} in {:?}",
                entries.iter().map(|e| &e.name).collect::<Vec<_>>()
            )
        })
    };

    for n in [
        "file with spaces.txt",
        "юникод-файл.txt",
        "-dash.txt",
        "it's $weird *name.txt",
    ] {
        let e = by(n);
        assert_eq!(e.kind, EntryKind::File, "{n}");
        assert!(!e.name_lossy, "{n}");
    }
    assert_eq!(by("dir with spaces").kind, EntryKind::Dir);
    assert_eq!(by("crlf.txt").size, 6);
    assert_eq!(by("nonl.txt").size, 10);

    let link = by("link_to_protected");
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(
        link.target_is_dir,
        Some(true),
        "symlink to a directory must be recognised"
    );
    assert_eq!(by("loop").kind, EntryKind::Symlink);

    // permissions and owner ids are reported
    assert_eq!(by("ro_dir").mode.unwrap() & 0o777, 0o555);
    let me = by("file with spaces.txt").uid.unwrap();
    assert_ne!(
        by("other_owner.txt").uid.unwrap(),
        me,
        "file of another owner must differ in uid"
    );
    c.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn non_utf8_name_is_flagged_and_does_not_break_the_listing() {
    need_servers!();
    let c = connect("lossy").await;
    let entries = c.list_dir(&format!("{}/zone", c.home)).await.unwrap();
    let lossy: Vec<_> = entries.iter().filter(|e| e.name_lossy).collect();
    assert_eq!(lossy.len(), 1, "exactly the 'bad\\xffname' file: {lossy:?}");
    assert!(lossy[0].name.starts_with("bad") && lossy[0].name.ends_with("name"));
    // the other entries are intact
    assert!(entries
        .iter()
        .any(|e| e.name == "юникод-файл.txt" && !e.name_lossy));
    c.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_ten_thousand_files_fast() {
    need_servers!();
    let c = connect("many").await;
    let t = Instant::now();
    let entries = c.list_dir(&format!("{}/zone/many", c.home)).await.unwrap();
    assert_eq!(entries.len(), 10_000);
    assert!(entries
        .iter()
        .all(|e| e.kind == EntryKind::File && !e.name_lossy));
    assert!(
        t.elapsed() < Duration::from_secs(20),
        "took {:?}",
        t.elapsed()
    );
    c.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_errors_are_human_readable() {
    need_servers!();
    let c = connect("errs").await;
    let e = c.list_dir("/definitely/not/here").await.unwrap_err();
    assert!(e.contains("Нет такого файла"), "{e}");
    let e = c.list_dir("/root").await.unwrap_err();
    assert!(e.contains("Нет прав"), "{e}");
    c.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn canonicalize_resolves_symlinks_and_dotdot() {
    need_servers!();
    let c = connect("canon").await;
    assert_eq!(
        c.canonicalize(&format!("{}/zone/link_to_protected", c.home))
            .await
            .unwrap(),
        "/protected"
    );
    assert_eq!(
        c.canonicalize(&format!("{}/zone/../zone", c.home))
            .await
            .unwrap(),
        format!("{}/zone", c.home)
    );
    c.close().await;
}
