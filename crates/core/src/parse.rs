//! Turns a captured Ethernet frame into a [`PacketHeader`].
//!
//! Live capture asks Npcap for at most [`SNAPLEN`] bytes per packet, enough
//! for Ethernet + IP + UDP/TCP headers, so most payload never reaches us. The
//! parser below reads header fields only; it never looks at whatever payload
//! bytes happen to fit inside the snapshot.

use std::net::{IpAddr, SocketAddr};

use etherparse::{LaxNetSlice, LaxSlicedPacket, TransportSlice};

use crate::capture::{Direction, PacketHeader, Protocol, RemoteAddr};

/// Bytes captured per packet. Worst common case: Ethernet (14) + one VLAN
/// tag (4) + IPv4 with options (60) + TCP with options (60) = 138. IPv6 (40)
/// with a small extension header also fits. A header cut short is dropped.
pub const SNAPLEN: u32 = 160;

/// The PC's own IP addresses on the capture adapter. Used to tell inbound
/// from outbound, so we know which side is "remote".
#[derive(Clone, Default)]
pub struct LocalAddrs(Vec<IpAddr>);

/// Hand-written: this PC's own (often globally unique IPv6) address is
/// identifying, so it never prints.
impl std::fmt::Debug for LocalAddrs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LocalAddrs(<{} redacted>)", self.0.len())
    }
}

impl LocalAddrs {
    pub fn new(addrs: impl IntoIterator<Item = IpAddr>) -> Self {
        Self(addrs.into_iter().collect())
    }

    fn contains(&self, ip: IpAddr) -> bool {
        self.0.contains(&ip)
    }
}

/// Parses one frame. Returns `None` for anything that isn't UDP/TCP over IP,
/// is too short, or isn't to/from this PC.
///
/// `wire_len` is the packet's original length on the wire (from the capture
/// header), not the truncated snapshot length.
pub fn parse_frame(
    frame: &[u8],
    wire_len: u32,
    timestamp_us: u64,
    local: &LocalAddrs,
) -> Option<PacketHeader> {
    // "Lax" slicing tolerates truncated packets, which every snapshot is.
    let packet = LaxSlicedPacket::from_ethernet(frame).ok()?;

    let (src_ip, dst_ip, ttl) = match packet.net? {
        LaxNetSlice::Ipv4(ip) => {
            let h = ip.header();
            (
                IpAddr::V4(h.source_addr()),
                IpAddr::V4(h.destination_addr()),
                h.ttl(),
            )
        }
        LaxNetSlice::Ipv6(ip) => {
            let h = ip.header();
            (
                IpAddr::V6(h.source_addr()),
                IpAddr::V6(h.destination_addr()),
                h.hop_limit(),
            )
        }
        LaxNetSlice::Arp(_) => return None,
    };

    let (protocol, src_port, dst_port) = match packet.transport? {
        TransportSlice::Udp(udp) => (Protocol::Udp, udp.source_port(), udp.destination_port()),
        TransportSlice::Tcp(tcp) => (Protocol::Tcp, tcp.source_port(), tcp.destination_port()),
        _ => return None,
    };

    let (direction, local_port, remote_ip, remote_port) = if local.contains(src_ip) {
        (Direction::Outbound, src_port, dst_ip, dst_port)
    } else if local.contains(dst_ip) {
        (Direction::Inbound, dst_port, src_ip, src_port)
    } else {
        return None;
    };

    Some(PacketHeader {
        timestamp_us,
        direction,
        protocol,
        local_port,
        remote: RemoteAddr::new(SocketAddr::new(remote_ip, remote_port)),
        wire_len,
        ttl: Some(ttl),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use etherparse::PacketBuilder;
    use std::net::{Ipv4Addr, Ipv6Addr};

    const ME: [u8; 4] = [192, 168, 1, 20];
    const SERVER: [u8; 4] = [203, 0, 113, 7];
    const MAC_A: [u8; 6] = [2, 0, 0, 0, 0, 1];
    const MAC_B: [u8; 6] = [2, 0, 0, 0, 0, 2];

    fn local() -> LocalAddrs {
        LocalAddrs::new([IpAddr::V4(Ipv4Addr::from(ME))])
    }

    fn udp_v4(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        PacketBuilder::ethernet2(MAC_A, MAC_B)
            .ipv4(src, dst, 64)
            .udp(sport, dport)
            .write(&mut out, payload)
            .unwrap();
        out
    }

    /// What live capture hands us: the frame cut to SNAPLEN.
    fn snap(frame: &[u8]) -> &[u8] {
        &frame[..frame.len().min(SNAPLEN as usize)]
    }

    #[test]
    fn outbound_udp() {
        let frame = udp_v4(ME, SERVER, 50_000, 6000, &[0; 40]);
        let h = parse_frame(&frame, frame.len() as u32, 7, &local()).unwrap();
        assert_eq!(h.direction, Direction::Outbound);
        assert_eq!(h.protocol, Protocol::Udp);
        assert_eq!(h.local_port, 50_000);
        assert_eq!(h.remote.get(), "203.0.113.7:6000".parse().unwrap());
        assert_eq!(h.timestamp_us, 7);
    }

    #[test]
    fn inbound_udp_swaps_sides() {
        let frame = udp_v4(SERVER, ME, 6000, 50_000, &[0; 40]);
        let h = parse_frame(&frame, frame.len() as u32, 0, &local()).unwrap();
        assert_eq!(h.direction, Direction::Inbound);
        assert_eq!(h.local_port, 50_000);
        assert_eq!(h.ttl, Some(64), "built with TTL 64");
        assert_eq!(h.remote.get().port(), 6000);
    }

    #[test]
    fn truncated_snapshot_still_parses_and_keeps_wire_len() {
        let frame = udp_v4(ME, SERVER, 50_000, 6000, &[0xAB; 1200]);
        let wire_len = frame.len() as u32;
        let h = parse_frame(snap(&frame), wire_len, 0, &local()).unwrap();
        assert_eq!(
            h.wire_len, wire_len,
            "length comes from the wire, not the snapshot"
        );
    }

    #[test]
    fn tcp_is_parsed() {
        let mut frame = Vec::new();
        PacketBuilder::ethernet2(MAC_A, MAC_B)
            .ipv4(ME, SERVER, 64)
            .tcp(51_000, 443, 1, 1024)
            .write(&mut frame, &[])
            .unwrap();
        let h = parse_frame(&frame, frame.len() as u32, 0, &local()).unwrap();
        assert_eq!(h.protocol, Protocol::Tcp);
        assert_eq!(h.local_port, 51_000);
    }

    #[test]
    fn ipv6_is_parsed() {
        let me6: Ipv6Addr = "2001:db8::20".parse().unwrap();
        let remote6: Ipv6Addr = "2001:db8::7".parse().unwrap();
        let mut frame = Vec::new();
        PacketBuilder::ethernet2(MAC_A, MAC_B)
            .ipv6(me6.octets(), remote6.octets(), 64)
            .udp(50_000, 6000)
            .write(&mut frame, &[0; 8])
            .unwrap();
        let local = LocalAddrs::new([IpAddr::V6(me6)]);
        let h = parse_frame(&frame, frame.len() as u32, 0, &local).unwrap();
        assert_eq!(h.remote.get().ip(), IpAddr::V6(remote6));
    }

    #[test]
    fn traffic_not_involving_this_pc_is_ignored() {
        let frame = udp_v4([10, 0, 0, 1], SERVER, 1, 2, &[]);
        assert!(parse_frame(&frame, frame.len() as u32, 0, &local()).is_none());
    }

    #[test]
    fn garbage_and_short_frames_are_ignored() {
        assert!(parse_frame(&[], 0, 0, &local()).is_none());
        assert!(parse_frame(&[0xFF; 10], 10, 0, &local()).is_none());
        let frame = udp_v4(ME, SERVER, 1, 2, &[]);
        assert!(parse_frame(&frame[..20], 60, 0, &local()).is_none());
    }
}
