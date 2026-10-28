#!/bin/bash
# Stage 3 dependencies for d256shell-core.
cd "$(dirname "$0")/.."
cargo add -p d256shell-core russh russh-sftp
cargo add -p d256shell-core tokio --features rt-multi-thread,net,time,sync,macros,io-util
cargo add -p d256shell-core hmac sha1 base64ct --features base64ct/alloc
for c in russh russh-sftp tokio hmac sha1 base64ct; do
  echo "== $c"; cargo info "$c" 2>/dev/null | grep -E "^(version|license|repository):"
done
grep -A20 "^\[dependencies\]" crates/core/Cargo.toml
