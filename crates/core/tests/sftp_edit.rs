//! Editor backend against a real sshd: byte-exact round trips, conflicts, refusals.
mod common;
use common::*;
use d256shell_core::sftp::edit::{Base, Eol};
use d256shell_core::sftp::SftpConnection;
use tokio::io::AsyncWriteExt;

macro_rules! need_servers {
    () => {
        if !enabled() {
            return;
        }
    };
}

const LIMIT: u64 = 5 * 1024 * 1024;

async fn connect(name: &str) -> SftpConnection {
    let d = tmp(name);
    let (i, _) = interaction(true, |_| None);
    SftpConnection::connect(params("sshd-password", Some(PASS), &d.join("kh")), i)
        .await
        .expect("connect")
}

async fn mk(c: &SftpConnection, path: &str, data: &[u8]) {
    let mut f = c.sftp.create(path.to_string()).await.unwrap();
    f.write_all(data).await.unwrap();
    f.shutdown().await.unwrap();
}

async fn workdir(c: &SftpConnection, tag: &str) -> String {
    let name = format!("edit-{tag}-{}", std::process::id());
    let _ = c.delete_entry(&format!("{}/{name}", c.home)).await;
    c.mkdir(&c.home, &name).await.unwrap()
}

fn base_of(v: &serde_json::Value) -> Base {
    Base::from_json(&v["base"]).expect("base")
}

fn eol_of(v: &serde_json::Value) -> Eol {
    match v["eol"].as_str().unwrap() {
        "crlf" => Eol::Crlf,
        _ => Eol::Lf,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn open_and_save_without_edits_keeps_every_byte() {
    need_servers!();
    let c = connect("e1").await;
    let w = workdir(&c, "rt").await;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("lf.txt", b"line1\nline2\n".to_vec()),
        ("crlf.txt", b"a\r\nb\r\nc\r\n".to_vec()),
        ("bom.txt", "\u{feff}с BOM\n".as_bytes().to_vec()),
        ("bom-crlf.txt", "\u{feff}x\r\ny".as_bytes().to_vec()),
        ("nonl.txt", b"no final newline".to_vec()),
        ("uni.txt", "юникод ✓ 日本語 🙂\n".as_bytes().to_vec()),
        ("empty.txt", Vec::new()),
    ];
    for (name, bytes) in cases {
        let p = format!("{w}/{name}");
        mk(&c, &p, &bytes).await;
        c.chmod_tree(&p, 0o640, false, true, false).await.unwrap();
        let o = c.edit_open(&p, LIMIT, false).await.ok().expect("open");
        let text = o["text"].as_str().unwrap().to_string();
        let bom = o["bom"].as_bool().unwrap();
        let saved = c
            .edit_save(
                &p,
                &text,
                bom,
                eol_of(&o),
                Some(&base_of(&o)),
                LIMIT,
                false,
                false,
                false,
            )
            .await
            .ok()
            .expect("save");
        assert_eq!(c.sftp.read(p.clone()).await.unwrap(), bytes, "{name}");
        let m = c.sftp.metadata(p.clone()).await.unwrap();
        assert_eq!(m.permissions.unwrap() & 0o7777, 0o640, "{name}: mode kept");
        // the new base matches the file: a second save needs no force
        let b2 = Base::from_json(&saved["base"]).unwrap();
        c.edit_save(
            &p,
            &text,
            bom,
            eol_of(&o),
            Some(&b2),
            LIMIT,
            false,
            false,
            false,
        )
        .await
        .ok()
        .expect("second save");
    }
    let left: Vec<_> = c
        .list_dir(&w)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.name.contains(".d256shell-"))
        .collect();
    assert!(left.is_empty());
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn edited_text_is_written_in_the_original_style() {
    need_servers!();
    let c = connect("e2").await;
    let w = workdir(&c, "style").await;
    let p = format!("{w}/f.txt");
    mk(&c, &p, "\u{feff}one\r\ntwo\r\n".as_bytes()).await;
    let o = c.edit_open(&p, LIMIT, false).await.ok().unwrap();
    assert_eq!(o["text"], "one\ntwo\n");
    assert_eq!(o["eol"], "crlf");
    c.edit_save(
        &p,
        "one\nTWO\nthree",
        true,
        Eol::Crlf,
        Some(&base_of(&o)),
        LIMIT,
        false,
        false,
        false,
    )
    .await
    .ok()
    .unwrap();
    assert_eq!(
        c.sftp.read(p.clone()).await.unwrap(),
        "\u{feff}one\r\nTWO\r\nthree".as_bytes()
    );
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_binary_big_and_non_utf8_files() {
    need_servers!();
    let c = connect("e3").await;
    let zone = format!("{}/zone", c.home);
    let e = c
        .edit_open(&format!("{zone}/binary.bin"), LIMIT, false)
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "binary");
    let w = workdir(&c, "refuse").await;
    mk(&c, &format!("{w}/latin1.txt"), b"caf\xe9\n").await;
    // a Latin-1 file is no longer refused: it opens in a guessed legacy encoding
    let l = c
        .edit_open(&format!("{w}/latin1.txt"), LIMIT, false)
        .await
        .ok()
        .expect("latin1 opens");
    assert_eq!(l["guessed"], true);
    assert!(l["text"].as_str().unwrap().starts_with("caf"));
    mk(&c, &format!("{w}/big.txt"), &vec![b'a'; 2 * 1024 * 1024]).await;
    let e = c
        .edit_open(&format!("{w}/big.txt"), 1024 * 1024, false)
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "too_large");
    assert_eq!(
        c.edit_open(&w, LIMIT, false).await.err().unwrap().code,
        "not_file"
    );
    // real CRLF / BOM files from the test zone
    assert_eq!(
        c.edit_open(&format!("{zone}/crlf.txt"), LIMIT, false)
            .await
            .ok()
            .unwrap()["eol"],
        "crlf"
    );
    assert_eq!(
        c.edit_open(&format!("{zone}/bom.txt"), LIMIT, false)
            .await
            .ok()
            .unwrap()["bom"],
        true
    );
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn conflict_when_changed_outside_deleted_and_force() {
    need_servers!();
    let c = connect("e4").await;
    let w = workdir(&c, "conf").await;
    let p = format!("{w}/f.txt");
    mk(&c, &p, b"original\n").await;
    let o = c.edit_open(&p, LIMIT, false).await.ok().unwrap();
    let base = base_of(&o);

    // someone else changes the file (same size, different content: the hash must catch it)
    mk(&c, &p, b"ORIGINAL\n").await;
    let e = c
        .edit_save(
            &p,
            "mine\n",
            false,
            Eol::Lf,
            Some(&base),
            LIMIT,
            false,
            false,
            false,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "conflict");
    assert_eq!(
        e.details.unwrap()["remote_text"],
        "ORIGINAL\n",
        "remote text is offered for the diff"
    );
    assert_eq!(
        c.sftp.read(p.clone()).await.unwrap(),
        b"ORIGINAL\n",
        "nothing written"
    );
    c.edit_save(
        &p,
        "mine\n",
        false,
        Eol::Lf,
        Some(&base),
        LIMIT,
        true,
        false,
        false,
    )
    .await
    .ok()
    .unwrap();
    assert_eq!(c.sftp.read(p.clone()).await.unwrap(), b"mine\n");

    // deleted on the server
    let o = c.edit_open(&p, LIMIT, false).await.ok().unwrap();
    c.sftp.remove_file(p.clone()).await.unwrap();
    let e = c
        .edit_save(
            &p,
            "x\n",
            false,
            Eol::Lf,
            Some(&base_of(&o)),
            LIMIT,
            false,
            false,
            false,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "deleted");
    assert!(
        c.sftp.metadata(p.clone()).await.is_err(),
        "not recreated silently"
    );
    c.edit_save(
        &p,
        "x\n",
        false,
        Eol::Lf,
        Some(&base_of(&o)),
        LIMIT,
        true,
        false,
        false,
    )
    .await
    .ok()
    .unwrap();
    assert_eq!(c.sftp.read(p.clone()).await.unwrap(), b"x\n");
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn new_file_is_created_on_first_save_and_never_overwrites() {
    need_servers!();
    let c = connect("e5").await;
    let w = workdir(&c, "new").await;
    let p = format!("{w}/fresh.txt");
    let o = c.edit_open(&p, LIMIT, true).await.ok().unwrap();
    assert!(
        c.sftp.metadata(p.clone()).await.is_err(),
        "not created by opening"
    );
    assert!(o["base"].is_null());
    c.edit_save(
        &p,
        "hello\n",
        false,
        Eol::Lf,
        None,
        LIMIT,
        false,
        false,
        false,
    )
    .await
    .ok()
    .unwrap();
    assert_eq!(c.sftp.read(p.clone()).await.unwrap(), b"hello\n");
    assert_eq!(
        c.edit_open(&p, LIMIT, true).await.err().unwrap().code,
        "exists"
    );
    let e = c
        .edit_save(
            &p,
            "other\n",
            false,
            Eol::Lf,
            None,
            LIMIT,
            false,
            false,
            false,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "exists");
    assert_eq!(c.sftp.read(p).await.unwrap(), b"hello\n");
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn foreign_owner_and_permissions_give_clear_answers() {
    need_servers!();
    let c = connect("e6").await;
    let zone = format!("{}/zone", c.home);
    let p = format!("{zone}/other_owner.txt");
    let o = c.edit_open(&p, LIMIT, false).await.ok().unwrap();
    let before = c.sftp.read(p.clone()).await.unwrap();
    let e = c
        .edit_save(
            &p,
            "mine\n",
            false,
            Eol::Lf,
            Some(&base_of(&o)),
            LIMIT,
            false,
            false,
            false,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "foreign_owner");
    assert_eq!(e.details.unwrap()["can_in_place"], true);
    // in place is refused by the server (file is 644 owned by someone else): the message names the user
    let e = c
        .edit_save(
            &p,
            "mine\n",
            false,
            Eol::Lf,
            Some(&base_of(&o)),
            LIMIT,
            false,
            false,
            true,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "no_permission");
    assert!(
        e.message.contains("tester") && e.message.contains("other_owner.txt"),
        "{}",
        e.message
    );
    assert_eq!(c.sftp.read(p).await.unwrap(), before);

    // folder not writable, file writable: atomic replace impossible -> offer in place, then it works
    let w = workdir(&c, "perm").await;
    let f = format!("{w}/f.txt");
    mk(&c, &f, b"before\n").await;
    let o = c.edit_open(&f, LIMIT, false).await.ok().unwrap();
    c.chmod_tree(&w, 0o555, false, false, true).await.unwrap();
    let e = c
        .edit_save(
            &f,
            "after\n",
            false,
            Eol::Lf,
            Some(&base_of(&o)),
            LIMIT,
            false,
            false,
            false,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "dir_not_writable");
    assert_eq!(
        c.sftp.read(f.clone()).await.unwrap(),
        b"before\n",
        "untouched"
    );
    c.edit_save(
        &f,
        "after\n",
        false,
        Eol::Lf,
        Some(&base_of(&o)),
        LIMIT,
        false,
        false,
        true,
    )
    .await
    .ok()
    .unwrap();
    assert_eq!(c.sftp.read(f.clone()).await.unwrap(), b"after\n");
    c.chmod_tree(&w, 0o755, false, false, true).await.unwrap();
    let left: Vec<_> = c
        .list_dir(&w)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.name.contains(".d256shell-"))
        .collect();
    assert!(left.is_empty());
    c.delete_entry(&w).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_encodings_are_detected_edited_and_saved_in_the_same_encoding() {
    need_servers!();
    use d256shell_core::sftp::codec::{by_name, encode_raw};
    let c = connect("e7").await;
    let w = workdir(&c, "enc").await;
    let text = "# Настройки сервера\nпароль=секрет\nадрес=example.org\n";
    let b1251 = encode_raw(text, by_name("windows-1251").unwrap()).unwrap();
    let p = format!("{w}/.env");
    mk(&c, &p, &b1251).await;

    // auto-detect: shown correctly, encoding reported
    let o = c.edit_open(&p, LIMIT, false).await.ok().expect("open");
    assert_eq!(o["encoding"], "windows-1251", "{o}");
    assert_eq!(o["guessed"], true);
    assert_eq!(o["text"], text);
    // unchanged save keeps every byte
    c.edit_save_enc(
        &p,
        text,
        false,
        Eol::Lf,
        "windows-1251",
        Some(&base_of(&o)),
        LIMIT,
        false,
        false,
        false,
    )
    .await
    .ok()
    .unwrap();
    assert_eq!(c.sftp.read(p.clone()).await.unwrap(), b1251);

    // wrong guess fixed by the user: reopen in KOI8-R shows other text, in 1251 the right one
    let k = c
        .edit_open_enc(&p, LIMIT, false, Some("KOI8-R"))
        .await
        .ok()
        .expect("koi8");
    assert_ne!(k["text"], text);
    assert_eq!(
        c.edit_open_enc(&p, LIMIT, false, Some("windows-1251"))
            .await
            .ok()
            .unwrap()["text"],
        text
    );

    // edit and save: a character the encoding lacks is refused, nothing is written
    let before = c.sftp.read(p.clone()).await.unwrap();
    let e = c
        .edit_save_enc(
            &p,
            "цена 5 ß\n",
            false,
            Eol::Lf,
            "windows-1251",
            Some(&base_of(&o)),
            LIMIT,
            false,
            false,
            false,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "unmappable");
    assert!(e.message.contains('ß'), "{}", e.message);
    assert_eq!(c.sftp.read(p.clone()).await.unwrap(), before);

    // convert to UTF-8 and to UTF-16 (with BOM) by saving in another encoding
    c.edit_save_enc(
        &p,
        text,
        false,
        Eol::Lf,
        "UTF-8",
        Some(&base_of(&o)),
        LIMIT,
        false,
        false,
        false,
    )
    .await
    .ok()
    .unwrap();
    assert_eq!(c.sftp.read(p.clone()).await.unwrap(), text.as_bytes());
    let o = c.edit_open(&p, LIMIT, false).await.ok().unwrap();
    assert_eq!(o["encoding"], "UTF-8");
    assert_eq!(o["guessed"], false);
    c.edit_save_enc(
        &p,
        text,
        true,
        Eol::Crlf,
        "UTF-16LE",
        Some(&base_of(&o)),
        LIMIT,
        false,
        false,
        false,
    )
    .await
    .ok()
    .unwrap();
    let raw = c.sftp.read(p.clone()).await.unwrap();
    assert_eq!(&raw[..2], &[0xFF, 0xFE]);
    let o = c.edit_open(&p, LIMIT, false).await.ok().unwrap();
    assert_eq!(o["encoding"], "UTF-16LE");
    assert_eq!(o["bom"], true);
    assert_eq!(o["eol"], "crlf");
    assert_eq!(o["text"], text);

    // real binary data is still refused, a forced unknown name too
    let zone = format!("{}/zone", c.home);
    assert_eq!(
        c.edit_open(&format!("{zone}/binary.bin"), LIMIT, false)
            .await
            .err()
            .unwrap()
            .code,
        "binary"
    );
    assert_eq!(
        c.edit_open_enc(&p, LIMIT, false, Some("klingon"))
            .await
            .err()
            .unwrap()
            .code,
        "encoding"
    );
    c.delete_entry(&w).await.unwrap();
}
