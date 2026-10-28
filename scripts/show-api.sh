#!/bin/bash
# Print key API facts of freshly resolved crates from the local registry sources.
R=$(ls -d /home/dev/.cargo/registry/src/*/ | head -1)
echo "--- argon2 lib.rs usage examples"; grep -n "hash_password_into\|Params::new\|Argon2::new\|pub fn new" $R/argon2-0.6.0/src/lib.rs $R/argon2-0.6.0/src/params.rs | head -12
echo "--- chacha20poly1305 README example"; sed -n '/```rust/,/```/p' $R/chacha20poly1305-0.11.0/README.md | head -40
echo "--- chacha20poly1305 lib exports"; grep -n "pub use\|pub type XChaCha20Poly1305\|XNonce" $R/chacha20poly1305-0.11.0/src/lib.rs | head -20
echo "--- getrandom"; grep -n "pub fn fill\|pub fn u32" $R/getrandom-0.4.3/src/lib.rs | head
