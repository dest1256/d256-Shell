# d256 Shell

**A portable SSH client for Windows and Linux** — terminal · SFTP file browser · editor · server monitor

![Windows](https://img.shields.io/badge/Windows-0078D6?logo=windows&logoColor=white)
![Linux](https://img.shields.io/badge/Linux-FCC624?logo=linux&logoColor=black)
![Tauri](https://img.shields.io/badge/Tauri-24C8DB?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)
![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)

d256 Shell puts everything you need to work with servers into a single window styled like Windows Terminal, with a MobaXterm-like panel layout: the server's file tree on the left, the terminal on the right, and live load statistics at the bottom. No installation required: all data is stored next to the program in the `data` folder and protected by a master password.

![Terminal with its live duplicate side by side](docs/screenshots/terminal-split.png)

## Features

### Terminal

- The real system `ssh` running in a pseudo-console (ConPTY / PTY), rendered with xterm.js and WebGL.
- The password is supplied automatically via `SSH_ASKPASS` — no typing, and it never appears on the command line. A wrong password is tried exactly once.
- 2FA and any other server prompts are shown in a dialog.
- Paste with a warning for multi-line text, search in output, clickable links, font zoom, tabs, color schemes.
- **Terminal duplication** (`Ctrl+Shift+D`): a second live shell opens on the right and replays the current directory, virtual environment (venv/conda), `su` login and the running program (`btop`, `tail -f …`). Commands are shown and confirmed before being sent.

### Key and password authentication

- Passwords are stored in an encrypted vault (Argon2id + XChaCha20-Poly1305).
- Private key login (ed25519, RSA, ECDSA; OpenSSH and PEM formats) with an optional passphrase.

<p align="center"><img src="docs/screenshots/key-login.png" alt="Server form with key-based login" width="420"></p>

### File browser (SFTP)

- Virtualized list (10,000+ files), a `..` entry, breadcrumbs, sorting, hidden files.
- Create, rename, copy, cut, paste, delete (with object count), permissions (including recursive), properties.
- Upload and download files and folders through a transfer queue: progress, speed, cancel, retry, conflict prompts.
- Safe writes: temporary file → size check → replace. If the connection drops, the original stays untouched.
- Drag and drop: from Explorer into the panel, within the panel (move), and from the panel out.

![File browser](docs/screenshots/explorer.png)

### Built-in editor

- CodeMirror 6: syntax highlighting, find and replace, go to line. `Ctrl+S` saves straight to the server.
- Preserves BOM, line endings (LF/CRLF) and a missing trailing newline byte for byte.
- Any encoding (Windows-1251, KOI8-R, UTF-16, Shift_JIS, GBK …): auto-detection, manual selection, saving in the chosen encoding (conversion). Characters that don't exist in the target encoding are never lost — the save is refused instead.
- If the file changed on the server, a diff is shown and you choose what to do.

![Save conflict dialog](docs/screenshots/conflict.png)

### Server monitor

A status bar under the terminal (CPU, RAM, swap, load average, network, uptime and sparklines) plus a detailed tab with charts, per-core and per-disk load. Data is read from `/proc` over SFTP: **not a single command is executed on the server**.

![Server monitor](docs/screenshots/monitor.png)

## Security

| | |
|---|---|
| **Host keys** | Always verified. A changed key means refusal; replacing it requires an explicit double confirmation. |
| **Passwords** | Never written to logs, `ssh` arguments or environment variables (only a one-time askpass token is passed). |
| **Server** | The program runs no service commands and no `sudo`. Everything goes through SFTP or the interactive shell you opened. |
| **Network** | No requests other than SSH to your servers: no telemetry, no auto-updates. |
| **Dependencies** | crates.io and npm only, pinned versions; `cargo audit`, `cargo deny` and `npm audit` report no issues. |
| **UI** | Fully bundled into the program — no CDNs or remote fonts, strict CSP. |

## Installation

Prebuilt packages are available on the [Releases](../../releases) page.

### Windows 10 / 11 (x64)

1. Extract `d256Shell.zip` into any folder you have write access to.
2. Run `d256Shell.exe`. Requires WebView2 (included in Windows 11) and the OpenSSH client (Settings → Optional features).
3. Verify the SHA-256 against `SHA256SUMS-windows.txt`.

### Ubuntu / Debian

```bash
sudo apt install ./d256shell_1.0.2_amd64.deb
```

### Arch Linux

Put `PKGBUILD`, `d256shell-1.0.2-linux-x86_64.tar.gz`, `d256shell.desktop` and `LICENSE` in one folder, then run:

```bash
makepkg -si
```

### Portable (any Linux)

```bash
tar xzf d256shell-1.0.2-linux-x86_64.tar.gz && ./d256shell/d256shell
```

Requires WebKitGTK 4.1, GTK 3 and OpenSSH. The binary is built on Ubuntu 22.04 (glibc ≥ 2.34) and tested by installing on clean Ubuntu 22.04, 24.04 and Arch.

### Where data is stored

If there is a `data` folder next to the program, it is used (portable mode — the whole profile can be moved by copying it). For the Linux package, data lives in `~/.local/share/d256shell`. A different folder can be set with `D256SHELL_DATA_DIR`.

## Building from source

Everything is built and tested in Docker — nothing needs to be installed on the host.

```bash
cd .devcontainer
docker compose up -d --build                         # dev container and test sshd servers (ports 2221–2226)
docker compose exec dev bash scripts/check-all.sh    # lint, tests, e2e, dependency audit
docker compose exec dev bash scripts/make-dist.sh    # Windows: d256Shell.exe and zip
docker compose --profile linux run --rm linux bash scripts/build-linux.sh   # Linux: tar.gz, deb, PKGBUILD
```

**Layout:** `crates/core` (all the logic), `crates/devbridge` (HTTP bridge for e2e), `src-tauri` (window and commands), `ui` (TypeScript + Vite), `e2e` (Playwright), `test-server` (test sshd image), `packaging` (Linux), `scripts`.

Tests only ever run against the project's own test servers in containers, never against real ones. Details and design decisions: [NOTES.md](NOTES.md); dependencies: [DEPENDENCIES.md](DEPENDENCIES.md).

## Versioning and releases

Versions follow `MAJOR.MINOR.PATCH` ([Semantic Versioning](https://semver.org)): patch — bug fixes, minor — new features without breaking compatibility, major — breaking changes. History is in [CHANGELOG.md](CHANGELOG.md). A release is made by pushing a `v<version>` tag: GitHub Actions builds the Windows and Linux packages and attaches them to the Releases page.

## Known limitations

- PuTTY key files (`.ppk`) are not supported — export them to OpenSSH format.
- The monitor and terminal duplication require a Linux server with SFTP; on OpenWrt the `openssh-sftp-server` package is needed.
- Copying between different servers is not available yet; no SSH agent or hardware key support.
- Executables are not code-signed, so Windows SmartScreen may show a warning.

## License

Copyright © 2026 dest1256.

Released under the [MIT License](LICENSE). You may use, copy, modify and distribute this software, provided the copyright notice and license text are kept.

Third-party libraries are distributed under their own licenses (MIT, Apache-2.0, etc.); the full list is available in the app under **Settings → About → Library licenses**.

## Acknowledgements

Developed by dest1256 with the assistance of [Claude](https://claude.ai) by Anthropic.
