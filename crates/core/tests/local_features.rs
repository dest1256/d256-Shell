//! Local features through `Service::call` (the API the UI uses): notes in the vault, scripts (own files and links),
//! local shells, icons. No SSH server is needed.
mod common;
#[cfg(unix)]
use base64ct::{Base64, Encoding};
use common::tmp;
use d256shell_core::paths;
use d256shell_core::service::Service;
use d256shell_core::vault::KdfParams;
use serde_json::{json, Value};
#[cfg(unix)]
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

#[cfg(unix)]
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
fn own_scripts_live_encrypted_in_the_vault_with_exact_bytes_and_any_encoding() {
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
        json!({"name": "hello.sh", "text": "echo SECRET-TOKEN-777\n"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(id, "own:hello.sh");
    assert!(
        svc.call("script_create", json!({"name": "HELLO.sh", "text": "x"}))
            .is_err(),
        "never overwrites (case-insensitive)"
    );
    // nothing on disk in the clear: not as a file, not inside vault.bin
    assert!(!root.join("scripts").join("hello.sh").exists());
    assert!(
        !String::from_utf8_lossy(&std::fs::read(root.join("vault.bin")).unwrap())
            .contains("SECRET-TOKEN-777")
    );
    assert_eq!(
        call(&svc, "script_read", json!({"id": id}))["text"],
        "echo SECRET-TOKEN-777\n"
    );

    // UTF-8 with BOM + CRLF: kept byte for byte (written as the editor does: text with \n, eol, bom, encoding)
    let ps = "Write-Host 1\nWrite-Host 2\n";
    call(
        &svc,
        "script_create",
        json!({"name": "win.ps1", "text": ""}),
    );
    call(
        &svc,
        "script_write",
        json!({"id": "own:win.ps1", "text": ps, "eol": "crlf", "bom": true, "encoding": "UTF-8"}),
    );
    let r = call(&svc, "script_read", json!({"id": "own:win.ps1"}));
    assert_eq!(
        (
            r["eol"].as_str(),
            r["bom"].as_bool(),
            r["encoding"].as_str()
        ),
        (Some("crlf"), Some(true), Some("UTF-8"))
    );
    assert_eq!(r["text"], ps);

    // a Windows-1251 batch file: auto-detected, rewritten in the same encoding; reread forced in another one
    call(
        &svc,
        "script_create",
        json!({"name": "old.bat", "text": ""}),
    );
    call(
        &svc,
        "script_write",
        json!({"id": "own:old.bat", "text": "@echo Привет, мир\n", "eol": "crlf", "bom": false, "encoding": "windows-1251"}),
    );
    let r = call(&svc, "script_read", json!({"id": "own:old.bat"}));
    assert_eq!(
        (r["encoding"].as_str(), r["guessed"].as_bool()),
        (Some("windows-1251"), Some(true))
    );
    assert_eq!(r["text"], "@echo Привет, мир\n");
    let koi = call(
        &svc,
        "script_read",
        json!({"id": "own:old.bat", "encoding": "KOI8-R"}),
    );
    assert_ne!(
        koi["text"], "@echo Привет, мир\n",
        "another encoding shows other letters"
    );
    assert_eq!(koi["encoding"], "KOI8-R");
    // saving in another encoding converts; a character the encoding cannot hold is named and nothing is written
    call(
        &svc,
        "script_write",
        json!({"id": "own:old.bat", "text": "@echo Привет, мир\n", "eol": "crlf", "bom": false, "encoding": "IBM866"}),
    );
    assert_eq!(
        call(
            &svc,
            "script_read",
            json!({"id": "own:old.bat", "encoding": "IBM866"})
        )["text"],
        "@echo Привет, мир\n"
    );
    let e = svc.call("script_write", json!({"id": "own:old.bat", "text": "ß 日本\n", "eol": "lf", "bom": false, "encoding": "windows-1251"})).unwrap_err();
    assert_eq!(e.code, "encoding");
    assert_eq!(
        call(
            &svc,
            "script_read",
            json!({"id": "own:old.bat", "encoding": "IBM866"})
        )["text"],
        "@echo Привет, мир\n",
        "failed write changed nothing"
    );
    // bytes that are invalid in the chosen encoding are refused with a code the UI turns into the picker
    call(&svc, "script_create", json!({"name": "bin.sh", "text": ""}));
    let e = svc
        .call(
            "script_read",
            json!({"id": "own:bin.sh", "encoding": "nope"}),
        )
        .unwrap_err();
    assert_eq!(e.code, "encoding");

    let list = call(&svc, "scripts_list", json!({}));
    let names: Vec<&str> = list["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["bin.sh", "hello.sh", "old.bat", "win.ps1"]);
    assert!(
        list["scripts"][0]["path"] == "",
        "no file path for a script in the vault"
    );

    // it survives lock / unlock, and delete removes it
    call(&svc, "lock", json!({}));
    assert!(svc.call("scripts_list", json!({})).is_err());
    call(&svc, "unlock", json!({"master": "long-enough-pw"}));
    assert_eq!(
        call(&svc, "script_read", json!({"id": "own:win.ps1"}))["text"],
        ps
    );
    call(&svc, "script_delete", json!({"id": "own:hello.sh"}));
    assert!(svc
        .call("script_read", json!({"id": "own:hello.sh"}))
        .is_err());
    assert!(
        svc.call("script_read", json!({"id": "own:../vault.bin"}))
            .is_err(),
        "no way out through a name"
    );
}

#[test]
fn scripts_of_version_1_1_move_from_data_scripts_into_the_vault() {
    let (svc, root) = service("scripts-migrate");
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(root.join("scripts/old.sh"), "echo old\n").unwrap();
    std::fs::write(root.join("scripts/win.bat"), b"@echo hi\r\n").unwrap();
    std::fs::write(root.join("scripts/readme.txt"), "not a script").unwrap();
    // a script of the same name already in the vault is not overwritten
    call(
        &svc,
        "script_create",
        json!({"name": "old.sh", "text": "echo new\n"}),
    );
    let list = call(&svc, "scripts_list", json!({}));
    assert_eq!(list["migrated"], 2);
    let names: Vec<&str> = list["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["old (2).sh", "old.sh", "win.bat"]);
    assert_eq!(
        call(&svc, "script_read", json!({"id": "own:old.sh"}))["text"],
        "echo new\n"
    );
    assert_eq!(
        call(&svc, "script_read", json!({"id": "own:old (2).sh"}))["text"],
        "echo old\n"
    );
    assert_eq!(
        std::fs::read(root.join("scripts/win.bat")).ok(),
        None,
        "the plain copy is gone"
    );
    assert!(
        root.join("scripts/readme.txt").exists(),
        "other files are not touched"
    );
    assert_eq!(call(&svc, "scripts_list", json!({}))["migrated"], 0, "once");
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

    // an own script runs from a decrypted private copy that is removed with its terminal
    call(
        &svc,
        "script_create",
        json!({"name": "where.sh", "text": "echo ran-$((20+22))-$(basename \"$0\")\n"}),
    );
    let before = std::fs::read_dir(root.join("staging")).unwrap().count();
    let r = call(&svc, "script_run", json!({"id": "own:where.sh"}));
    let term = r["term_id"].as_str().unwrap().to_string();
    read_until(&svc, &term, "ran-42-where.sh");
    assert_eq!(
        std::fs::read_dir(root.join("staging")).unwrap().count(),
        before + 1,
        "a decrypted copy exists while it runs"
    );
    call(&svc, "term_close", json!({"term_id": term}));
    assert_eq!(
        std::fs::read_dir(root.join("staging")).unwrap().count(),
        before,
        "and is gone after the terminal closed"
    );
    assert!(!root.join("scripts").exists(), "no plain scripts folder");
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

#[cfg(unix)]
#[test]
fn local_monitor_samples_have_the_server_shape_plus_gpu_temperature_and_processes() {
    use std::os::unix::fs::PermissionsExt;
    let (svc, _root) = service("localmon");
    // a stand-in for nvidia-smi (the real one needs an NVIDIA card): the program is found through D256SHELL_NVIDIA_SMI
    let dir = tmp("localmon-smi");
    let smi = dir.join("nvidia-smi");
    let me = std::process::id();
    std::fs::write(
        &smi,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --query-gpu*) echo '0, NVIDIA GeForce RTX 4090, 92, 21914, 24564, 71, 312.45, 450.00, 64, 2535';;\n  --query-compute-apps*) echo '{me}, 20800';;\nesac\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&smi, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("D256SHELL_NVIDIA_SMI", &smi);

    let first = call(&svc, "local_monitor_sample", json!({"info": true}));
    // the shape of the server monitor
    assert_eq!(first["cpu"].as_array().unwrap().len(), 2);
    assert!(!first["cores"].as_array().unwrap().is_empty());
    assert!(first["mem"]["total"].as_u64().unwrap() > 0);
    assert!(first["uptime"].as_f64().unwrap() > 0.0);
    assert!(first["net"].is_object() && first["disks"].is_array() && first["ifaces"].is_array());
    assert!(
        first["info"]["cpu_model"].is_string() || first["info"]["hostname"].is_string(),
        "{}",
        first["info"]
    );
    assert!(first["info"]["devices"].is_object());
    // the video card (from the stand-in) with its numbers
    let g = &first["gpus"][0];
    assert_eq!(g["name"], "NVIDIA GeForce RTX 4090");
    assert_eq!(
        (g["util"].as_f64(), g["temp"].as_f64(), g["power"].as_f64()),
        (Some(92.0), Some(71.0), Some(312.45))
    );
    assert_eq!(g["vram_total"].as_u64(), Some(24564 * 1024 * 1024));
    assert!(first.get("cpu_temp").is_some());
    assert!(first["procs"].as_array().is_some());

    // burn a little CPU so that this very process shows up, then the second sample has the process list
    let t = std::time::Instant::now();
    let mut x = 0u64;
    while t.elapsed() < Duration::from_millis(300) {
        x = x.wrapping_add(std::hint::black_box(1));
    }
    assert!(x > 0);
    let second = call(&svc, "local_monitor_sample", json!({}));
    assert!(
        second.get("info").is_none(),
        "the machine description comes only when asked"
    );
    assert!(
        second["cpu"][1].as_u64() >= first["cpu"][1].as_u64(),
        "cumulative counters only grow"
    );
    let procs = second["procs"].as_array().unwrap();
    let mine = procs
        .iter()
        .find(|p| p["pid"].as_u64() == Some(u64::from(me)))
        .unwrap_or_else(|| panic!("this process is busy and must be listed: {procs:?}"));
    assert_eq!(
        mine["vram"].as_u64(),
        Some(20800 * 1024 * 1024),
        "memory of a compute app comes from nvidia-smi"
    );
    assert!(mine["cpu"].as_f64().unwrap() > 0.0 && mine["mem"].as_u64().unwrap() > 0);
    std::env::remove_var("D256SHELL_NVIDIA_SMI");

    // a missing nvidia-smi is not an error: no cards, everything else works
    std::env::set_var("D256SHELL_NVIDIA_SMI", dir.join("nope"));
    let svc2 = service("localmon2").0;
    let s = call(&svc2, "local_monitor_sample", json!({}));
    assert_eq!(
        s["gpus"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| g["name"].as_str().is_some_and(|n| n.contains("RTX")))
            .count(),
        0
    );
    std::env::remove_var("D256SHELL_NVIDIA_SMI");
}
