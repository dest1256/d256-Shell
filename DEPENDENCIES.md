# d256 Shell dependencies

Author and copyright holder: **dest1256**. All packages come only from crates.io and registry.npmjs.org: no git dependencies,
forks or third-party mirrors. `Cargo.lock` and `package-lock.json` are committed; builds use `cargo build --locked` and `npm ci`.
npm runs with `ignore-scripts=true` (`ui/.npmrc`, `e2e/.npmrc`): no package runs install scripts.
Versions of direct npm dependencies are pinned exactly (no `^`/`~`).

Before adding each package the following was checked: the exact name (typosquatting protection), the repository in the metadata, activity,
and the absence of a fresh suspicious release (for npm and `tauri-plugin-dialog` versions older than two weeks at the time of selection were chosen).

## Rust: direct dependencies (end up in the exe)

| Crate | Version | Purpose | Repository | License | Why it is needed |
|---|---|---|---|---|---|
| tauri | 2.12.5 | Window, WebView2, IPC | github.com/tauri-apps/tauri | Apache-2.0 OR MIT | Application shell (per the spec) |
| tauri-build | 2.7.4 (build) | Tauri build script, exe resources, manifest | github.com/tauri-apps/tauri | Apache-2.0 OR MIT | Version/manifest/icon of the exe |
| tauri-plugin-opener | 2.7.2 | Opens links from the terminal in the system browser | github.com/tauri-apps/plugins-workspace | Apache-2.0 OR MIT | Links open only on click, never inside the WebView |
| tauri-plugin-dialog | =2.8.1 | System file/folder picker | github.com/tauri-apps/plugins-workspace | Apache-2.0 OR MIT | "Upload files", "Download…" (real local paths are needed) |
| drag | 2.1.1 | Native drag of files from the panel to Windows Explorer | github.com/crabnebula-dev/drag-rs | Apache-2.0 OR MIT | Drag-out (per the spec, item 6.4) |
| russh | 0.65.0 | SSH client for SFTP | github.com/warp-tech/russh | Apache-2.0 | SFTP connection, host key verification |
| russh-sftp | 3.0.1 | SFTP on top of russh | github.com/AspectUnk/russh-sftp | Apache-2.0 | Explorer operations |
| portable-pty | 0.9.0 | Pseudo-console (ConPTY on Windows, PTY on Linux) | github.com/wezterm/wezterm | MIT | Runs the system ssh in the terminal |
| tokio | 1.54.1 | Async runtime | github.com/tokio-rs/tokio | MIT | SFTP, transfer queue |
| argon2 | 0.6.0 | Master password KDF (Argon2id) | github.com/RustCrypto/password-hashes | MIT OR Apache-2.0 | Password vault |
| chacha20poly1305 | 0.11.0 | Vault encryption (XChaCha20-Poly1305) | github.com/RustCrypto/AEADs | Apache-2.0 OR MIT | Password vault |
| encoding_rs | =0.8.42 | Text encodings in the editor (Windows-1251, KOI8-R, UTF-16, Shift_JIS, GBK …) | github.com/hsivonen/encoding_rs | (Apache-2.0 OR MIT) AND BSD-3-Clause | Opening and saving files that are not UTF-8 (implementation of the WHATWG Encoding standard, the same code as in Firefox) |
| chardetng | =1.0.0 | Automatic file encoding detection | github.com/hsivonen/chardetng | Apache-2.0 OR MIT | A Windows-1251/KOI8-R file opens without questions; same author and standard as encoding_rs |
| zeroize | 1.9.1 | Wiping secrets in memory | github.com/RustCrypto/utils | Apache-2.0 OR MIT | Passwords and keys |
| getrandom | 0.4.3 | OS random number generator | github.com/rust-random/getrandom | MIT OR Apache-2.0 | Salts, nonces, askpass token, temporary file names |
| hmac, sha1 | 0.13.0, 0.11.0 | Matching hashed `known_hosts` entries (HMAC-SHA1) | github.com/RustCrypto/MACs, /hashes | MIT OR Apache-2.0 | OpenSSH known_hosts format |
| sha2 | 0.11.1 | SHA-256 of the file version in the editor | github.com/RustCrypto/hashes | MIT OR Apache-2.0 | Detecting that a file changed on the server |
| base64ct | 1.8.3 | Base64 (known_hosts, terminal IPC) | github.com/RustCrypto/formats | Apache-2.0 OR MIT | OpenSSH formats, terminal input |
| serde, serde_json | 1.0.229, 1.0.154 | API and settings serialization | github.com/serde-rs | MIT OR Apache-2.0 | JSON API, settings.json |

Tests only (`dev-dependencies` of devbridge): base64ct, getrandom, hmac, sha1 — generating the TOTP code for the 2FA test.

The `devbridge` crate (HTTP bridge for the e2e tests) is not part of the Windows exe.

## npm: direct UI dependencies (`ui/`, end up in the exe)

| Package | Version | Purpose | Repository | License |
|---|---|---|---|---|
| @xterm/xterm | 6.0.0 | Terminal | github.com/xtermjs/xterm.js | MIT |
| @xterm/addon-fit, -search, -serialize, -unicode11, -web-links, -webgl | 0.11.0, 0.16.0, 0.14.0, 0.9.0, 0.12.0, 0.19.0 | Size, search, screen snapshot (terminal duplication), Unicode 11, links, WebGL | github.com/xtermjs/xterm.js | MIT |
| @tauri-apps/api | 2.12.4 | Tauri IPC, window, drag-drop event | github.com/tauri-apps/tauri | Apache-2.0 OR MIT |
| @tauri-apps/plugin-opener | 2.7.2 | Opening links | github.com/tauri-apps/plugins-workspace | MIT OR Apache-2.0 |
| @tauri-apps/plugin-dialog | 2.8.1 | File picker | github.com/tauri-apps/plugins-workspace | MIT OR Apache-2.0 |
| @codemirror/state, view, commands, language, search | 6.7.6, 6.43.13, 6.11.1, 6.12.4, 6.7.2 | Editor (core, input, history, highlighting, search/replace) | code.haverbeke.berlin/codemirror (the official repository of the CodeMirror author; mirror github.com/codemirror) | MIT |
| @codemirror/merge | 6.12.2 | Diff view for a save conflict | same | MIT |
| @codemirror/lang-javascript, -json, -yaml, -xml, -html, -css, -python, -php, -sql, -markdown | see `ui/package.json` | Syntax highlighting | github.com/codemirror | MIT |
| @codemirror/legacy-modes | 6.5.4 | sh/bash, nginx, toml, properties (ini/conf/systemd), Dockerfile | code.haverbeke.berlin/codemirror | MIT |
| @lezer/highlight | 1.2.5 | Highlight tags for the editor theme | code.haverbeke.berlin/lezer | MIT |

Build-time dev dependencies (not in the exe): `typescript` 7.0.2, `vite` 8.3.5, `@playwright/test` 1.64.1 (`e2e/`, tests only).

Note: the `@codemirror/*` repositories were moved by their author (Marijn Haverbeke) to his own server `code.haverbeke.berlin`;
this is reflected in `repository.url` in the npm package metadata, and the owner of the packages on npm is unchanged.

## Tools (dev container only)

`cargo-xwin`, `tauri-cli`, `cargo-audit`, `cargo-deny`, `cargo-about` — `cargo install --locked` from crates.io; the `rust`, `node` and `debian` images
are pinned by digest (`.devcontainer/Dockerfile`). The MSVC CRT/SDK comes through `cargo-xwin` from Microsoft servers.
Nothing is installed on Windows. `ssh.exe` is not bundled: the system Windows OpenSSH is used.

## Checks at build time

Run by `scripts/check-all.sh`:

- `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok (graph for the Windows target only). Exception: RUSTSEC-2023-0071 (`rsa`,
  Marvin attack, no fix available) — pulled in by `russh`; d256 Shell performs no private RSA operations (password login only, host keys are only verified),
  the justification is in `deny.toml` and `.cargo/audit.toml`.
- `cargo audit`: 0 vulnerabilities (same exception).
- `npm audit --audit-level=moderate`: `ui/` (production) — 0 vulnerabilities; `e2e/` — 0 vulnerabilities.
- Dependency licenses are permissive only (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0 …); no copyleft.
  The full license texts are embedded in the exe (About → Library licenses) and generated by `scripts/gen-licenses.mjs`.

## Network

The application makes no network requests except SSH to the user's servers: no updates, telemetry or crash reports.
The frontend is fully embedded (no CDNs or remote fonts), CSP `default-src 'self'`.
