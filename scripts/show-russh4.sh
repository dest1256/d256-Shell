#!/bin/bash
R=/home/dev/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f
S=$R/russh-0.65.0/src
F=$R/russh-sftp-3.0.1/src
echo "=== cert.rs imports"; sed -n 1,9p $S/cert.rs
echo "=== russh lib.rs pub items"; grep -n "^pub \|^    pub use\|^pub(crate)" $S/lib.rs | head -50
echo "=== sftp de string"; grep -rn "String" $F/de.rs $F/buf.rs | head -20
echo "=== sftp client/fs/mod.rs Metadata"; sed -n 1,80p $F/client/fs/mod.rs
echo "=== protocol dir"; ls $F/protocol; grep -n "pub struct FileAttributes" -A22 $F/protocol/file.rs
echo "=== FileAttributes impl names"; grep -n "pub fn" $F/protocol/file.rs | head -30
