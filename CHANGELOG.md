# Changelog

The versioning scheme is described in [VERSIONING.md](VERSIONING.md).

## 1.3.2 — 2026-11-11
**Changed**
- **Video card graph:** the CUDA (Compute) load is now filled like the 3D load and the other graphs, not only drawn as a line. A long generation at 99% or at 0% is visible at a glance.

## 1.3.1 — 2026-11-05
A fix for the release workflow; the program itself is unchanged.

**Fixed**
- The release job failed with "a release with the same tag name already exists" when the release had been created by hand before the workflow finished. It now adds the files to the existing release (`gh release upload --clobber`) and creates the release only when there is none.

## 1.3.0 — 2026-11-05
The monitor of this computer: a bar under local terminals, a column next to the terminal, the video card and the processes.

**Added**
- **Monitor of this computer:** a local terminal now has the same bar under it as an SSH session (processor, memory, disks, network, uptime, small graphs). A click opens the same task-manager tab as for servers, for your computer.
- **Video card:** 3D load, CUDA (Compute) load, Copy and Video engines, video memory (VRAM), temperature, power, fan and clock. On Windows the loads come from the performance counters “GPU Engine” and “GPU Adapter Memory” (the data of the Task Manager, any card, no extra programs); if `nvidia-smi` is installed (it comes with the NVIDIA driver) it adds temperature, power, fan, clock and the total VRAM. On Linux: `nvidia-smi` and, for AMD, sysfs. Several cards are listed separately. Without a video card or tool nothing about it is shown.
- **Processor temperature** when the system reports it (Windows thermal zones; Linux hwmon).
- **Processes:** a new “Processes” item shows the busiest processes with processor load, video load, video memory and memory (for example `python` of ComfyUI).
- **Monitor column to the right of the terminal**, in local **and SSH** tabs: the button at the right end of the bar folds and unfolds it, a splitter changes its width. The resources (CPU, Memory, GPU, Disk, Network, Processes) are chips at the top, the details of the chosen one are below, in a column. The “Monitor” tab stays as it was, both use the same data. (A server has no video card and no process list: SFTP cannot give them.)

**Notes**
- Everything is read on your own computer: performance counters, `/proc`, `/sys` and, when present, `nvidia-smi` (`D256SHELL_NVIDIA_SMI` can point to it) and `reg query` for the processor name. Nothing is sent anywhere. The refresh rate of the Monitor settings applies (the video card is asked at most once a second).
- The Windows counters were built and checked for compilation only; they could not be run on a real Windows computer with a video card during development.

## 1.2.1 — 2026-11-04
A fix for the GitHub Actions build; the program itself is unchanged.

**Fixed**
- The Windows unit test job failed in `paths::tests::xdg_locations`: that test checks Unix (XDG) paths, which are not absolute paths on Windows. It now runs on Unix only.

## 1.2.0 — 2026-11-04
Scripts: a choice of the encoding and encrypted storage; a check for new releases on GitHub.

**Added**
- **Encoding of a script:** the script editor (and any local text file opened from the explorer) has the encoding button at the bottom right, like the editor of remote files: the encoding is detected automatically (and shown), “Reread in this encoding” decodes the file again, “Save as this” converts it to another encoding, with an optional BOM. A character the chosen encoding cannot hold is named and nothing is written; bytes that do not decode ask for an encoding.

- **Update check:** at start the program asks GitHub (the public repository) whether a newer release exists and, if so, shows “Update X is available · Release page” in the status bar (it can be dismissed; a dismissed version is not announced again). Settings → Updates: switch the check off, or press “Check now”. It is **one HTTPS request** to `api.github.com` at most once a day, made by the UI; nothing about you is sent, nothing is downloaded or installed, only the release page of this repository can be opened. A missing network is silent at start.

**Changed**
- **Scripts written in the program are stored encrypted in the vault** (`vault.bin`, together with the passwords and notes) as exact file bytes, no longer as plain files in `data/scripts`. To run one, the program decrypts it into a private folder under `data/staging` for the time of the run; that copy is removed when the terminal tab is closed (and `staging` is emptied at every start and exit). Linked scripts (files that stay where they are) are unchanged.
- **Migration:** scripts that 1.1.x kept as files in `data/scripts` move into the vault automatically the first time the Scripts section is opened (a toast tells how many); a file is deleted only after the vault was saved. A name that is already taken gets a suffix (`name (2).sh`).

**Compatibility**
- Like notes, scripts in `vault.bin` are ignored by versions before 1.2 and would be dropped when those save the vault. Use 1.2.0 or newer everywhere the same `data` folder is used.

## 1.1.4 — 2026-11-04
Another fix for the GitHub Actions build; the program itself is unchanged.

**Fixed**
- The Windows jobs could not build the frontend: `npm ci` on Windows did not install the native packages of TypeScript 7, rolldown and lightningcss, because the lock file (made on Linux) lists only the Linux ones ("Unable to resolve @typescript/typescript-win32-x64"). The workflows now install the Windows packages at the exact versions of the lock after `npm ci` (nothing is saved to the lock file).

## 1.1.3 — 2026-11-04
A fix for the GitHub Actions build; the program itself is unchanged.

**Fixed**
- The Windows lint job failed on a test helper that is used only on Linux ("function tmp is never used"); it is now compiled only where it is used.
- The workflow steps of the Windows jobs now run in bash, which stops at the first failing command. PowerShell only checks the last command of a step, so a failing frontend build used to be hidden and showed up later as "frontendDist ../ui/dist does not exist". The build now also checks that ui/dist/index.html exists.

## 1.1.2 — 2026-11-03
Scripts behave like launchers, the side panel folds with one click, scripts can be renamed.

**Changed**
- **Scripts:** a click on a script now **runs** it, or, if it is already running, switches to its tab (a finished run is replaced by a new one). The row buttons are: Edit, Run a copy in a new tab, Rename, Change icon, Delete / Remove (Locate the file… for a missing link).
- **Terminals:** a shell in the list opens with a **double click** (like a session), a single click only selects it.
- **Local explorer:** a file opens as a pane of the same tab (“Terminal | file”), exactly like in an SSH tab, instead of a separate top tab.
- **Closing a local terminal** that is still running asks for confirmation, as for an SSH session (a finished one closes at once).
- **Side panel:** a click on the **active** icon of the left column folds or unfolds the panel; a click on another icon switches to that section and unfolds the panel.

**Added**
- **Rename scripts:** give a script any name (the file keeps its own name, shown underneath). The name is used in the lists, cards and tab titles. An empty name or the file name restores the default. Names are kept in `settings.json` (`script_names`), the script files are not touched.

## 1.1.1 — 2026-11-02
A file explorer for local terminals.

**Added**
- **Local explorer:** a local terminal tab now has the file explorer of **this computer** on its left, just like an SSH tab has the SFTP explorer: list with hidden files, breadcrumbs and the “..” row, create, rename, copy, cut, paste, delete, properties, drag and drop inside the panel. On Windows the root shows the drives. A text file opens in the built-in editor (encoding, BOM and line endings are kept) and is saved with Ctrl+S.
- Nothing is ever overwritten: a name clash is an error, a copy gets a new “name (copy)” name. The drive root, the home folder and the system folders cannot be deleted from the panel; a link is removed itself, never its target.
- Not offered in the local explorer (they make no sense on one computer): upload, download and permissions.

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
