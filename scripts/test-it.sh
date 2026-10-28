#!/bin/bash
# Integration tests against the docker test servers (dev container only). Args: cargo test filter / flags.
cd "$(dirname "$0")/.."
cargo test -p d256shell-core --test '*' "$@" -- --test-threads=1 2>&1 | grep -v "^ *Compiling\|^ *Downloaded\|^ *Downloading" | head -${LINES_MAX:-150}
