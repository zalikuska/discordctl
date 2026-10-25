//! Общее ядро discordctl: пути, протокол локального API, IPC, правила шима.

pub mod args;
pub mod client;
pub mod config;
pub mod handler;
pub mod ipc;
pub mod paths;
pub mod permissions;
pub mod protocol;
pub mod shim;
