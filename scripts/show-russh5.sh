#!/bin/bash
R=/home/dev/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f
S=$R/russh-0.65.0/src
F=$R/russh-sftp-3.0.1/src
echo "=== FileAttributes"; sed -n 1,140p $F/protocol/file_attrs.rs | grep -n "pub \|impl\|fn " | head -60
echo "=== russh keys mod"; ls $S/keys* 2>/dev/null | head; grep -rn "pub use" $S/keys/mod.rs 2>/dev/null | head -20; grep -n "pub mod keys\|mod keys\|use .*keys" $S/lib.rs
echo "=== Preferred keys default"; grep -n "pub const DEFAULT" -B2 -A16 $S/negotiation.rs | grep -n "key\|KEY" | head
