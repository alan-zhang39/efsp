//! Packet capture abstraction.
//!
//! The app reads packets through [`PacketSource`] so the same flow/measurement
//! code can run on live Npcap capture (Windows) or on a recorded trace
//! (replay, used on macOS and in tests).

use std::fmt;
use std::net::SocketAddr;

/// One captured packet, reduced to header facts and timing.
///
/// There is deliberately no payload field: the type itself makes it
/// impossible to carry packet contents through the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketHeader {
    /// Capture timestamp in microseconds since the Unix epoch.
    pub timestamp_us: u64,
    pub direction: Direction,
    pub protocol: Protocol,
    /// The game's local port this packet was seen on.
    pub local_port: u16,
    /// The remote end. Opponent addresses must stay in memory only.
    pub remote: RemoteAddr,
    /// Length on the wire, in bytes (headers + payload size, never contents).
    pub wire_len: u32,
    /// IPv4 TTL / IPv6 hop limit as received: a header field that counts down
    /// at each router, so it tells us how many hops away the sender is.
    /// `None` for traces recorded before it was captured (trace v1).
    pub ttl: Option<u8>,
}

/// Estimated router hops between the sender and us, from a received TTL.
///
/// Senders start at one of a few well-known values (Windows 128, Linux and
/// macOS 64, some routers 255, a few devices 32); each router subtracts one.
/// The nearest start value at or above the TTL is almost always the real one.
pub fn hops_from_ttl(ttl: u8) -> u8 {
    let start = [32u8, 64, 128, 255]
        .into_iter()
        .find(|s| *s >= ttl)
        .unwrap_or(255);
    start - ttl
}

/// A remote address that never prints itself.
///
/// `Debug` and `Display` show `<redacted>`, so a log line, `dbg!`, panic
/// message or failed test can't leak an opponent's IP. Code that
/// truly needs the address (probing, hashing) calls [`RemoteAddr::get`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RemoteAddr(SocketAddr);

impl RemoteAddr {
    pub fn new(addr: SocketAddr) -> Self {
        Self(addr)
    }

    pub fn get(&self) -> SocketAddr {
        self.0
    }
}

impl fmt::Debug for RemoteAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RemoteAddr(<redacted>)")
    }
}

impl fmt::Display for RemoteAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Inbound,
    Outbound,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Udp,
    Tcp,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CaptureError {
    /// The capture driver or adapter failed (live capture).
    #[error("capture failed: {0}")]
    Device(String),
    /// A trace file couldn't be read or is malformed (replay).
    #[error("trace line {line}: {reason}")]
    BadTrace { line: usize, reason: String },
}

/// What a [`PacketSource`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Packet(PacketHeader),
    /// Nothing arrived within the source's read timeout (live capture). Lets
    /// the caller refresh the UI and the capture filter between packets.
    Idle,
    /// The source is finished (end of a replay trace).
    End,
}

/// A stream of packet headers. Read-only by design: there is no method to
/// send, drop, delay or modify a packet.
///
/// Implementations: `trace::ReplaySource` here in core, and the Npcap-backed
/// live source in the app crate (it needs real I/O, which core avoids).
pub trait PacketSource {
    fn next(&mut self) -> Result<Next, CaptureError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_addresses_never_print() {
        let header = PacketHeader {
            timestamp_us: 1,
            direction: Direction::Inbound,
            protocol: Protocol::Udp,
            local_port: 50_000,
            remote: RemoteAddr::new("203.0.113.7:6000".parse().unwrap()),
            wire_len: 120,
            ttl: Some(57),
        };
        let printed = format!("{header:?} {}", header.remote);
        assert!(!printed.contains("203.0.113.7"), "{printed}");
        assert_eq!(header.remote.get().port(), 6000);
    }

    #[test]
    fn hops_from_common_start_values() {
        assert_eq!(hops_from_ttl(57), 7); // started at 64
        assert_eq!(hops_from_ttl(116), 12); // started at 128
        assert_eq!(hops_from_ttl(64), 0); // same subnet
        assert_eq!(hops_from_ttl(240), 15); // started at 255
        assert_eq!(hops_from_ttl(30), 2); // started at 32
    }
}
