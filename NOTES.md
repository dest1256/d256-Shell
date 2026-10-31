# NOTES

Author: dest1256.

## Stages 0–1: decisions and deviations

- The dev container is built and started with `docker compose` from the host (the "Reopen in Container" button is not required):
  `docker compose -f .devcontainer/docker-compose.yml up -d`, commands: `docker compose exec dev …`.
- Images are pinned by digest: `rust:1.99.1-bookworm`, `node:22.23.4-bookworm`, `debian:bookworm-slim`.
- The tools (`cargo-xwin`, `tauri-cli`, `cargo-audit`, `cargo-deny`, `cargo-about --features cli`) are installed in `/opt/tools`
  so that the `CARGO_HOME` volume does not shadow them.
- Tauri: crates.io already has a 3.0 alpha; `tauri = "2"` (the latest 2.x) is used, as the spec requires.
- Files created from the Windows side appear in the container as `root:755`; `chown dev` was run once for the project directories
  (`docker compose exec -u root dev …`). If EACCES appears again, repeat it.
- The user accepted the Microsoft license for the CRT/SDK used by `cargo-xwin`; `XWIN_ACCEPT_LICENSE` is passed only on the command line.

## Known gaps (planned)

- The standard Tauri manifest in the exe **does not contain** `asInvoker`, `PerMonitorV2`, `longPathAware`, `supportedOS` — a custom
  manifest is needed (stage 9, item 13.2). The icon and VERSIONINFO (dest1256) are embedded during cross-compilation — checked with `scripts/inspect-exe.sh`.
- The "no traces outside the program folder" check and running on Windows have not been done yet.
- The data folder error message is currently written to stderr; a native message window comes later.

## Stage 2: vault, server manager

- `vault.bin` format: `SHPV | ver | m,t,p (u32 LE) | salt16 | nonce24 | ciphertext+tag`, the whole header is AAD. The parameters from the
  header are checked against sane limits (protection against a huge `m_cost` in a tampered file).
- The crate APIs were checked against the sources: `chacha20poly1305 0.11` (`Key`/`XNonce` are `hybrid_array::Array`, `from_slice` is deprecated),
  `argon2 0.6`, `getrandom 0.4` (`getrandom::fill`).
- The whole application API is one dispatcher `d256shell_core::service::Service::call(method, params)`. The Tauri layer is a single `api` command.
- **Deviation:** `devbridge` is not a WebSocket but HTTP JSON-RPC (`POST /rpc`) + serving `ui/dist` on the same origin, on `std` only
  (no new dependencies). The terminal (stage 4) needs push — then a WebSocket (dev-only dependency) will be added.
- The host and user name are validated on save (cannot start with `-`, no spaces, no `@`) — protection against option injection into the ssh argv.
- The password is not returned in the server list; it is shown by the "Show" button (`get_password`).
- `cargo audit`: 2 warnings (`proc-macro-error` unmaintained, `glib` unsound) — these are Linux/GTK dependencies of Tauri and are not part of the Windows
  build. To be documented in `DEPENDENCIES.md` at stage 9.
- Not done in stage 2: "Quick connect" and "Test connection" (need SFTP, stage 3); auto-lock on inactivity (stage 8).
- The `d256shell` (Tauri) crate is not built/tested on Linux (no GTK); lint and build go through `cargo xwin` for the Windows target.

## Stage 3: known_hosts, SFTP, explorer (read)

- Dependencies: `russh 0.65` (the repository is now `warp-tech/russh`, formerly Eugeny), `russh-sftp 3.0` (AspectUnk), `tokio`,
  `hmac`/`sha1`/`base64ct` (RustCrypto; needed for hashed known_hosts entries). The APIs were checked against the sources.
- `russh-sftp` handles non-UTF-8 names: names are decoded with replacement (`U+FFFD`), the listing does not break. Such entries are marked
  `name_lossy` and **blocked for all operations** (replacing it with `ssh2` was not required).
- Login policy: `authenticate_none` → method choice: if `password` is available — exactly one attempt (no fallback to keyboard-interactive with the same
  password), otherwise keyboard-interactive: the password is supplied once, the other prompts (2FA, repeated password) go to the user.
  Confirmed by tests against the sshd log (`Failed password` exactly +1).
- Host key: the algorithm order in `russh` is rearranged so that those known from known_hosts come first (as OpenSSH does);
  checked for ed25519/ecdsa/rsa. A changed key is refused before the password is sent; replacing it is a separate action with a double confirmation.
- Interactive questions (host key, password, 2FA) are implemented without push: `connect` blocks, the UI polls `poll_prompts` and replies with `answer_prompt`.
- Test infrastructure: the sshd servers write their log to a shared volume (`/sshlogs`), host keys are regenerated at start (otherwise all containers
  share the same ones — caught by the "key changed" test), `pam_google_authenticator` requires mode 0600 on the secret file.
- `aws-lc-sys` (the `russh` crypto) needs NASM for the Windows target — it was added to the dev image from the Debian repository.
- `cargo audit`: RUSTSEC-2023-0071 (`rsa`, Marvin) — an exception justified in `.cargo/audit.toml` (there are no private RSA operations).
- Stage limitations: no automatic reconnect on a dropped connection (there is a "Reconnect" banner), no write operations (stage 5).

## Stage 4: terminal

- The terminal is the system `ssh.exe` in ConPTY (`portable-pty`), output goes to xterm.js 6 (webgl/fit/unicode11/web-links/search). Bytes go through a `Channel` as is
  (UTF-8 is not reassembled), the queue is limited to 4 MiB with back-pressure; the 50 MB output test passes.
- ssh arguments: `-F none`, security options first, user options by allowlist, then `-p -l -t -- host`; `UserKnownHostsFile` with forward slashes and quotes when there are spaces.
- Password: `SSH_ASKPASS` points to the exe itself (helper mode by `D256SHELL_ASKPASS=1`), exchange over loopback TCP with a token, `SSH_ASKPASS_REQUIRE=force` (OpenSSH ≥ 8.4).
  For old ssh versions — expect mode with strict conditions (≤60 s, once, the user typed nothing, the prompt is at the end of the output and has been quiet for 150 ms).
- A bug found by a test: when the askpass dialog was cancelled, OpenSSH sent empty passwords (3 failed attempts on the server). Now `OnRefuse` kills ssh before the helper answers: 1 question, 1 failed attempt.
- A server without SFTP (error "sender dropped"): the session opens, the terminal works, the file panel explains the reason (`sftp_unavailable`).
- Paste: multi-line/large paste needs confirmation; bracketed paste is preserved. Ctrl+T/W/B with focus in the terminal go to the server, the application uses Ctrl+Shift+T/W/B.
- A test artifact: Playwright `keyboard.type` uses `insertText` for non-ASCII and reorders the input, `keyboard.press` does not know Cyrillic — Cyrillic is checked through paste, emoji through server output. This does not concern the product.
- Not verified on Windows (a manual run is needed): ConPTY, askpass from a GUI exe without a window flash, short 8.3 paths, ssh.exe terminating when the tab is closed.

## Stage 5: write operations, transfers

- Everything goes through SFTP, not a single command on the server (including OpenWrt/dropbear without SFTP: the hint is `opkg install openssh-sftp-server`, a fallback through the shell is deliberately **not** implemented — item 14 of the spec).
- Upload (`sftp/xfer.rs`): `.<name>.d256shell-<rand>.part` in the same folder → size check → permissions and mtime of the old file → replace. A symlink target is written "through" (the link stays a link), a broken link/folder/special file is refused.
  A foreign owner (uid ≠ uid of the home folder: SFTP cannot do "whoami") is a separate user decision ("write in place" / "replace").
- **Deviation:** the `posix-rename@openssh.com` extension is not used (`russh-sftp` does not expose extended requests), the fallback order from the spec is always used:
  original → `.bak`, temporary → target, delete `.bak`, on error restore the original. The window "between" the two renames is short; therefore servers "without posix-rename" need no separate test (the fallback path is the only one and is tested by all write tests). `statvfs`/`hardlink` are not detected (not needed).
- Leftovers (`.part`/`.bak` that could not be removed because of a dropped connection) are remembered in the session and deleted at the next connection to the same server (`Hub::connect` takes the list from old sessions).
- Recursion: upload follows local symlinks with loop protection (canonical paths), download does not enter remote symlinks to folders, symlinks to files are read. Name conflict **inside** a folder: the user's choice for the top level ("replace"/"skip"/"keep both" applies to all nested files) — a separate question is not asked for each nested file.
- Deletion: count before confirmation, `lstat` recursion, a hard refusal for `/`, an empty path, the home folder and everything containing it, and system roots; it stops at the first error with a report.
- Names: download refuses to create local names that are invalid on Windows (`:`, `\`, `CON`, trailing dot/space …) — skipped with a report. Non-UTF-8 names are blocked for all operations (and skipped in recursions).
- Transfer queue: `parallel_transfers` in parallel (2 by default), progress/speed/cancel/retry; a retry after a drop goes through a new session; the SFTP reconnect happens at the next explorer operation (one automatic retry) and on Ctrl+S.
- chmod: a dialog with rwx and an octal field, recursive only by checkbox, separately "files / folders", symlinks are not touched.
- A 512 MB test: `scripts/test-big.sh` (the default suite uses 96 MB, SHA-256 streamed).

## Stage 6: editor

- A server tab has a strip "Terminal | file1 ● | file2…" (shown when at least one file is open). CodeMirror 6, colors come from the selected terminal scheme.
- Opening: the limit from the settings (5 MB), NUL bytes or non-UTF-8 — refused with an offer to download, the BOM and the line ending type are remembered, the "base version" is SHA-256, size, mtime, mode, uid. Mixed line endings — LF/CRLF is asked at the first save.
  The editor always sees `\n`; the backend restores CRLF and the BOM (unit tests for a byte-exact round trip, including an empty file and a missing final newline).
- Saving: re-read and compare the hash → "File changed on the server" (show the diff `@codemirror/merge` / overwrite / cancel), "deleted" → create again, a foreign owner or an inaccessible folder → an offer of an "in place" write (explaining that it is not atomic), no permission → a message with the user name; the content is not lost, the ● marker stays. A new file is created only at the first save, an existing name is not overwritten.
- Closing a file tab / session / window with unsaved changes asks for confirmation.
- Not done (optional in the spec): "open in an external editor".

## Stage 7: drag and drop

- Inside the panel — on pointer events (HTML5 DnD does not work in WebView2 when the Tauri file drop is on): the target is a folder in the list or a path segment, with the confirmation "Move N items to …?" and "don't ask in this session". Verified by e2e.
- From Explorer into the panel: the Tauri `onDragDropEvent`; the coordinates are physical → divided by `devicePixelRatio`, the folder under the cursor is looked up (and highlighted). **Not verified on Windows** (a manual test at 100% and 150% is needed).
- From the panel to Explorer: outside the panel the files are downloaded to `data\staging\<rand>\` (the "don't release the button" overlay, cancel on release, limit from the settings, 200 MB by default), then a native drag with the `drag` crate. Staging is removed 120 s after the drop (Explorer may still be copying) and at startup. **Not verified on Windows.**

## Stage 8: polish

- Settings: transfers (parallelism, mtime), drag-out and editor limits, auto-lock on inactivity (off by default; does not trigger with an unsaved file), "About" (version, author dest1256, offline library licenses — `scripts/gen-licenses.mjs`: `cargo about` + npm).
- Renaming a tab — double click. Wheel in btop/htop/mc: arrows by default (ConPTY + `ssh.exe` lose mouse reports), switchable in the settings.
- Not done (optional): a light theme. Auto-lock is not covered by an automatic test (the minimum interval is a minute).

## Stage 9: results

- `deny.toml` (graph for the Windows target only): advisories/bans/licenses/sources — ok; `cargo audit` and `npm audit` (ui and e2e) — 0 vulnerabilities; the RUSTSEC-2023-0071 exception is documented.
- exe manifest: `asInvoker`, `PerMonitorV2`, long paths, Windows 10/11, UTF-8; version info (ProductName/CompanyName/LegalCopyright/description) — dest1256; no UPX and no obfuscation; `strip`.
- `scripts/run-win-tests.ps1` (run by the user): metadata, manifest, Defender (`MpCmdRun`), starting from a folder with spaces and Cyrillic, startup time, memory, traces outside the folder, askpass mode, `ssh.exe` login through SSH_ASKPASS to a local test server.
- **Deviation:** separate test exes with a real ConPTY (`cargo xwin test --no-run`) are not built — the terminal tests use a unix PTY and run in the container; Windows behavior (ConPTY, askpass from a GUI exe, 8.3 paths, closing ssh.exe) is checked with the manual checklist from the README.
- The Defender and VirusTotal results are entered in this file by the user after running `run-win-tests.ps1` (we do not upload files to third-party services).
- Known limitations: copying between different servers is unavailable; no digital signature (the `signtool` slot in CI is off); no auto-updates and no telemetry, per the spec.

## Server monitor (after stage 9)

- The bottom line under the terminal: numbers first, after a gap — mini charts (CPU, RAM, network); a click opens the "Monitor" tab (history charts of ~5 minutes, per-core load, memory/swap, network, load average, processes, machine information).
- **No commands on the server (item 14 of the spec):** the values are read as ordinary files `/proc/stat`, `/proc/meminfo`, `/proc/loadavg`, `/proc/uptime`, `/proc/net/dev` (+ once per session `/proc/cpuinfo`, `/etc/os-release`, hostname) over SFTP. The UI computes percentages and speeds from two samples (`crates/core/src/monitor.rs` — the parser, unit and integration tests).
- Limitations: Linux is required (on OpenWrt/dropbear without SFTP, or on non-Linux, a line with an explanation is shown), disk space (`statvfs`) is not shown — the SFTP extension is not available in `russh-sftp`. Polling runs only while the tab is visible and the window is not minimized; the interval and switching on are in the settings.
- Disks: `/proc/diskstats` (only whole physical devices: no partitions, loop/ram/sr; devices without a single operation are hidden). From two samples read/write (sector = 512 B), IOPS, queue and busy time (`ms_doing_io`, like `%util` in iostat) are calculated; the "Monitor" tab has a total chart and a row for each disk. The bottom line has no disks.

## The ".." row and the terminal snapshot

- Explorer: above all folders there is always a ".." row (except at the root): double click/Enter goes one level up, files can be dropped on it (move to the parent). It is not a real entry: it takes no part in copying, deleting, Ctrl+A and counters.
- "Duplicate" (button and Ctrl+Shift+D): to the right of the terminal (full height, the border can be dragged) a **live copy** opens — a second `ssh` process on the same server. At the top it shows the screen and scrollback of the source terminal (with colors, `@xterm/addon-serialize`), below a separator and a new shell into which the working folder, the virtual environment and the running program (btop, `tail -f`, …) are **replayed**.
  How this is found out without commands on the server: the terminal's shell is located by the client port in `SSH_CONNECTION`/`SSH_CLIENT` of its environment (the local `ssh` port is taken from the OS TCP table: `GetExtendedTcpTable` on Windows, `/proc/net/tcp` on Linux), then `/proc/<pid>/{stat,cwd,cmdline,environ}` are read over SFTP: the shell's folder, the foreground program (the terminal's process group) and its `VIRTUAL_ENV`/`CONDA_*`. For an idle shell, venv activation is not visible in `/proc` (the environment changes only in the shell's memory), so `source …/activate`, `conda activate`, `deactivate` typed in the terminal are additionally remembered (only if the line was typed completely by hand, without Tab/arrows).
  **Deviation from item 14 of the spec (at the user's direct request):** commands are typed into the copy automatically. Protection: before that they are shown in a window and are editable (`Settings → Terminal duplication`: "ask" — the default, "immediately" or "output only"), and they are typed only after the new shell has printed its prompt and while the user has typed nothing themself. The state of the running program is not copied — it starts anew; SFTP and a Linux server are required (on OpenWrt/dropbear it works if dropbear sets `SSH_CONNECTION`/`SSH_CLIENT`). Closing the copy asks for confirmation.
- Fixed: xterm replies to shell queries (BusyBox ash asks for the cursor position at start) arrive through `onData` and were previously counted as "the user started typing" — commands were not typed into the copy. Now only real key presses, paste and IME input (an 80 ms window) count as user input. For full-screen programs (btop, htop, mc, vim, tmux …) showing the earlier output at the top is off by default (a switch in the duplication dialog): their screen is a snapshot of one moment and looks like garbage in the copy's scrollback.
- Nested shells (`su - user`, `sudo -i`, `bash`): from the process table (`/proc/*/stat`) a chain is built from the login shell to the foreground program; the inner shell and the command that started it (`su - comfyui`) go first into the copy's plan, then `cd` into the inner shell's folder, venv activation and the program. This works without a password if the login is as root (`su`/`sudo` from root). As a regular user `su`/`sudo`/`ssh` will ask for a password, so **only** the login line is typed into the copy, the rest (folder, venv, program) is shown in the window and typed by hand after the password — nothing is typed "into a password prompt". For an idle inner shell the venv is not visible in `/proc`, so remembering what was typed works (`cd ~/App && source .venv/bin/activate` in one line is understood; a `cd` before activation is saved together with it, since the script path is usually relative). Another user's folder is read only with the ptrace right (root on a regular server; Docker lacks it — the test accounts for that).
- State detection is faster (first `stat` of all processes, the environment is read only for session leaders with a terminal), a notification is shown, the wait is limited to 25 s, any error is not hidden: the copy opens without commands, with an explanation. Between the launch lines there is a "terminal quiet for 400 ms" pause.
- The test sshd with a password allows root login (tests only) — to check `su - user` without a password.

## Linux port (Ubuntu, Arch)

- The code is shared: on Linux the terminal uses the same `portable-pty` (PTY instead of ConPTY) and the system `ssh`; askpass, expect mode, terminal duplication, the monitor, SFTP — unchanged (the integration and terminal tests run on Linux anyway).
- Differences: (1) **data folder** — a portable `data` next to the binary if it exists, otherwise XDG (`~/.local/share/d256shell`), because the package is installed in `/usr/bin` (no write access); `D256SHELL_DATA_DIR` still takes priority. Windows is unchanged. (2) **window frame** — on Linux it is the system one (`decorations(true)`), our own minimize/close buttons are hidden (`NATIVE_FRAME`); a frameless GTK window has no borders for resizing. (3) **drag-out** — the `drag` crate for GTK accepts a `gtk::ApplicationWindow` (`window.gtk_window()`), for Windows a window handle. (4) **download names** — the strict Windows name rules apply only on Windows (`windows_name_ok`), on Linux anything the remote side allows is fine except `/` and NUL. (5) the "ssh not found" error text is Linux-specific, with install commands.
- Build: `Dockerfile.linux` on Ubuntu 22.04 (the image is pinned by digest; the Rust and Node toolchains are copied from the same official images as the main dev image), `scripts/build-linux.sh` → tar.gz, deb (built by `dpkg-deb`), PKGBUILD with the ready tar.gz. The binary needs glibc ≥ 2.34 and WebKitGTK 4.1. The `custom-protocol` feature was added to `src-tauri` (embedding the UI in a plain `cargo build --release`).
- Verified: clippy (`scripts/clippy-linux.sh`), installing the deb into clean Ubuntu 22.04 and 24.04, building and installing the PKGBUILD in a clean Arch (`makepkg`, `pacman -U`), running on a virtual display: the window appears, the data folder is created in XDG, screenshots `e2e/screenshots/{ubuntu-22.04,ubuntu-24.04,arch}.png`.
- **Not verified** (there is no real desktop in the container): operation under Wayland/X11 with a window manager, system file pickers, dragging from/to a file manager, the WebKitGTK clipboard, WebGL in xterm on various drivers (for rendering problems — `WEBKIT_DISABLE_DMABUF_RENDERER=1`). AppImage and Flatpak are not built (they need third-party downloaders at build time; postponed under the supply-chain rules).

## Private key login

- The server stores **only the path** to the key file (`key_path`) and, if the user wants, the passphrase (`key_passphrase`, inside the same encrypted vault; wiped when removed from memory). The key itself is not copied anywhere. Clearing the "Use a private key" checkbox removes the path and the saved passphrase.
- Terminal: `-i <path> -o PubkeyAuthentication=yes -o IdentitiesOnly=yes -o IdentityAgent=none -o AddKeysToAgent=no -o PreferredAuthentications=publickey,keyboard-interactive,password`; without a key the behavior is as before (`PubkeyAuthentication=no`, no "foreign" keys). The path is passed as a separate argument (spaces need no quotes). The file is checked before ssh starts (no file / `.ppk` / a public `.pub` — a clear message).
- The passphrase in the terminal reaches askpass as "Enter passphrase for key …": the saved one is supplied **once**, after that the question goes to the user (masked input), as with the password; the passphrase and the password are not confused.
- SFTP (russh): the key is tried first (RSA — with the best hash the server supports); an encrypted key is decrypted locally, up to three passphrase questions without a single login attempt on the server; if the key is rejected — "The server did not accept the key", and with a saved password (or on partial success, key + 2FA code) the login continues under the earlier policy: exactly one password attempt.
- Not supported: `.ppk` (export to OpenSSH is needed), the SSH agent, certificates (`*-cert.pub`), hardware keys (FIDO).
- Tests: the test sshd `sshd-key` (port 2226, keys are generated at start into the shared volume `/testkeys`: ed25519, RSA, RSA-PEM, ECDSA, a key with the passphrase `keypass`, an unauthorized key, a `.ppk` file); integration (`key_login`), terminal (a real `ssh -i` + askpass), e2e (`keys.spec.ts`).

## Encodings in the editor

- Previously a non-UTF-8 file did not open ("The file cannot be opened in the editor"). Now all encodings of the WHATWG standard are supported (the `encoding_rs`, `chardetng` crates): UTF-8/16, Windows-125x, KOI8-R/U, CP866, ISO-8859-x, Mac, Shift_JIS, EUC-JP/KR, GBK, GB18030, Big5 (the choice list is `ui/src/encodings.ts`, the same as `sftp/codec.rs`).
- Opening: BOM → valid UTF-8 → auto-detection (`chardetng`; the status bar says "detected automatically"). The encoding button at the bottom right: "Reopen with this encoding" (if the guess was wrong) and "Save with this" (conversion; a BOM checkbox for Unicode). If the file cannot be decoded automatically, a choice of encoding is offered on opening.
- **Data safety:** a file is opened in an encoding only if writing the unchanged text back gives the same bytes (otherwise a refusal, "choose another encoding"); invalid bytes are not replaced by "�"; a character that does not exist in the selected encoding (for example `ß` in Windows-1251) is named in the error on saving and nothing is written. Null bytes still mean a binary file (except UTF-16 with a BOM or manually selected UTF-16).
- Auto-detection is a heuristic: for short files it can be wrong (for example KOI8-R/Windows-1251) — hence the manual choice. UTF-32 is not supported.

## "Monitor" tab in the style of the Windows 10 task manager

- On the left a list of resources with live mini charts (CPU, Memory, each disk, each network interface), on the right the details of the selected one — like the Performance tab: CPU (a chart per logical core or a total one, utilization, speed in GHz, threads, uptime d:hh:mm:ss, load), Memory (chart, a "used / cache / free" bar, available, cached, swap), Disk (active time, average response time, read/write speeds with a dashed line for writes, capacity/type HDD-SSD/model, IOPS, queue), Network (receive/send in bit/s, state, link speed, MAC, total received/sent). The chart window is 60 seconds.
- Sources (read-only files over SFTP, no commands): `/proc/stat`, `/proc/meminfo`, `/proc/loadavg`, `/proc/uptime`, `/proc/net/dev`, `/proc/diskstats`, `/proc/cpuinfo` (frequency), once `/sys/block/<disk>/{size,queue/rotational,device/model}` and `/sys/class/net/<interface>/{speed,operstate,address}`.
- Not available (impossible over SFTP without commands): GPU, the adapter's IP address, the number of processes, the base frequency and cache sizes of the CPU. "Threads" is the number of tasks from `/proc/loadavg`. In virtual machines and containers `/sys` may hold no data about disks and network cards: then the corresponding fields show "—".
- Refresh rate: the "Refresh rate" list at the bottom of the "Monitor" tab (pause, 1/2/5/10/30 s); the value is saved in `settings.json` (`monitor_interval_sec`, the same field in Settings, range 1–60 s) and applied immediately. The chart window is always 60 seconds (the number of points depends on the rate).
- Fixed: the device information (model, capacity and type of a disk, network card speed, CPU model) arrives in the first sample, but the tab read it from the last one and showed "—"; it is also read later when a disk or interface appears later (idle disks are hidden in the first sample).
- The refresh rate is given in **milliseconds** (`monitor_interval_ms`, 100–60000; an old value in seconds from an old `settings.json` is migrated automatically). The list on the "Monitor" tab: pause, 250 ms … 30 s and "Custom rate…" (a field in ms, from 100). The actual pace is limited by network latency: the next poll starts after the previous reply, so 100 ms is unreachable on a slow link — the charts simply update as often as the server answers. The history keeps up to 700 points (a minute at 100 ms).

## Release 1.0.0 and the rename

- The project was renamed **d256 Shell** (formerly Shellport): the window, titles, "About", exe version info (ProductName `d256 Shell`), packages. File names without spaces: `d256Shell.exe`, `d256Shell.zip`, Linux `d256shell` (`d256shell-1.0.0-linux-x86_64.tar.gz`, `d256shell_1.0.0_amd64.deb`); identifier `com.dest1256.d256shell`; crates `d256shell`, `d256shell-core`, `d256shell-devbridge`; environment variables `D256SHELL_*` (`D256SHELL_DATA_DIR` …); the temporary file suffix on the server `.d256shell-<rand>.part`; the Linux XDG folder `~/.local/share/d256shell`.
- The data format did not change (`vault.bin`, `known_hosts`, `settings.json`), so the `data` folder from pre-release builds can be used as is. Only small UI settings in the WebView (panel positions etc.: their `localStorage` key is now `d256shell.*`) are not carried over.
- The internal dev container path `/workspaces/shellport` was left on purpose: changing it would require a full rebuild of the Docker image, and it does not affect the product.
- Version 1.0.0 (the single source is `Cargo.toml`); the rules and the bump script are in `VERSIONING.md`, `scripts/bump-version.sh`; the log is `CHANGELOG.md`; a GitHub release is the tag `v<version>` (workflow `release.yml`, **never run**: only the syntax and the pinned action SHAs were checked).

## 1.0.1

- The confirmation dialog for pasting into the terminal (multi-line or large text) contains an editable `textarea`: the edited text is pasted, `Ctrl+Enter` pastes, empty text = cancel. The warning and the line and character counters update while editing.
- The procedure for publishing an update: `RELEASING.md`.

## 1.0.2: MIT license and English

- The project is distributed under the MIT license (`LICENSE`, © 2026 dest1256), instead of the previously adopted "private" `COPYRIGHT` notice. Metadata: `license = "MIT"` in `Cargo.toml` and the crates, `"license": "MIT"` in `package.json`, `license=('MIT')` in `PKGBUILD`. The license is placed in the packages (tar.gz, `/usr/share/doc/d256shell/copyright` in the deb, `/usr/share/licenses` in Arch). MIT requires users to keep the copyright notice and the license text; third-party library licenses stay under their own terms (the list is in "About").
- Before publishing it was checked that the repository contains no secrets: the test credentials (`tester/testpass`, the `test-server` keys) are used only in the isolated Docker test containers.
- The whole program and repository were translated to English: UI, messages from the core, tests, scripts, documentation. Dates and numbers use `en-US`. The Russian words that the program must recognize in server prompts (for example the password prompt of a Russian-localized server) were deliberately kept in `prompt.rs` and `terminal/expect.rs`, and Cyrillic test data (file names, text in other encodings) was kept as test input.
