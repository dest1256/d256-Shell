#!/bin/bash
# Stage 2 dependencies for d256shell-core (all from crates.io, official RustCrypto / serde).
cd "$(dirname "$0")/.."
cargo add -p d256shell-core argon2 chacha20poly1305 zeroize getrandom serde --features serde/derive
cargo add -p d256shell-core serde_json
for c in argon2 chacha20poly1305 zeroize getrandom serde serde_json; do
  echo "== $c"; cargo info "$c" 2>/dev/null | grep -E "^(version|license|repository|documentation):"
done
grep -A12 "^\[dependencies\]" crates/core/Cargo.toml
