//! The game's own server pings, read passively out of the
//! capture. A real trace shows one game socket sending a small packet
//! to ~60 servers every ~5 s, each answered within tens to hundreds of ms.
//! Pairing request and reply gives an RTT per server without sending any
//! probes of our own. Checked against ICMP on a real home network (within 0–5 ms,
//! `docs/experiments/game-pings.md`); the Servers page shows them
//! per region. The ICMP comparison itself stays a dev-build tool.
//!
//! Pairing rule: an outbound packet on a (local port, remote) that had been
//! quiet for at least [`QUIET_BEFORE_US`], answered by an inbound packet on
//! the same pair within [`REPLY_WITHIN_US`]. Busy flows (like the match,
//! ~50 pps each way) never satisfy "quiet before", so they don't pollute it.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

use serde::Serialize;

use crate::bars::BarThresholds;
use crate::capture::{Direction, PacketHeader, Protocol, RemoteAddr};
use crate::classify::Endpoint;

pub const QUIET_BEFORE_US: u64 = 1_000_000;
pub const REPLY_WITHIN_US: u64 = 1_000_000;
/// A (local port, remote) silent this long is forgotten, address and all
/// (addresses live in memory only while in use). The game pings
/// each server every ~5 s, so active servers stay.
pub const FORGET_AFTER_US: u64 = 30_000_000;
/// RTT samples kept per server.
const KEEP: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamePingsView {
    /// Servers with at least one paired ping.
    pub servers: u32,
    pub samples: u32,
    /// The lowest per-server median: roughly "the nearest server".
    pub nearest_ms: Option<f64>,
    /// Median across servers' medians.
    pub typical_ms: Option<f64>,
    /// The same pings grouped by city, fastest first. Filled by the caller via [`GamePings::regions`].
    pub regions: Vec<RegionPing>,
}

/// The game's ping to one region (or one kind of endpoint).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionPing {
    pub endpoint: Endpoint,
    /// Servers in this region with paired pings.
    pub servers: u32,
    /// Fastest server's median: what a match there would roughly get.
    pub best_ms: f64,
    /// efsp bars for that ping.
    pub bars: u8,
}

#[derive(Default)]
pub struct GamePings {
    /// Per (local port, remote): last outbound time, whether it still awaits
    /// a reply, and whether the pair ever turned busy.
    last_out: HashMap<(u16, RemoteAddr), Out>,
    /// Per (local port, remote): recent RTTs in µs, for pairs that never
    /// turned busy. Memory only, never shown.
    rtts: HashMap<(u16, RemoteAddr), VecDeque<u32>>,
    last_prune_us: u64,
}

struct Out {
    sent_us: u64,
    waiting: bool,
    busy: bool,
}

/// Hand-written: holds real server addresses.
impl std::fmt::Debug for GamePings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GamePings")
            .field("servers", &self.rtts.len())
            .finish_non_exhaustive()
    }
}

impl GamePings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&mut self, h: &PacketHeader) {
        if h.protocol != Protocol::Udp {
            return;
        }
        self.prune(h.timestamp_us);
        let key = (h.local_port, h.remote);
        match h.direction {
            Direction::Outbound => {
                let previous = self.last_out.get(&key);
                let busy = previous.is_some_and(|o| {
                    o.busy || h.timestamp_us.saturating_sub(o.sent_us) < QUIET_BEFORE_US
                });
                if busy {
                    // Packets < 1 s apart: a data flow (like the match), not
                    // a ping exchange. Drop anything it contributed.
                    self.rtts.remove(&key);
                }
                self.last_out.insert(
                    key,
                    Out {
                        sent_us: h.timestamp_us,
                        waiting: !busy,
                        busy,
                    },
                );
            }
            Direction::Inbound => {
                if let Some(out) = self.last_out.get_mut(&key)
                    && out.waiting
                {
                    out.waiting = false;
                    let rtt = h.timestamp_us.saturating_sub(out.sent_us);
                    if rtt <= REPLY_WITHIN_US {
                        let samples = self.rtts.entry(key).or_default();
                        if samples.len() == KEEP {
                            samples.pop_front();
                        }
                        samples.push_back(rtt as u32);
                    }
                }
            }
        }
    }

    /// Forgets pairs that have been silent for [`FORGET_AFTER_US`]. Runs at
    /// most once a second.
    fn prune(&mut self, now_us: u64) {
        if now_us.saturating_sub(self.last_prune_us) < 1_000_000 {
            return;
        }
        self.last_prune_us = now_us;
        self.last_out
            .retain(|_, o| now_us.saturating_sub(o.sent_us) < FORGET_AFTER_US);
        let live = &self.last_out;
        self.rtts.retain(|key, _| live.contains_key(key));
    }

    /// Pairs currently remembered (for tests and the memory bound).
    pub fn remembered(&self) -> usize {
        self.last_out.len()
    }

    fn medians_us(&self) -> Vec<(IpAddr, f64)> {
        let mut out: Vec<(IpAddr, f64)> = self
            .rtts
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|((_, remote), v)| {
                let mut s: Vec<u32> = v.iter().copied().collect();
                s.sort_unstable();
                let n = s.len();
                let m = if n % 2 == 1 {
                    f64::from(s[n / 2])
                } else {
                    (f64::from(s[n / 2 - 1]) + f64::from(s[n / 2])) / 2.0
                };
                (remote.get().ip(), m)
            })
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out
    }

    /// The `n` servers with the lowest median, for cross-checking with ICMP.
    /// Addresses stay inside Rust.
    #[cfg(feature = "dev-tools")]
    pub fn nearest(&self, n: usize) -> Vec<(IpAddr, f64)> {
        self.medians_us()
            .into_iter()
            .take(n)
            .map(|(ip, us)| (ip, (us / 100.0).round() / 10.0))
            .collect()
    }

    pub fn view(&self) -> GamePingsView {
        let medians = self.medians_us();
        let ms = |us: f64| (us / 100.0).round() / 10.0;
        let typical = (!medians.is_empty()).then(|| {
            let n = medians.len();
            if n % 2 == 1 {
                medians[n / 2].1
            } else {
                (medians[n / 2 - 1].1 + medians[n / 2].1) / 2.0
            }
        });
        GamePingsView {
            servers: medians.len() as u32,
            samples: self.rtts.values().map(|v| v.len() as u32).sum(),
            nearest_ms: medians.first().map(|m| ms(m.1)),
            typical_ms: typical.map(ms),
            regions: Vec::new(),
        }
    }

    /// Per-region pings. `endpoint` classifies an address (the flow
    /// tracker's cached [`crate::flow::FlowTracker::endpoint_for`]).
    pub fn regions(
        &self,
        endpoint: &mut dyn FnMut(IpAddr) -> Endpoint,
        thresholds: &BarThresholds,
    ) -> Vec<RegionPing> {
        let ms = |us: f64| (us / 100.0).round() / 10.0;
        let mut out: Vec<RegionPing> = Vec::new();
        // Medians come fastest first, so a region's first server is its best.
        for (ip, median) in self.medians_us() {
            // Grouped by city: Google and AWS in Frankfurt are one Frankfurt.
            let e = endpoint(ip);
            let area = e.area_key();
            match out.iter_mut().find(|r| r.endpoint.area_key() == area) {
                Some(r) => r.servers += 1,
                None => out.push(RegionPing {
                    endpoint: e,
                    servers: 1,
                    best_ms: ms(median),
                    bars: crate::bars::bars(ms(median), 0.0, 0.0, thresholds),
                }),
            }
        }
        out
    }
}

#[cfg(feature = "dev-tools")]
/// One row of the experiment: the game's own ping to a server next to ours.
/// Servers are numbered, never named or addressed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerComparison {
    pub server: u32,
    pub game_ms: f64,
    /// Median of our ICMP replies; `None` if the server ignored every probe.
    pub icmp_ms: Option<f64>,
    pub icmp_replies: u32,
    /// Game minus ICMP.
    pub difference_ms: Option<f64>,
}

/// Builds the rows from the game medians (nearest first) and, per server,
/// our ICMP results in µs (`None` = lost).
#[cfg(feature = "dev-tools")]
pub fn compare_rows(game_ms: &[f64], icmp_us: &[Vec<Option<u32>>]) -> Vec<ServerComparison> {
    game_ms
        .iter()
        .zip(icmp_us)
        .enumerate()
        .map(|(i, (game, probes))| {
            let mut replies: Vec<u32> = probes.iter().flatten().copied().collect();
            replies.sort_unstable();
            let n = replies.len();
            let icmp_ms = (n > 0).then(|| {
                let us = if n % 2 == 1 {
                    f64::from(replies[n / 2])
                } else {
                    (f64::from(replies[n / 2 - 1]) + f64::from(replies[n / 2])) / 2.0
                };
                (us / 100.0).round() / 10.0
            });
            ServerComparison {
                server: i as u32 + 1,
                game_ms: *game,
                icmp_ms,
                icmp_replies: n as u32,
                difference_ms: icmp_ms.map(|icmp| ((game - icmp) * 10.0).round() / 10.0),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(t: u64, dir: Direction, remote: &str) -> PacketHeader {
        PacketHeader {
            timestamp_us: t,
            direction: dir,
            protocol: Protocol::Udp,
            local_port: 53_376,
            remote: RemoteAddr::new(remote.parse().unwrap()),
            wire_len: 44,
            ttl: None,
        }
    }

    #[test]
    fn pairs_sparse_request_and_reply() {
        let mut g = GamePings::new();
        for round in 0..3u64 {
            let t = round * 5_000_000;
            g.observe(&pkt(t, Direction::Outbound, "203.0.113.4:5521"));
            g.observe(&pkt(t + 40_000, Direction::Inbound, "203.0.113.4:5521"));
            g.observe(&pkt(t, Direction::Outbound, "203.0.113.9:5521"));
            g.observe(&pkt(t + 120_000, Direction::Inbound, "203.0.113.9:5521"));
        }
        let v = g.view();
        assert_eq!((v.servers, v.samples), (2, 6));
        assert_eq!(v.nearest_ms, Some(40.0));
        assert_eq!(v.typical_ms, Some(80.0));
        #[cfg(feature = "dev-tools")]
        assert_eq!(g.nearest(1)[0].1, 40.0);
    }

    #[test]
    fn regions_group_servers_and_keep_the_fastest() {
        use crate::classify::Endpoint;
        let mut g = GamePings::new();
        for (remote, rtt) in [
            ("203.0.113.4:1", 40_000),
            ("203.0.113.9:1", 25_000),
            ("198.51.100.1:1", 90_000),
        ] {
            g.observe(&pkt(0, Direction::Outbound, remote));
            g.observe(&pkt(rtt, Direction::Inbound, remote));
        }
        // Stand-in classifier: 203.0.113.x is region europe-west3, 198.51.100.x
        // eu-central-1. Both are Frankfurt, so they make one row.
        let mut classify = |ip: IpAddr| {
            let region = if ip.to_string().starts_with("203.") {
                "europe-west3"
            } else {
                "eu-central-1"
            };
            Endpoint::Server {
                region: region.into(),
                place: crate::classify::place(region),
            }
        };
        let regions = g.regions(&mut classify, &BarThresholds::default());
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].endpoint.label(), "Frankfurt");
        assert_eq!((regions[0].servers, regions[0].best_ms), (3, 25.0));
        assert!(!serde_json::to_string(&regions).unwrap().contains("203."));
    }

    #[test]
    fn busy_flows_are_ignored() {
        // A match-like flow: out/in every 20 ms. Never "quiet before".
        let mut g = GamePings::new();
        for i in 0..200 {
            g.observe(&pkt(i * 20_000, Direction::Outbound, "203.0.113.71:5735"));
            g.observe(&pkt(
                i * 20_000 + 9_000,
                Direction::Inbound,
                "203.0.113.71:5735",
            ));
        }
        // Its first request paired, but the flow turned busy: excluded.
        assert_eq!(g.view().samples, 0);
    }

    #[test]
    fn late_or_unsolicited_replies_dont_count() {
        let mut g = GamePings::new();
        g.observe(&pkt(0, Direction::Outbound, "203.0.113.4:5521"));
        g.observe(&pkt(1_500_000, Direction::Inbound, "203.0.113.4:5521"));
        g.observe(&pkt(2_000_000, Direction::Inbound, "198.51.100.2:5521"));
        assert_eq!(g.view().samples, 0);
    }

    #[test]
    fn silent_pairs_are_forgotten() {
        let mut g = GamePings::new();
        g.observe(&pkt(0, Direction::Outbound, "203.0.113.4:5521"));
        g.observe(&pkt(30_000, Direction::Inbound, "203.0.113.4:5521"));
        assert_eq!((g.remembered(), g.view().servers), (1, 1));
        // 31 s later another server speaks; the first is forgotten.
        g.observe(&pkt(31_000_000, Direction::Outbound, "203.0.113.9:5521"));
        assert_eq!((g.remembered(), g.view().servers), (1, 0));
    }

    #[test]
    fn never_prints_addresses() {
        let mut g = GamePings::new();
        g.observe(&pkt(0, Direction::Outbound, "203.0.113.4:5521"));
        g.observe(&pkt(30_000, Direction::Inbound, "203.0.113.4:5521"));
        assert!(!format!("{g:?}").contains("203.0.113.4"));
        assert!(
            !serde_json::to_string(&g.view())
                .unwrap()
                .contains("203.0.113.4")
        );
    }

    #[cfg(feature = "dev-tools")]
    #[test]
    fn comparison_rows() {
        let rows = compare_rows(
            &[20.0, 41.9],
            &[
                vec![Some(18_000), Some(19_000), None, Some(21_000)],
                vec![None, None, None],
            ],
        );
        assert_eq!(rows[0].server, 1);
        assert_eq!(rows[0].icmp_ms, Some(19.0));
        assert_eq!(rows[0].icmp_replies, 3);
        assert_eq!(rows[0].difference_ms, Some(1.0));
        assert_eq!((rows[1].icmp_ms, rows[1].difference_ms), (None, None));
    }
}
