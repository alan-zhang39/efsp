//! Pure logic for efsp. Nothing in this crate touches the network, the
//! firewall, the registry or the filesystem directly: real I/O lives behind
//! traits (such as [`capture::PacketSource`]) or in the Tauri app crate, so
//! everything here can be unit-tested on any OS.

pub mod bars;
pub mod capture;
pub mod cascade;
pub mod classify;
pub mod firewall;
pub mod flow;
pub mod gameping;
pub mod history;
pub mod measure;
pub mod npcap;
pub mod parse;
pub mod picker;
#[cfg(feature = "dev-tools")]
pub mod pingexe;
pub mod ports;
pub mod session;
pub mod trace;
