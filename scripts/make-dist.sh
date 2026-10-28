#!/bin/bash
# Build the Windows exe from the already built UI and refresh dist/ (dev container).
set -e
cd "$(dirname "$0")/.."
export XWIN_ACCEPT_LICENSE="${XWIN_ACCEPT_LICENSE:-1}"
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --no-bundle -- --locked 2>&1 | grep -E "^error|Finished|Built application"
EXE=${CARGO_TARGET_DIR:-target}/x86_64-pc-windows-msvc/release/d256shell.exe
rm -rf dist/d256Shell dist/d256Shell.zip
mkdir -p dist/d256Shell/data dist/win-tests
cp "$EXE" dist/d256Shell/d256Shell.exe
cp scripts/run-win-tests.ps1 dist/win-tests/
(cd dist && zip -qr d256Shell.zip d256Shell && sha256sum d256Shell/d256Shell.exe d256Shell.zip | tee SHA256SUMS.txt)
ls -l dist/d256Shell/d256Shell.exe
