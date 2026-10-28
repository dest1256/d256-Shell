#!/bin/bash
# One-command build, run inside the dev container:
#   XWIN_ACCEPT_LICENSE=1 scripts/build.sh
# security checks -> tests -> UI -> Windows exe -> dist/ -> hashes
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== security checks"
if [ -f deny.toml ]; then cargo deny check; else echo "(deny.toml not yet present, skipped)"; fi
cargo audit
(cd ui && npm audit --omit=dev --audit-level=moderate)

echo "== tests"
cargo test --locked --workspace --exclude d256shell

echo "== UI"
(cd ui && npm ci && npm run build)

echo "== Windows exe"
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --no-bundle -- --locked
EXE=${CARGO_TARGET_DIR:-target}/x86_64-pc-windows-msvc/release/d256shell.exe

echo "== dist"
rm -rf dist/d256Shell dist/d256Shell.zip
mkdir -p dist/d256Shell/data
cp "$EXE" dist/d256Shell/d256Shell.exe
(cd dist && zip -qr d256Shell.zip d256Shell && sha256sum d256Shell/d256Shell.exe d256Shell.zip > SHA256SUMS.txt)
cat dist/SHA256SUMS.txt
