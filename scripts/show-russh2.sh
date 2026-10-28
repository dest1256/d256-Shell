#!/bin/bash
R=/home/dev/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f
S=$R/russh-0.65.0/src
F=$R/russh-sftp-3.0.1/src
echo "=== connect_stream"; sed -n 1185,1230p $S/client/mod.rs
echo "=== Handler check_server_key"; grep -n "fn check_server_key" -B3 -A8 $S/client/mod.rs | head -40
echo "=== Prompt struct / kbdint start/respond"; sed -n 375,430p $S/client/mod.rs
grep -n "pub struct Prompt" -A6 $S/client/mod.rs
echo "=== lib.rs exports"; grep -n "pub use\|pub mod\|extern crate" $S/lib.rs | head -30
echo "=== MethodSet"; grep -n "MethodSet" $S/auth.rs | head -8; grep -n "pub enum MethodKind" -A12 $S/auth.rs
echo "=== Config fields"; grep -n "pub struct Config" -A40 $S/client/mod.rs | grep "pub " | head -30
echo "=== sftp dir entry/metadata"; sed -n 1,80p $F/client/fs/dir.rs
echo "=== sftp metadata"; grep -n "pub fn\|pub struct" $F/client/fs/metadata.rs | head -30
echo "=== sftp FileAttributes"; grep -n "pub " $F/protocol/file.rs | head -50
echo "=== extensions"; grep -n "pub" $F/extensions.rs | head -30
echo "=== session new / limits / extensions on session"; sed -n 30,95p $F/client/session.rs
