#!/bin/bash
R=/home/dev/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f
S=$R/russh-0.65.0/src
F=$R/russh-sftp-3.0.1/src
echo "=== lib.rs (keys etc)"; grep -n "keys\|ssh_key\|pub use" $S/lib.rs | head -20
echo "=== PublicKeyOrCertificate"; grep -rn "enum PublicKeyOrCertificate" -B2 -A8 $S | head -20
echo "=== sftp de.rs string"; grep -n "utf8\|from_utf8\|String" $F/de.rs | head -20
echo "=== sftp Metadata"; grep -rn "pub struct Metadata" -A12 $F/client | head -30; grep -n "pub fn" $F/client/fs/file.rs | head -3
ls $F/client/fs
echo "=== FileAttributes"; grep -n "pub struct FileAttributes" -A25 $F/protocol/attrs.rs | head -45; grep -n "pub fn" $F/protocol/attrs.rs | head -30
echo "=== raw session fns"; grep -n "pub async fn\|pub fn" $F/client/rawsession.rs | head -60
echo "=== russh keys crate dependency"; grep -n "^ssh-key\|^russh-keys\|^internal-russh-forked-ssh-key\|^\[dependencies" -A0 $R/russh-0.65.0/Cargo.toml | head; ls $R | grep -i "ssh-key\|russh"
