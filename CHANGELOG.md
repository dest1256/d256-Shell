# Changelog

The versioning scheme is described in [VERSIONING.md](VERSIONING.md).

## 1.0.2 — 2026-10-31
The project is now released under the MIT license, and the whole program and repository are in English.

**Changed**
- License: instead of the "all rights reserved" notice (`COPYRIGHT`) the project is distributed under the **MIT** license (`LICENSE` file). The license is stated in the metadata (`Cargo.toml`, `package.json`, `PKGBUILD`), shipped in the Linux packages (tar.gz, deb, Arch) and shown in the About window.
- New English `README.md` for the repository: features, security, installation, building, versions, limitations, license.
- The release files for the `PKGBUILD` build now contain `LICENSE` instead of `COPYRIGHT`.
- **Language:** the user interface, messages from the core, tests, scripts and documentation are translated to English. Dates and numbers use the `en-US` format.

## 1.0.1 — 2026-10-28
A small improvement of the paste dialog.

**Changed**
- The paste confirmation dialog now contains an editable field: before pasting you can fix the text or remove an unwanted (dangerous) line. What remains in the field is pasted. The title and the character counter update as you edit, `Ctrl+Enter` pastes, empty text counts as cancel.
- The Arch release files now include `d256shell.desktop` and `COPYRIGHT`, needed by the `PKGBUILD` (`scripts/sync-github.sh`).

## 1.0.0 — 2026-10-28
First release. The program is called **d256 Shell** (during development the project was called Shellport).

- **Terminal** on top of the system `ssh` (ConPTY / PTY), automatic password entry through askpass, 2FA, tabs, search, paste with a warning, color schemes.
- **Password vault** protected by a master password (Argon2id + XChaCha20-Poly1305), host key verification, login with a private key (ed25519, RSA, ECDSA) with a passphrase.
- **SFTP explorer:** create, rename, copy, move, delete, permissions, transfer queue, drag-and-drop in both directions, safe file writes.
- **CodeMirror editor:** Ctrl+S straight to the server, conflict detection with a diff view, any encoding (Windows-1251, KOI8-R, UTF-16 …) with auto-detection and a choice of the saving encoding.
- **Server monitor** in the style of the Windows task manager: CPU per core, memory, disks, network; selectable refresh rate (down to 100 ms). Data is read from `/proc` and `/sys`; no commands are run on the server.
- **Terminal duplication** (Ctrl+Shift+D): a second live shell that repeats the folder, venv, `su` and the running program.
- **Platforms:** Windows 10/11 (x64) and Linux (Ubuntu/Debian, Arch, portable archive).
- Proprietary software: no license is granted, all rights reserved (see COPYRIGHT).
