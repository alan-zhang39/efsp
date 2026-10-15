//! Pure logic for efsp. Nothing in this crate touches the network, the
//! firewall, the registry or the filesystem directly: real I/O lives behind
//! traits (such as [`capture::PacketSource`]) or in the Tauri app crate, so
//! everything here can be unit-tested on any OS.

pub mod capture;
pub mod classify;
pub mod flow;
pub mod parse;
pub mod ports;
pub mod trace;
