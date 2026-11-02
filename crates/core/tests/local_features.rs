//! Local features through `Service::call` (the API the UI uses): notes in the vault, scripts (own files and links),
//! local shells, icons. No SSH server is needed.
mod common;
use base64ct::{Base64, Encoding};
use common::tmp;
use d256shell_core::paths;
use d256shell_core::service::Service;
use d256shell_core::vault::KdfParams;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const KDF: KdfParams = KdfParams {
    m_cost_kib: 64,
    t_cost: 1,
    p_cost: 1,
};

fn service(name: &str) -> (Service, std::path::PathBuf) {
    let root = tmp(name);
    let svc = Service::with_kdf(paths::prepare(root.clone()).unwrap(), KDF);
    svc.call("create_vault", json!({"master": "long-enough-pw"}))
        .unwrap();
    (svc, root)
}

fn call(svc: &Service, m: &str, p: Value) -> Value {
    svc.call(m, p)
        .unwrap_or_else(|e| panic!("{m}: {}", e.message))
}

fn read_until(svc: &Service, term: &str, needle: &str) -> String {
    let t = Instant::now();
    let mut all = String::new();
    while t.elapsed() < Duration::from_secs(15) {
        let r = call(svc, "term_read", json!({"term_id": term, "wait_ms": 300}));
        if let Some(d) = r["data"].as_str() {
            all.push_str(&String::from_utf8_lossy(&Base64::decode_vec(d).unwrap()));
            if all.contains(needle) {
                return all;
            }
        }
        if !r["exit"].is_null() {
            break;
        }
    }
    panic!("did not see {needle:?} in the terminal output: {all:?}");
}

#[test]
fn notes_live_in_the_vault_and_survive_lock_and_unlock() {
    let (svc, root) = service("notes");
    assert_eq!(
        call(&svc, "note_list", json!({}))["notes"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let a = call(
        &svc,
        "note_save",
        json!({"title": "VPS passwords", "text": "root: s3cret ключ"}),
    );
    let id = a["id"].as_str().unwrap().to_string();
    let b = call(
        &svc,
        "note_save",
        json!({"title": "  ", "text": "no title"}),
    );
    let id_b = b["id"].as_str().unwrap().to_string();
    assert_eq!(
        call(&svc, "note_get", json!({"id": id_b}))["title"],
        "Untitled"
    );
    // editing keeps the id and replaces the text
    call(
        &svc,
        "note_save",
        json!({"id": id, "title": "VPS", "text": "new text"}),
    );
    let g = call(&svc, "note_get", json!({"id": id}));
    assert_eq!(
        (g["title"].as_str(), g["text"].as_str()),
        (Some("VPS"), Some("new text"))
    );
    assert_eq!(
        call(&svc, "note_list", json!({}))["notes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // encrypted at rest: the text is not in vault.bin
    let raw = std::fs::read(root.join("vault.bin")).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("new text"));
    // survives a lock/unlock cycle
    call(&svc, "lock", json!({}));
    assert!(
        svc.call("note_list", json!({})).is_err(),
        "locked vault: no notes"
    );
    call(&svc, "unlock", json!({"master": "long-enough-pw"}));
    assert_eq!(
        call(&svc, "note_get", json!({"id": id}))["text"],
        "new text"
    );
    call(&svc, "note_delete", json!({"id": id}));
    let left = call(&svc, "note_list", json!({}));
    assert_eq!(left["notes"].as_array().unwrap().len(), 1);
    assert!(svc.call("note_get", json!({"id": id})).is_err());
}

#[test]
fn own_scripts_are_files_in_data_scripts_with_exact_bytes() {
    let (svc, root) = service("scripts-own");
    for bad in [
        "../x.sh",
        "a/b.sh",
        "noext",
        "x.exe",
        ".hidden.sh",
        "a:b.sh",
        "",
    ] {
        assert!(
            svc.call("script_create", json!({"name": bad, "text": ""}))
                .is_err(),
            "{bad}"
        );
    }
    let id = call(
        &svc,
        "script_create",
        json!({"name": "hello.sh", "text": "echo hi\n"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(id, "own:hello.sh");
    assert!(
        svc.call("script_create", json!({"name": "hello.sh", "text": "x"}))
            .is_err(),
        "never overwrites"
    );
    assert_eq!(
        std::fs::read(root.join("scripts/hello.sh")).unwrap(),
        b"echo hi\n"
    );

    // CRLF + BOM are kept byte for byte through read -> write
    std::fs::write(
        root.join("scripts/win.ps1"),
        b"\xEF\xBB\xBFWrite-Host 1\r\nWrite-Host 2\r\n",
    )
    .unwrap();
    let r = call(&svc, "script_read", json!({"id": "own:win.ps1"}));
    assert_eq!(
        (r["eol"].as_str(), r["bom"].as_bool()),
        (Some("crlf"), Some(true))
    );
    assert_eq!(r["text"], "Write-Host 1\nWrite-Host 2\n");
    call(
        &svc,
        "script_write",
        json!({"id": "own:win.ps1", "text": r["text"], "eol": r["eol"], "bom": r["bom"], "encoding": r["encoding"]}),
    );
    assert_eq!(
        std::fs::read(root.join("scripts/win.ps1")).unwrap(),
        b"\xEF\xBB\xBFWrite-Host 1\r\nWrite-Host 2\r\n"
    );

    // a Windows-1251 batch file is not damaged
    let cp1251: Vec<u8> = vec![
        b'@', b'e', b'c', b'h', b'o', b' ', 0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, b'\r', b'\n',
    ];
    std::fs::write(root.join("scripts/old.bat"), &cp1251).unwrap();
    let r = call(&svc, "script_read", json!({"id": "own:old.bat"}));
    assert_eq!(r["text"], "@echo Привет\n");
    call(
        &svc,
        "script_write",
        json!({"id": "own:old.bat", "text": r["text"], "eol": r["eol"], "bom": r["bom"], "encoding": r["encoding"]}),
    );
    assert_eq!(std::fs::read(root.join("scripts/old.bat")).unwrap(), cp1251);

    let list = call(&svc, "scripts_list", json!({}));
    let names: Vec<&str> = list["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["hello.sh", "old.bat", "win.ps1"]);

    // no temp files left behind
    let left: Vec<String> = std::fs::read_dir(root.join("scripts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(left.iter().all(|n| !n.contains("d256tmp")), "{left:?}");

    call(&svc, "script_delete", json!({"id": "own:hello.sh"}));
    assert!(!root.join("scripts/hello.sh").exists());
    assert!(
        svc.call("script_read", json!({"id": "own:../vault.bin"}))
            .is_err(),
        "no way out of the scripts folder"
    );
}

#[test]
fn linked_scripts_store_only_the_path_and_never_touch_the_file() {
    let (svc, root) = service("scripts-link");
    let outside = tmp("scripts-link-src");
    let good = outside.join("tool.sh");
    std::fs::write(&good, "echo linked\n").unwrap();
    let not_script = outside.join("notes.txt");
    std::fs::write(&not_script, "x").unwrap();
    let r = call(
        &svc,
        "script_link_add",
        json!({"paths": [good.to_string_lossy(), not_script.to_string_lossy(), outside.join("nope.sh").to_string_lossy()]}),
    );
    assert_eq!(r["added"], 1);
    assert_eq!(r["skipped"].as_array().unwrap().len(), 2);
    call(
        &svc,
        "script_link_add",
        json!({"paths": [good.to_string_lossy()]}),
    );
    let list = call(&svc, "scripts_list", json!({}));
    let links: Vec<&Value> = list["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] == "link")
        .collect();
    assert_eq!(links.len(), 1, "added twice, listed once");
    assert_eq!(links[0]["exists"], true);
    assert!(
        !root.join("scripts").join("tool.sh").exists(),
        "the file is not copied"
    );

    // saved in settings.json, so it survives a restart
    let saved = std::fs::read_to_string(root.join("settings.json")).unwrap();
    assert!(saved.contains("tool.sh"));

    // the file goes away: the entry stays, grey; "locate" repoints it
    let id = format!("link:{}", good.to_string_lossy());
    std::fs::rename(&good, outside.join("moved.sh")).unwrap();
    let list = call(&svc, "scripts_list", json!({}));
    let l = list["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "link")
        .unwrap()
        .clone();
    assert_eq!(l["exists"], false);
    assert!(svc.call("script_run", json!({"id": id})).is_err());
    let moved = outside.join("moved.sh");
    let new_id = call(
        &svc,
        "script_link_move",
        json!({"old": good.to_string_lossy(), "new": moved.to_string_lossy()}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        call(&svc, "script_read", json!({"id": new_id}))["text"],
        "echo linked\n"
    );

    // deleting a link removes the entry only
    call(&svc, "script_delete", json!({"id": new_id}));
    assert!(moved.exists(), "the real file stays");
    let list = call(&svc, "scripts_list", json!({}));
    assert!(list["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["kind"] != "link"));
    assert!(
        svc.call("script_read", json!({"id": "link:/etc/passwd"}))
            .is_err(),
        "only listed links can be read"
    );
}

#[cfg(unix)]
#[test]
fn local_shells_are_listed_and_run_in_a_terminal_and_scripts_run_in_theirs() {
    let (svc, root) = service("local-shell");
    let p = call(&svc, "shell_profiles", json!({}));
    let profiles = p["profiles"].as_array().unwrap();
    assert!(!profiles.is_empty(), "a Linux box has at least one shell");
    assert!(profiles
        .iter()
        .any(|p| p["name"] == "bash" || p["name"] == "sh"));
    assert_eq!(p["startup"], "home");
    assert!(svc
        .call(
            "local_term_open",
            json!({"profile_id": "auto:no-such-shell"})
        )
        .is_err());

    let sh = profiles
        .iter()
        .find(|p| p["name"] == "bash")
        .or_else(|| profiles.first())
        .unwrap();
    let t = call(
        &svc,
        "local_term_open",
        json!({"profile_id": sh["id"], "cols": 100, "rows": 30}),
    );
    let term = t["term_id"].as_str().unwrap().to_string();
    let data = Base64::encode_string(b"echo local-$((20+22))\n");
    call(
        &svc,
        "term_write",
        json!({"term_id": term, "data_b64": data}),
    );
    read_until(&svc, &term, "local-42");
    call(&svc, "term_close", json!({"term_id": term}));

    // a script runs in a terminal in its own folder
    call(
        &svc,
        "script_create",
        json!({"name": "where.sh", "text": "echo ran-in-$(basename \"$PWD\")\n"}),
    );
    let r = call(&svc, "script_run", json!({"id": "own:where.sh"}));
    let term = r["term_id"].as_str().unwrap().to_string();
    read_until(&svc, &term, "ran-in-scripts");
    call(&svc, "term_close", json!({"term_id": term}));
    assert!(root.join("scripts/where.sh").exists());
    // other types are refused with a reason
    call(
        &svc,
        "script_create",
        json!({"name": "x.bat", "text": "echo"}),
    );
    let e = svc
        .call("script_run", json!({"id": "own:x.bat"}))
        .unwrap_err();
    assert!(e.message.contains("Windows"), "{}", e.message);
}

#[test]
fn custom_profiles_and_the_startup_choice_come_from_settings() {
    let (svc, _root) = service("profiles");
    let prog = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
    call(
        &svc,
        "set_settings",
        json!({
            "shell_profiles": [{"id": "py", "name": "My shell", "program": prog, "args": [], "cwd": ""}, {"id": "", "name": "bad", "program": "x"}],
            "startup_profile": "custom:py", "tabs_position": "left", "icons": {"server:1": "linux"}
        }),
    );
    let p = call(&svc, "shell_profiles", json!({}));
    let mine: Vec<&Value> = p["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["custom"] == true)
        .collect();
    assert_eq!(mine.len(), 1, "the entry without an id is dropped");
    assert_eq!(mine[0]["id"], "custom:py");
    assert_eq!(p["startup"], "custom:py");
    let s = call(&svc, "get_settings", json!({}));
    assert_eq!(
        (s["tabs_position"].as_str(), s["icons"]["server:1"].as_str()),
        (Some("left"), Some("linux"))
    );
    call(&svc, "set_settings", json!({"tabs_position": "diagonal"}));
    assert_eq!(
        call(&svc, "get_settings", json!({}))["tabs_position"],
        "top"
    );
    #[cfg(unix)]
    {
        let bad = svc.call(
            "local_term_open",
            json!({"profile_id": "custom:py", "cwd": "/definitely/not/here"}),
        );
        assert!(
            bad.is_ok(),
            "the profile has no start folder: the shell starts"
        );
    }
}

#[test]
fn icons_are_copied_into_data_and_served_as_data_urls() {
    let (svc, root) = service("icons");
    let src = tmp("icons-src");
    let png = src.join("a b.png");
    let bytes = b"\x89PNG\r\n\x1a\nfake";
    std::fs::write(&png, bytes).unwrap();
    let name = call(&svc, "icon_import", json!({"path": png.to_string_lossy()}))["name"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(name.ends_with(".png"));
    assert_eq!(
        std::fs::read(root.join("icons").join(&name)).unwrap(),
        bytes
    );
    let url = call(&svc, "icon_get", json!({"name": name}))["data_url"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(url.starts_with("data:image/png;base64,"));
    for bad in ["../vault.bin", "a/b.png", "x.exe", ".png", ""] {
        assert!(svc.call("icon_get", json!({"name": bad})).is_err(), "{bad}");
    }
    let exe = src.join("x.exe");
    std::fs::write(&exe, "MZ").unwrap();
    assert!(svc
        .call("icon_import", json!({"path": exe.to_string_lossy()}))
        .is_err());
    let big = src.join("big.png");
    std::fs::write(&big, vec![0u8; 1024 * 1024 + 1]).unwrap();
    assert!(svc
        .call("icon_import", json!({"path": big.to_string_lossy()}))
        .is_err());
}

#[cfg(unix)]
#[test]
fn local_explorer_lists_changes_and_edits_files_through_the_service() {
    let (svc, _root) = service("lfs");
    let base = tmp("lfs-area");
    let b = base.to_string_lossy().into_owned();
    std::fs::write(base.join("note.txt"), "hello\n").unwrap();
    std::fs::write(base.join(".dot"), "").unwrap();

    let l = call(&svc, "lfs_list", json!({"path": b}));
    let names: Vec<&str> = l["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"note.txt") && names.contains(&".dot"),
        "hidden files are listed: {names:?}"
    );
    // no path = the home folder
    assert!(call(&svc, "lfs_list", json!({}))["path"]
        .as_str()
        .unwrap()
        .starts_with('/'));
    assert!(svc
        .call("lfs_list", json!({"path": "/definitely/not/here"}))
        .is_err());

    let made = call(
        &svc,
        "lfs",
        json!({"op": "mkdir", "parent": b, "name": "sub"}),
    );
    let sub = made["path"].as_str().unwrap().to_string();
    assert!(svc
        .call("lfs", json!({"op": "mkdir", "parent": b, "name": "sub"}))
        .is_err());
    call(
        &svc,
        "lfs",
        json!({"op": "touch", "parent": sub, "name": "new.txt"}),
    );
    let r = call(
        &svc,
        "lfs",
        json!({"op": "copy", "paths": [format!("{b}/note.txt")], "dest": sub}),
    );
    assert_eq!(r["errors"].as_array().unwrap().len(), 0);
    let r = call(
        &svc,
        "lfs",
        json!({"op": "copy", "paths": [format!("{b}/note.txt")], "dest": sub}),
    );
    assert!(r["done"][0].as_str().unwrap().ends_with("note (copy).txt"));
    let c = call(&svc, "lfs", json!({"op": "count", "paths": [sub]}));
    assert_eq!(
        (c["files"].as_u64(), c["dirs"].as_u64()),
        (Some(3), Some(1))
    );
    let r = call(
        &svc,
        "lfs",
        json!({"op": "move", "paths": [format!("{b}/note.txt"), "/definitely/not/here"], "dest": sub}),
    );
    assert_eq!(
        (
            r["done"].as_array().unwrap().len(),
            r["errors"].as_array().unwrap().len()
        ),
        (0, 2),
        "note.txt already exists there"
    );

    // a text file is edited with the same codec as scripts (exact bytes)
    let id = format!("file:{b}/note.txt");
    let t = call(&svc, "script_read", json!({"id": id}));
    assert_eq!(t["text"], "hello\n");
    call(
        &svc,
        "script_write",
        json!({"id": id, "text": "changed\n", "eol": t["eol"], "bom": t["bom"], "encoding": t["encoding"]}),
    );
    assert_eq!(
        std::fs::read_to_string(base.join("note.txt")).unwrap(),
        "changed\n"
    );
    assert!(
        svc.call("script_delete", json!({"id": id})).is_err(),
        "the explorer's files cannot be deleted through the script list"
    );
    assert!(base.join("note.txt").exists());

    let r = call(&svc, "lfs", json!({"op": "delete", "paths": [sub]}));
    assert_eq!(r["errors"].as_array().unwrap().len(), 0);
    assert!(!base.join("sub").exists());
    let r = call(&svc, "lfs", json!({"op": "delete", "paths": ["/etc"]}));
    assert_eq!(
        r["errors"].as_array().unwrap().len(),
        1,
        "system folders are refused"
    );
}
