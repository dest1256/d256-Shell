#!/bin/bash
# The 512 MB round trip of the task (10.3): upload + download, SHA-256 compared. Slow, run on demand (dev container).
cd "$(dirname "$0")/.."
D256SHELL_BIG=1 cargo test --locked -p d256shell-core --test sftp_xfer large_upload -- --test-threads=1 --nocapture 2>&1 | grep -E "^test |test result|panicked"
