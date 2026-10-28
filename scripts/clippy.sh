#!/bin/bash
# Lint: core (+ integration tests) on Linux, the Tauri app on the Windows target (GTK deps intentionally absent).
cd "$(dirname "$0")/.."
noise='^ *Compiling\|^ *Checking\|^ *Downloaded\|^ *Downloading\|LNK4099\|failed to load reference'
rc=0
cargo fmt --all --check || rc=1
cargo clippy --locked -p d256shell-core -p d256shell-devbridge --all-targets -- -D warnings 2>&1 | grep -v "$noise"
[ "${PIPESTATUS[0]}" = 0 ] || rc=1
XWIN_ACCEPT_LICENSE="${XWIN_ACCEPT_LICENSE:-1}" cargo xwin clippy --locked --target x86_64-pc-windows-msvc -p d256shell -- -D warnings 2>&1 | grep -v "$noise"
[ "${PIPESTATUS[0]}" = 0 ] || rc=1
echo "clippy rc=$rc"
exit $rc
