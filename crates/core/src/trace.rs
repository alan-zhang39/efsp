//! Header-only traces: record live traffic, replay it on any OS.
//!
//! Format: JSON Lines. The first line is a header
//! `{"format":"efsp-trace","version":2}`; every other line is one packet:
//! `{"t":1500,"d":"inbound","p":"udp","lp":50000,"ra":"203.0.113.1","rp":6000,"len":120,"ttl":57}`
//! (`ttl` is absent in version 1 traces, which still load.)
//! where `t` is microseconds since the first packet.
//!
//! Privacy:
//! - [`TraceWriter`] swaps every remote IP for a reserved documentation
//!   address *before* serialising, so a real IP never reaches the file.
//! - The local IP is never recorded, and times are relative, not wall-clock.
//! - [`TraceReader`] rejects any non-documentation address, so a raw trace
//!   can't sneak into the test fixtures.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::capture::{
    CaptureError, Direction, Next, PacketHeader, PacketSource, Protocol, RemoteAddr,
};

pub const FORMAT: &str = "efsp-trace";
/// v2 added `ttl`. v1 traces still load, with `ttl: None`.
pub const VERSION: u32 = 2;
const OLDEST_READABLE: u32 = 1;

/// IPv4 documentation ranges (RFC 5737). Hosts .1–.254 of each.
const V4_POOLS: [[u8; 3]; 3] = [[203, 0, 113], [198, 51, 100], [192, 0, 2]];
const V4_CAPACITY: usize = V4_POOLS.len() * 254;

#[derive(Debug, Serialize, Deserialize)]
struct FileHeader {
    format: String,
    version: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    t: u64,
    d: Direction,
    p: Protocol,
    lp: u16,
    ra: IpAddr,
    rp: u16,
    len: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ttl: Option<u8>,
}

/// True for addresses reserved for documentation (RFC 5737 / RFC 3849),
/// which never belong to a real host.
pub fn is_documentation(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            V4_POOLS.iter().any(|p| o[..3] == p[..])
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            s[0] == 0x2001 && s[1] == 0x0db8
        }
    }
}

/// Maps real addresses to documentation addresses, consistently within one
/// trace: the same real IP always gets the same stand-in.
#[derive(Default)]
pub struct Anonymiser {
    v4: HashMap<Ipv4Addr, Ipv4Addr>,
    v6: HashMap<Ipv6Addr, Ipv6Addr>,
}

/// Hand-written: the map holds real addresses, so only counts are printed.
impl std::fmt::Debug for Anonymiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Anonymiser")
            .field("v4", &self.v4.len())
            .field("v6", &self.v6.len())
            .finish()
    }
}

impl Anonymiser {
    pub fn map(&mut self, ip: IpAddr) -> Result<IpAddr, TraceError> {
        match ip {
            IpAddr::V4(real) => {
                if let Some(fake) = self.v4.get(&real) {
                    return Ok(IpAddr::V4(*fake));
                }
                let n = self.v4.len();
                if n >= V4_CAPACITY {
                    return Err(TraceError::TooManyRemotes);
                }
                let [a, b, c] = V4_POOLS[n / 254];
                let fake = Ipv4Addr::new(a, b, c, (n % 254) as u8 + 1);
                self.v4.insert(real, fake);
                Ok(IpAddr::V4(fake))
            }
            IpAddr::V6(real) => {
                // Host part n+1 spread over the last 64 bits: never overflows.
                let n = self.v6.len() as u64 + 1;
                let fake = *self.v6.entry(real).or_insert_with(|| {
                    let [a, b, c, d] = [
                        (n >> 48) as u16,
                        (n >> 32) as u16,
                        (n >> 16) as u16,
                        n as u16,
                    ];
                    Ipv6Addr::new(0x2001, 0x0db8, 0, 0, a, b, c, d)
                });
                Ok(IpAddr::V6(fake))
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("more than {V4_CAPACITY} distinct IPv4 remotes in one trace")]
    TooManyRemotes,
    #[error("could not write trace: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not encode trace line: {0}")]
    Encode(#[from] serde_json::Error),
}

/// Writes an anonymised trace. Wrap the file in a `BufWriter`.
pub struct TraceWriter<W: Write> {
    out: W,
    anon: Anonymiser,
    start_us: Option<u64>,
}

impl<W: Write> TraceWriter<W> {
    pub fn new(mut out: W) -> Result<Self, TraceError> {
        let header = FileHeader {
            format: FORMAT.into(),
            version: VERSION,
        };
        serde_json::to_writer(&mut out, &header)?;
        out.write_all(b"\n")?;
        Ok(Self {
            out,
            anon: Anonymiser::default(),
            start_us: None,
        })
    }

    pub fn write(&mut self, h: &PacketHeader) -> Result<(), TraceError> {
        let start = *self.start_us.get_or_insert(h.timestamp_us);
        let record = Record {
            t: h.timestamp_us.saturating_sub(start),
            d: h.direction,
            p: h.protocol,
            lp: h.local_port,
            ra: self.anon.map(h.remote.get().ip())?,
            rp: h.remote.get().port(),
            len: h.wire_len,
            ttl: h.ttl,
        };
        serde_json::to_writer(&mut self.out, &record)?;
        self.out.write_all(b"\n")?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<W, TraceError> {
        self.out.flush()?;
        Ok(self.out)
    }
}

/// Replays a trace as a [`PacketSource`]. Yields packets as fast as they are
/// read; the caller decides whether to pace them in real time.
pub struct ReplaySource<R: BufRead> {
    lines: std::io::Lines<R>,
    line_no: usize,
    checked_header: bool,
}

/// Alias that reads better at call sites that only parse.
pub type TraceReader<R> = ReplaySource<R>;

impl<R: BufRead> ReplaySource<R> {
    pub fn new(input: R) -> Self {
        Self {
            lines: input.lines(),
            line_no: 0,
            checked_header: false,
        }
    }

    fn bad(&self, reason: impl Into<String>) -> CaptureError {
        CaptureError::BadTrace {
            line: self.line_no,
            reason: reason.into(),
        }
    }

    fn next_line(&mut self) -> Result<Option<String>, CaptureError> {
        loop {
            match self.lines.next() {
                None => return Ok(None),
                Some(Err(e)) => return Err(self.bad(e.to_string())),
                Some(Ok(line)) => {
                    self.line_no += 1;
                    if !line.trim().is_empty() {
                        return Ok(Some(line));
                    }
                }
            }
        }
    }
}

impl<R: BufRead> PacketSource for ReplaySource<R> {
    fn next(&mut self) -> Result<Next, CaptureError> {
        if !self.checked_header {
            let line = self.next_line()?.ok_or_else(|| self.bad("empty trace"))?;
            let header: FileHeader =
                serde_json::from_str(&line).map_err(|_| self.bad("malformed header"))?;
            if header.format != FORMAT || !(OLDEST_READABLE..=VERSION).contains(&header.version) {
                return Err(self.bad(format!(
                    "unsupported trace {} v{}",
                    header.format, header.version
                )));
            }
            self.checked_header = true;
        }

        let Some(line) = self.next_line()? else {
            return Ok(Next::End);
        };
        // Omits serde's message on purpose: it can quote the line.
        let r: Record = serde_json::from_str(&line).map_err(|_| self.bad("malformed line"))?;
        if !is_documentation(r.ra) {
            return Err(self.bad("trace contains a non-anonymised address"));
        }
        Ok(Next::Packet(PacketHeader {
            timestamp_us: r.t,
            direction: r.d,
            protocol: r.p,
            local_port: r.lp,
            remote: RemoteAddr::new(SocketAddr::new(r.ra, r.rp)),
            wire_len: r.len,
            ttl: r.ttl,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(t: u64, ip: &str, port: u16) -> PacketHeader {
        PacketHeader {
            timestamp_us: t,
            direction: Direction::Inbound,
            protocol: Protocol::Udp,
            local_port: 50_000,
            remote: RemoteAddr::new(SocketAddr::new(ip.parse().unwrap(), port)),
            wire_len: 120,
            ttl: Some(57),
        }
    }

    fn record(packets: &[PacketHeader]) -> String {
        let mut w = TraceWriter::new(Vec::new()).unwrap();
        for p in packets {
            w.write(p).unwrap();
        }
        String::from_utf8(w.finish().unwrap()).unwrap()
    }

    fn replay(text: &str) -> Result<Vec<PacketHeader>, CaptureError> {
        let mut src = ReplaySource::new(text.as_bytes());
        let mut out = Vec::new();
        loop {
            match src.next()? {
                Next::Packet(p) => out.push(p),
                Next::Idle => {}
                Next::End => return Ok(out),
            }
        }
    }

    #[test]
    fn real_addresses_never_reach_the_file() {
        let text = record(&[
            pkt(1_000_000, "10.1.2.3", 6000),
            pkt(1_000_500, "fd00::1", 6000),
        ]);
        assert!(!text.contains("10.1.2.3"), "{text}");
        assert!(!text.contains("fd00"), "{text}");
        assert!(text.starts_with(r#"{"format":"efsp-trace","version":2}"#));
    }

    #[test]
    fn same_real_ip_maps_to_same_stand_in_and_times_are_relative() {
        let text = record(&[
            pkt(5_000_000, "10.1.2.3", 6000),
            pkt(5_000_100, "10.1.2.3", 6001),
            pkt(5_000_200, "172.16.5.9", 6000),
        ]);
        let back = replay(&text).unwrap();
        assert_eq!(back[0].remote.get().ip(), back[1].remote.get().ip());
        assert_ne!(back[0].remote.get().ip(), back[2].remote.get().ip());
        assert_eq!(back[1].remote.get().port(), 6001);
        let times: Vec<u64> = back.iter().map(|p| p.timestamp_us).collect();
        assert_eq!(times, [0, 100, 200]);
    }

    #[test]
    fn roundtrip_keeps_everything_but_the_ip() {
        let original = pkt(0, "10.1.2.3", 6000);
        let back = &replay(&record(std::slice::from_ref(&original))).unwrap()[0];
        assert_eq!(back.direction, original.direction);
        assert_eq!(back.protocol, original.protocol);
        assert_eq!(back.local_port, original.local_port);
        assert_eq!(back.wire_len, original.wire_len);
        assert_eq!(back.ttl, Some(57));
        assert!(is_documentation(back.remote.get().ip()));
    }

    #[test]
    fn anonymiser_uses_all_pools_then_refuses() {
        let mut a = Anonymiser::default();
        let mut seen = std::collections::HashSet::new();
        for i in 0..V4_CAPACITY as u32 {
            let fake = a.map(IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i))).unwrap();
            assert!(is_documentation(fake));
            assert!(seen.insert(fake), "stand-ins must be unique");
        }
        assert!(matches!(
            a.map(IpAddr::V4(Ipv4Addr::new(11, 0, 0, 1))),
            Err(TraceError::TooManyRemotes)
        ));
    }

    #[test]
    fn reader_rejects_real_addresses_bad_headers_and_garbage() {
        let header = r#"{"format":"efsp-trace","version":1}"#;
        let real = format!(
            "{header}\n{}",
            r#"{"t":0,"d":"inbound","p":"udp","lp":1,"ra":"10.1.2.3","rp":2,"len":3}"#
        );
        assert!(matches!(
            replay(&real),
            Err(CaptureError::BadTrace { line: 2, .. })
        ));
        assert!(replay("").is_err());
        assert!(replay(r#"{"format":"pcap","version":1}"#).is_err());
        assert!(replay(r#"{"format":"efsp-trace","version":3}"#).is_err());
        assert!(replay(&format!("{header}\nnot json")).is_err());
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = record(&[pkt(0, "10.1.2.3", 6000)]).replace('\n', "\n\n");
        assert_eq!(replay(&text).unwrap().len(), 1);
    }
}
