# Changelog

The versioning scheme is described in [VERSIONING.md](VERSIONING.md).

## 1.1.0 — 2026-11-02
One window for everything command-line: SSH sessions, local shells, scripts and notes, in eight languages.

**Added**
- **Languages:** English, Русский, Español, Deutsch, Français, Português (Brasil), 简体中文, 日本語. The default follows the system language; choose another in Settings → Language (applied at once). Messages from the program core are translated too. Catalogs are checked by `scripts/check-i18n.mjs` (every text in every language, same placeholders).
- **Terminals:** the left column has a new section. Local shells are found on the computer and open in tabs next to the SSH sessions: Windows PowerShell, PowerShell 7, Command Prompt, WSL distributions, Git Bash (Windows), bash/zsh/fish and the others from `/etc/shells` (Linux). Your own profiles (program, arguments, start folder) are added in the section. One shell can open at start (Settings → Open at start).
- **Scripts:** `.bat`, `.cmd`, `.ps1` and `.sh` files. Two ways to add one: write it in the built-in editor (saved as a file in `data/scripts`) or drop/choose a file: then only a **link** is stored and the file is never copied or modified by the list. A link whose file was moved is shown grey with “file not found”, and can be pointed to the new place or removed. “Run” opens a terminal tab with the right shell in the script's folder (nothing runs without a click). Encodings, BOM and line endings are kept as they are.
- **Notes:** any number of notes with any titles, saved automatically in the encrypted vault (`vault.bin`), together with the passwords.
- **Icons:** every session, shell, script and note can have its own icon (a built-in glyph or your own png/svg/jpg/webp/ico/gif image, copied into `data/icons`).
- **Tabs on the left:** Settings → Tabs → Left (vertical) moves the tab strip into the left column.

**Changed**
- The left column is now a section switcher (Sessions, Terminals, Scripts, Notes) and the home tab shows the cards of the chosen section.
- After the active tab is closed, the side panel is shown again.

**Compatibility**
- Notes are stored in `vault.bin`. Version 1.1.0 opens vaults from older versions, but older versions ignore the notes and would drop them the next time they save the vault. After starting to use notes, use 1.1.0 or newer everywhere the same `data` folder is used.

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
