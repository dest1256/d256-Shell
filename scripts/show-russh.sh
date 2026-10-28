#!/bin/bash
R=/home/dev/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f
S=$R/russh-0.65.0/src
F=$R/russh-sftp-3.0.1/src
echo "=== russh client: pub fns/handler"; grep -n "pub async fn \(connect\|connect_stream\|authenticate_[a-z_]*\|keyboard_interactive_[a-z_]*\|channel_open_session\|best_supported_rsa_hash\)\|async fn check_server_key\|pub enum KeyboardInteractiveAuthResponse\|pub enum AuthResult\|pub struct Config\|keepalive\|inactivity_timeout\|preferred" $S/client/mod.rs | head -40
echo "=== AuthResult / methods"; grep -rn "pub enum AuthResult" -A12 $S | head -30
echo "=== kbdint response"; grep -rn "pub enum KeyboardInteractiveAuthResponse" -A14 $S | head -30
echo "=== russh keys re-export"; grep -n "pub use\|pub mod keys" $S/lib.rs | head -20
echo "=== sftp client"; ls $F $F/client 2>/dev/null; grep -n "pub async fn" $F/client/session.rs | head -50
