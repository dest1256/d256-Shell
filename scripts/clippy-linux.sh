#!/bin/bash
# Lint of the Tauri crate for Linux (GTK/WebKit dev libraries are in the Ubuntu-based container only):
#   cd .devcontainer && docker compose --profile linux run --rm linux bash scripts/clippy-linux.sh
cd "$(dirname "$0")/.."
rustup component add clippy >/dev/null 2>&1 || true
cargo clippy --locked -p d256shell --features custom-protocol -- -D warnings 2>&1 | grep -v "^ *Compiling\|^ *Checking\|^ *Downloaded\|^ *Downloading"
exit ${PIPESTATUS[0]}
