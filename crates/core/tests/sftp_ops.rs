//! Explorer write operations against a real sshd. Everything happens in a fresh folder under $HOME.
mod common;
use common::*;
use d256shell_core::sftp::SftpConnection;

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

async fn names(c: &SftpConnection, dir: &str) -> Vec<String> {
    let mut v: Vec<String> = c
        .list_dir(dir)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn create_rename_copy_move_delete_chmod() {
    need_servers!();
    let c = connect("ops1").await;
    let base = format!("{}/ops-{}", c.home, std::process::id());
    c.mkdir(&c.home, &format!("ops-{}", std::process::id()))
        .await
        .unwrap();

    // create
    let f = c.create_file(&base, "a.txt").await.unwrap();
    assert!(c.create_file(&base, "a.txt").await.is_err(), "no overwrite");
    assert!(c.mkdir(&base, "a.txt").await.is_err());
    for bad in ["", ".", "..", "x/y", "a\nb"] {
        assert!(c.create_file(&base, bad).await.is_err(), "{bad:?}");
    }
    c.mkdir(&base, "sub dir").await.unwrap();
    c.create_file(&format!("{base}/sub dir"), "inner.txt")
        .await
        .unwrap();

    // rename: clash refused
    c.create_file(&base, "b.txt").await.unwrap();
    assert!(c.rename_entry(&f, "b.txt").await.is_err());
    let f = c.rename_entry(&f, "renamed — файл.txt").await.unwrap();
    assert!(f.ends_with("renamed — файл.txt"));

    // copy: file, folder tree, repeated copy gets a new name
    let c1 = c.copy_entry(&f, &base).await.unwrap();
    assert!(c1.ends_with("renamed — файл (copy).txt"), "{c1}");
    let c2 = c.copy_entry(&f, &base).await.unwrap();
    assert!(c2.ends_with("(copy 2).txt"), "{c2}");
    let d = c
        .copy_entry(&format!("{base}/sub dir"), &base)
        .await
        .unwrap();
    assert_eq!(names(&c, &d).await, vec!["inner.txt"]);
    assert!(
        c.copy_entry(&format!("{base}/sub dir"), &format!("{base}/sub dir/inner"))
            .await
            .is_err(),
        "folder into itself"
    );

    // move: into itself and clashes refused
    c.mkdir(&base, "target").await.unwrap();
    assert!(c
        .move_entry(&format!("{base}/target"), &format!("{base}/target"))
        .await
        .is_err());
    let moved = c.move_entry(&c1, &format!("{base}/target")).await.unwrap();
    assert!(moved.starts_with(&format!("{base}/target/")));
    assert!(c.move_entry(&c2, &format!("{base}/target")).await.is_ok());
    c.copy_entry(&f, &format!("{base}/target")).await.unwrap();
    assert!(
        c.move_entry(&f, &format!("{base}/target")).await.is_err(),
        "clash"
    );

    // chmod
    c.chmod_tree(&format!("{base}/b.txt"), 0o600, false, true, true)
        .await
        .unwrap();
    let e = c
        .list_dir(&base)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.name == "b.txt")
        .unwrap();
    assert_eq!(e.mode.unwrap() & 0o7777, 0o600);
    assert!(c
        .chmod_tree(&base, 0o77777, false, true, true)
        .await
        .is_err());

    // delete: tree, and a symlink only removes the link
    c.sftp
        .symlink("/protected".to_string(), format!("{base}/lnk"))
        .await
        .unwrap();
    c.delete_entry(&format!("{base}/lnk")).await.unwrap();
    assert!(
        c.list_dir("/protected").await.is_ok(),
        "link target untouched"
    );
    c.delete_entry(&base).await.unwrap();
    assert!(c.list_dir(&base).await.is_err());
    for top in ["/", "/etc", "relative"] {
        assert!(c.delete_entry(top).await.is_err(), "{top}");
    }
    c.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn copying_a_symlink_makes_a_symlink() {
    need_servers!();
    let c = connect("ops2").await;
    let name = format!("ops-link-{}", std::process::id());
    let base = c.mkdir(&c.home, &name).await.unwrap();
    c.sftp
        .symlink("/protected".to_string(), format!("{base}/lnk"))
        .await
        .unwrap();
    let copy = c.copy_entry(&format!("{base}/lnk"), &base).await.unwrap();
    assert_eq!(c.sftp.read_link(copy).await.unwrap(), "/protected");
    c.delete_entry(&base).await.unwrap();
    c.close().await;
}
