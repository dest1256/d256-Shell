#!/bin/bash
# Compile d256shell-core and show diagnostics only (dev container).
cd "$(dirname "$0")/.."
cargo build -p d256shell-core --all-targets 2>&1 | grep -v "^ *Compiling\|^ *Downloaded\|^ *Downloading" | head -${LINES_MAX:-120}
