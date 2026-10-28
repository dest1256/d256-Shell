//! d256 Shell core: all logic lives here, independent of Tauri.
pub mod hub;
pub mod keyfile;
pub mod known_hosts;
pub mod logging;
pub mod monitor;
pub mod paths;
pub mod prompt;
pub mod remote_path;
pub mod service;
pub mod settings;
pub mod sftp;
pub mod terminal;
pub mod transfers;
pub mod vault;
