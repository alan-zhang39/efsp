//! Groups packet headers into flows.
//!
//! A flow is (protocol, local port, remote address, remote port). The tracker
//! is pure: it never reads a clock. Callers pass `now_us` explicitly (system
//! time for live capture, trace time for replay), so tests are deterministic.
//!
//! Remote addresses stay inside the tracker. What leaves it ([`FlowView`]) only
//! carries a per-session label like "Remote 3" and the remote port, never the
//! IP.

use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;

use serde::Serialize;

use crate::capture::{Direction, PacketHeader, Protocol, RemoteAddr};
use crate::classify::{Endpoint, GeoLookup, NoGeo, classify};

/// Window used for the packet rate.
pub const RATE_WINDOW_US: u64 = 1_000_000;
/// A flow counts as "sustained" once it is at least this old...
pub const SUSTAINED_MIN_AGE_US: u64 = 1_000_000;
/// ...and has carried at least this many packets per second recently, in
/// both directions. Only UDP flows qualify (see `is_sustained`).
///
/// From a real anonymised trace (`real-two-match-offers.jsonl`): match flows
/// run at ~106 pps; lobby/probe UDP flows stay under 1 pps; HTTPS (TCP 443)
/// bursts briefly reach 13–17 pps, which is why TCP is excluded.
pub const SUSTAINED_MIN_PPS: u32 = 10;
/// TCP matches: a TCP flow qualifies like a UDP
/// one, but only to a port other than these (HTTPS and HTTP carry lobby,
/// store and downloads, which burst past 10 pps)...
pub const TCP_NOT_MATCH_PORTS: [u16; 2] = [443, 80];
/// ...and only once it has been around this long, so short bursts on other
/// ports don't count.
pub const TCP_MIN_AGE_US: u64 = 3_000_000;
/// A flow with no packets for this long is shown as idle.
pub const IDLE_AFTER_US: u64 = 3_000_000;
/// A flow with no packets for this long is dropped.
pub const EXPIRE_AFTER_US: u64 = 30_000_000;
/// Cap on remembered timestamps per flow, so a flood can't grow memory.
const MAX_RECENT: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FlowKey {
    protocol: Protocol,
    local_port: u16,
    remote: RemoteAddr,
}

#[derive(Debug)]
struct Flow {
    id: u32,
    remote_label: u32,
    first_seen_us: u64,
    last_seen_us: u64,
    packets_in: u64,
    packets_out: u64,
    bytes_in: u64,
    bytes_out: u64,
    /// Timestamps of recent packets, oldest first, with direction.
    recent: VecDeque<(u64, Direction)>,
    /// TTL of the latest inbound packet, for the hop-count estimate.
    last_ttl_in: Option<u8>,
}

/// One flow's state, for following the match being measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowStatus {
    /// No packets for [`IDLE_AFTER_US`].
    pub idle: bool,
    /// Match-level traffic (see `match_like`).
    pub sustained: bool,
}

/// The connection to measure: the newest UDP flow with match-level traffic.
/// Holds the remote address for probing; it stays in Rust and its `Debug`
/// is redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub flow_id: u32,
    pub protocol: Protocol,
    pub local_port: u16,
    pub remote: RemoteAddr,
    /// Router hops to the remote, estimated from received TTLs.
    pub hops: Option<u8>,
}

/// One flow as the UI sees it. Contains no IP address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowView {
    /// Stable for the session; use as a React key.
    pub id: u32,
    /// "Remote N": the same remote address always gets the same N this session.
    pub remote_label: u32,
    pub protocol: Protocol,
    pub local_port: u16,
    pub remote_port: u16,
    pub age_ms: u64,
    pub idle_ms: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Packets per second over the last [`RATE_WINDOW_US`], both directions.
    pub rate_pps: u32,
    pub sustained: bool,
    pub idle: bool,
    /// Server region, peer country, local or unclassified.
    pub endpoint: Endpoint,
}

pub struct FlowTracker {
    flows: HashMap<FlowKey, Flow>,
    /// Keyed by IP only, so one server on several ports keeps one label.
    /// Held in memory only, and dropped once no flow uses the address.
    remote_labels: HashMap<IpAddr, u32>,
    /// Classification per address, done once (on the first snapshot that
    /// shows it) and dropped with the label.
    endpoints: HashMap<IpAddr, Endpoint>,
    geo: Arc<dyn GeoLookup + Send + Sync>,
    next_flow_id: u32,
    next_label: u32,
}

impl Default for FlowTracker {
    fn default() -> Self {
        Self::with_geo(Arc::new(NoGeo))
    }
}

/// Hand-written so `{:?}`, `dbg!` or a panic can never print an address.
impl std::fmt::Debug for FlowTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowTracker")
            .field("flows", &self.flows.len())
            .field("remotes", &self.remote_labels.len())
            .finish_non_exhaustive()
    }
}

impl FlowTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// A tracker that labels remotes using `geo` for countries and networks.
    pub fn with_geo(geo: Arc<dyn GeoLookup + Send + Sync>) -> Self {
        Self {
            flows: HashMap::new(),
            remote_labels: HashMap::new(),
            endpoints: HashMap::new(),
            geo,
            next_flow_id: 0,
            next_label: 0,
        }
    }

    /// Classifies an address (cached while a flow uses it).
    pub fn endpoint_for(&mut self, ip: IpAddr) -> Endpoint {
        if let Some(e) = self.endpoints.get(&ip) {
            return e.clone();
        }
        let e = classify(ip, self.geo.as_ref());
        // Only cache addresses with a live flow, so nothing outlives its use.
        if self.remote_labels.contains_key(&ip) {
            self.endpoints.insert(ip, e.clone());
        }
        e
    }

    pub fn observe(&mut self, h: &PacketHeader) {
        let key = FlowKey {
            protocol: h.protocol,
            local_port: h.local_port,
            remote: h.remote,
        };
        let flow = match self.flows.get_mut(&key) {
            Some(flow) => flow,
            None => {
                let remote_label = *self
                    .remote_labels
                    .entry(h.remote.get().ip())
                    .or_insert_with(|| {
                        self.next_label += 1;
                        self.next_label
                    });
                self.next_flow_id += 1;
                self.flows.entry(key).or_insert(Flow {
                    id: self.next_flow_id,
                    remote_label,
                    first_seen_us: h.timestamp_us,
                    last_seen_us: h.timestamp_us,
                    packets_in: 0,
                    packets_out: 0,
                    bytes_in: 0,
                    bytes_out: 0,
                    recent: VecDeque::new(),
                    last_ttl_in: None,
                })
            }
        };

        // Captures can deliver slightly out-of-order timestamps; never go back.
        flow.last_seen_us = flow.last_seen_us.max(h.timestamp_us);
        match h.direction {
            Direction::Inbound => {
                flow.packets_in += 1;
                flow.bytes_in += u64::from(h.wire_len);
            }
            Direction::Outbound => {
                flow.packets_out += 1;
                flow.bytes_out += u64::from(h.wire_len);
            }
        }
        if flow.recent.len() == MAX_RECENT {
            flow.recent.pop_front();
        }
        flow.recent.push_back((h.timestamp_us, h.direction));
        if h.direction == Direction::Inbound && h.ttl.is_some() {
            flow.last_ttl_in = h.ttl;
        }
    }

    /// True while the flow exists and has carried a packet within
    /// [`IDLE_AFTER_US`].
    pub fn is_active(&self, flow_id: u32, now_us: u64) -> bool {
        self.flows
            .values()
            .any(|f| f.id == flow_id && now_us.saturating_sub(f.last_seen_us) < IDLE_AFTER_US)
    }

    /// The flow to measure right now, if any: the newest UDP flow carrying at
    /// least [`SUSTAINED_MIN_PPS`] both ways in the last second. Unlike
    /// `sustained`, there's no minimum age, so probing can start ~1 s sooner.
    pub fn match_candidate(&self, now_us: u64) -> Option<Candidate> {
        self.flows
            .iter()
            .filter(|(key, f)| match_like(key, f, now_us, false))
            // UDP first (the normal case), then the newest.
            .max_by_key(|(key, f)| (key.protocol == Protocol::Udp, f.id))
            .map(|(key, f)| Candidate {
                flow_id: f.id,
                protocol: key.protocol,
                local_port: key.local_port,
                remote: key.remote,
                hops: f.last_ttl_in.map(crate::capture::hops_from_ttl),
            })
    }

    /// Periodic upkeep: drops expired flows, forgets addresses no flow uses
    /// any more, classifies new remotes and trims old packet times. Cheap;
    /// call it on every snapshot, whether or not anyone wants the flow list.
    pub fn tick(&mut self, now_us: u64) {
        self.flows
            .retain(|_, f| now_us.saturating_sub(f.last_seen_us) < EXPIRE_AFTER_US);
        // Forget addresses no flow uses any more (addresses stay in memory
        // only while in use). A returning address gets a new label.
        let flows = &self.flows;
        self.remote_labels
            .retain(|ip, _| flows.keys().any(|k| k.remote.get().ip() == *ip));
        let labels = &self.remote_labels;
        self.endpoints.retain(|ip, _| labels.contains_key(ip));
        let geo = &self.geo;
        for key in self.flows.keys() {
            let ip = key.remote.get().ip();
            self.endpoints
                .entry(ip)
                .or_insert_with(|| classify(ip, geo.as_ref()));
        }
        // Keep enough history for the TCP rule; the rate is the last second.
        let keep_from = now_us.saturating_sub(TCP_MIN_AGE_US);
        for f in self.flows.values_mut() {
            while f.recent.front().is_some_and(|(t, _)| *t < keep_from) {
                f.recent.pop_front();
            }
        }
    }

    /// Whether one flow is idle or sustained right now; `None` once it has
    /// expired. What the monitor needs to follow the match it measures.
    pub fn status(&self, flow_id: u32, now_us: u64) -> Option<FlowStatus> {
        self.flows
            .iter()
            .find(|(_, f)| f.id == flow_id)
            .map(|(key, f)| FlowStatus {
                idle: now_us.saturating_sub(f.last_seen_us) >= IDLE_AFTER_US,
                sustained: match_like(key, f, now_us, true),
            })
    }

    /// Every flow as the UI's connections table sees it, newest first.
    /// Only developer builds show that table; call [`Self::tick`] first.
    pub fn views(&self, now_us: u64) -> Vec<FlowView> {
        let window_start = now_us.saturating_sub(RATE_WINDOW_US);
        let mut views: Vec<FlowView> = self
            .flows
            .iter()
            .map(|(key, f)| {
                let rate = f.recent.iter().filter(|(t, _)| *t >= window_start).count() as u32;
                let age_us = now_us.saturating_sub(f.first_seen_us);
                let idle_us = now_us.saturating_sub(f.last_seen_us);
                FlowView {
                    id: f.id,
                    remote_label: f.remote_label,
                    protocol: key.protocol,
                    local_port: key.local_port,
                    remote_port: key.remote.get().port(),
                    age_ms: age_us / 1000,
                    idle_ms: idle_us / 1000,
                    packets_in: f.packets_in,
                    packets_out: f.packets_out,
                    bytes_in: f.bytes_in,
                    bytes_out: f.bytes_out,
                    rate_pps: rate,
                    sustained: match_like(key, f, now_us, true),
                    idle: idle_us >= IDLE_AFTER_US,
                    endpoint: self
                        .endpoints
                        .get(&key.remote.get().ip())
                        .cloned()
                        .unwrap_or(Endpoint::Unknown {
                            network: None,
                            country: None,
                        }),
                }
            })
            .collect();
        views.sort_by_key(|v| Reverse(v.id));
        views
    }

    /// [`Self::tick`] then [`Self::views`].
    pub fn snapshot(&mut self, now_us: u64) -> Vec<FlowView> {
        self.tick(now_us);
        self.views(now_us)
    }
}

/// At least [`SUSTAINED_MIN_PPS`] packets, both directions, in the 1 s
/// window ending at `end_us`.
fn busy_in(f: &Flow, end_us: u64) -> bool {
    let start = end_us.saturating_sub(RATE_WINDOW_US);
    let (mut n, mut inbound, mut outbound) = (0u32, false, false);
    for (_, d) in f.recent.iter().filter(|(t, _)| *t >= start && *t <= end_us) {
        n += 1;
        match d {
            Direction::Inbound => inbound = true,
            Direction::Outbound => outbound = true,
        }
    }
    n >= SUSTAINED_MIN_PPS && inbound && outbound
}

/// Match-level traffic.
/// - UDP: busy in the last second. Enough to start measuring at once
///   (`for_sustained` false); the "sustained" mark also needs the flow to be
///   [`SUSTAINED_MIN_AGE_US`] old.
/// - TCP: only to non-web ports, and busy in *each* of the last
///   [`TCP_MIN_AGE_US`] worth of seconds, so a long-lived lobby connection
///   that bursts briefly doesn't count. Same for both uses.
fn match_like(key: &FlowKey, f: &Flow, now_us: u64, for_sustained: bool) -> bool {
    match key.protocol {
        Protocol::Udp => {
            let old_enough = now_us.saturating_sub(f.first_seen_us) >= SUSTAINED_MIN_AGE_US;
            (!for_sustained || old_enough) && busy_in(f, now_us)
        }
        Protocol::Tcp => {
            let windows = TCP_MIN_AGE_US / RATE_WINDOW_US;
            !TCP_NOT_MATCH_PORTS.contains(&key.remote.get().port())
                && now_us >= TCP_MIN_AGE_US
                && (0..windows).all(|k| busy_in(f, now_us - k * RATE_WINDOW_US))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    const SEC: u64 = 1_000_000;

    fn pkt(t_us: u64, dir: Direction, remote: &str) -> PacketHeader {
        PacketHeader {
            timestamp_us: t_us,
            direction: dir,
            protocol: Protocol::Udp,
            local_port: 50_000,
            remote: RemoteAddr::new(remote.parse::<SocketAddr>().unwrap()),
            wire_len: 100,
            ttl: None,
        }
    }

    /// `pps` packets per second, alternating directions, from `start` for `secs`.
    fn steady(t: &mut FlowTracker, remote: &str, start: u64, secs: u64, pps: u64) {
        let step = SEC / pps;
        let mut now = start;
        let mut i = 0;
        while now < start + secs * SEC {
            let dir = if i % 2 == 0 {
                Direction::Outbound
            } else {
                Direction::Inbound
            };
            t.observe(&pkt(now, dir, remote));
            now += step;
            i += 1;
        }
    }

    #[test]
    fn counts_packets_and_bytes_per_direction() {
        let mut t = FlowTracker::new();
        t.observe(&pkt(0, Direction::Outbound, "203.0.113.7:6000"));
        t.observe(&pkt(10, Direction::Inbound, "203.0.113.7:6000"));
        t.observe(&pkt(20, Direction::Inbound, "203.0.113.7:6000"));
        let v = t.snapshot(20);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].packets_out, v[0].packets_in), (1, 2));
        assert_eq!((v[0].bytes_out, v[0].bytes_in), (100, 200));
        assert_eq!(v[0].remote_port, 6000);
    }

    #[test]
    fn different_remote_ports_are_different_flows_but_same_label() {
        let mut t = FlowTracker::new();
        t.observe(&pkt(0, Direction::Outbound, "203.0.113.7:6000"));
        t.observe(&pkt(1, Direction::Outbound, "203.0.113.7:6001"));
        t.observe(&pkt(2, Direction::Outbound, "198.51.100.9:6000"));
        let v = t.snapshot(2);
        assert_eq!(v.len(), 3);
        let labels: Vec<u32> = v.iter().map(|f| f.remote_label).collect();
        assert_eq!(labels, [2, 1, 1], "newest first; same IP keeps its label");
    }

    #[test]
    fn status_follows_the_flow_until_it_expires() {
        let mut t = FlowTracker::new();
        steady(&mut t, "203.0.113.7:6000", 0, 2, 30);
        t.tick(2 * SEC);
        let id = t.views(2 * SEC)[0].id;
        let busy = FlowStatus {
            idle: false,
            sustained: true,
        };
        assert_eq!(t.status(id, 2 * SEC), Some(busy));
        let quiet = t.status(id, 2 * SEC + IDLE_AFTER_US).unwrap();
        assert!(quiet.idle && !quiet.sustained);
        t.tick(2 * SEC + EXPIRE_AFTER_US);
        assert_eq!(t.status(id, 2 * SEC + EXPIRE_AFTER_US), None);
        assert!(t.views(2 * SEC + EXPIRE_AFTER_US).is_empty());
    }

    #[test]
    fn steady_two_way_traffic_becomes_sustained() {
        let mut t = FlowTracker::new();
        steady(&mut t, "203.0.113.7:6000", 0, 2, 30);
        let v = t.snapshot(2 * SEC);
        assert!(v[0].sustained, "{v:?}");
        assert!((28..=31).contains(&v[0].rate_pps), "{}", v[0].rate_pps);
    }

    #[test]
    fn young_or_one_way_or_slow_flows_are_not_sustained() {
        let mut young = FlowTracker::new();
        steady(&mut young, "203.0.113.7:6000", 0, 1, 30);
        assert!(!young.snapshot(SEC / 2)[0].sustained);

        let mut one_way = FlowTracker::new();
        for i in 0..60 {
            one_way.observe(&pkt(i * SEC / 30, Direction::Outbound, "203.0.113.7:6000"));
        }
        assert!(!one_way.snapshot(2 * SEC)[0].sustained);

        let mut slow = FlowTracker::new();
        steady(&mut slow, "203.0.113.7:6000", 0, 3, 4);
        assert!(!slow.snapshot(3 * SEC)[0].sustained);
    }

    #[test]
    fn tcp_bursts_are_never_sustained() {
        // Real traces show HTTPS downloads briefly at 13–17 pps.
        let mut t = FlowTracker::new();
        let mut now = 0;
        for i in 0..60 {
            let dir = if i % 2 == 0 {
                Direction::Outbound
            } else {
                Direction::Inbound
            };
            let mut p = pkt(now, dir, "203.0.113.7:443");
            p.protocol = Protocol::Tcp;
            t.observe(&p);
            now += SEC / 30;
        }
        let v = t.snapshot(now);
        assert!(v[0].rate_pps >= SUSTAINED_MIN_PPS && !v[0].sustained);
    }

    #[test]
    fn flows_go_idle_then_expire() {
        let mut t = FlowTracker::new();
        steady(&mut t, "203.0.113.7:6000", 0, 2, 30);
        let v = t.snapshot(2 * SEC + IDLE_AFTER_US);
        assert!(v[0].idle && !v[0].sustained && v[0].rate_pps == 0);
        assert!(t.snapshot(2 * SEC + EXPIRE_AFTER_US).is_empty());
    }

    #[test]
    fn out_of_order_timestamps_do_not_rewind_last_seen() {
        let mut t = FlowTracker::new();
        t.observe(&pkt(5 * SEC, Direction::Inbound, "203.0.113.7:6000"));
        t.observe(&pkt(4 * SEC, Direction::Inbound, "203.0.113.7:6000"));
        assert_eq!(t.snapshot(5 * SEC)[0].idle_ms, 0);
    }

    #[test]
    fn expired_remotes_are_forgotten_and_never_reuse_a_label() {
        let mut t = FlowTracker::new();
        t.observe(&pkt(0, Direction::Inbound, "203.0.113.7:6000"));
        assert!(t.snapshot(EXPIRE_AFTER_US).is_empty());
        assert_eq!(format!("{t:?}"), "FlowTracker { flows: 0, remotes: 0, .. }");

        t.observe(&pkt(
            EXPIRE_AFTER_US,
            Direction::Inbound,
            "198.51.100.9:6000",
        ));
        assert_eq!(t.snapshot(EXPIRE_AFTER_US)[0].remote_label, 2);
    }

    #[test]
    fn candidate_is_the_newest_busy_udp_flow_without_waiting_for_age() {
        let mut t = FlowTracker::new();
        steady(&mut t, "198.51.100.9:6000", 0, 3, 30); // older busy flow
        // A newer one: 30 pps for half a second, with TTL 57 inbound.
        let mut now = 2 * SEC + SEC / 2;
        for i in 0..15 {
            let dir = if i % 2 == 0 {
                Direction::Outbound
            } else {
                Direction::Inbound
            };
            let mut p = pkt(now, dir, "203.0.113.7:5735");
            p.ttl = Some(57);
            t.observe(&p);
            now += SEC / 30;
        }
        let c = t.match_candidate(now).unwrap();
        assert_eq!(c.remote.get().port(), 5735);
        assert_eq!(c.hops, Some(7));
        assert!(
            !t.snapshot(now)[0].sustained,
            "not old enough to be sustained yet"
        );
        assert!(!format!("{c:?}").contains("203.0.113.7"));
    }

    #[test]
    fn is_active_follows_idle() {
        let mut t = FlowTracker::new();
        t.observe(&pkt(0, Direction::Inbound, "203.0.113.7:6000"));
        let id = t.snapshot(0)[0].id;
        assert!(t.is_active(id, IDLE_AFTER_US - 1));
        assert!(!t.is_active(id, IDLE_AFTER_US));
        assert!(!t.is_active(id + 1, 0));
    }

    #[test]
    fn quiet_or_tcp_flows_are_not_candidates() {
        let mut t = FlowTracker::new();
        steady(&mut t, "203.0.113.7:6000", 0, 3, 4);
        assert_eq!(t.match_candidate(3 * SEC), None);
        let mut tcp = FlowTracker::new();
        for i in 0..60 {
            let dir = if i % 2 == 0 {
                Direction::Outbound
            } else {
                Direction::Inbound
            };
            let mut p = pkt(i * SEC / 30, dir, "203.0.113.7:443");
            p.protocol = Protocol::Tcp;
            tcp.observe(&p);
        }
        assert_eq!(tcp.match_candidate(2 * SEC), None);
    }

    #[test]
    fn views_never_contain_the_ip() {
        let mut t = FlowTracker::new();
        t.observe(&pkt(0, Direction::Inbound, "203.0.113.7:6000"));
        let v = t.snapshot(0);
        let json = serde_json::to_string(&v).unwrap();
        let debug = format!("{v:?} {t:?}");
        assert!(!json.contains("203.0.113.7") && !debug.contains("203.0.113.7"));
    }

    #[test]
    fn flows_carry_their_endpoint_and_forget_it_with_the_address() {
        use crate::classify::Endpoint;
        let mut t = FlowTracker::new();
        t.observe(&pkt(0, Direction::Outbound, "192.168.1.1:53"));
        let views = t.snapshot(SEC);
        assert_eq!(views[0].endpoint, Endpoint::Local);
        assert_eq!(t.endpoints.len(), 1);
        assert!(t.snapshot(SEC + EXPIRE_AFTER_US).is_empty());
        assert!(t.endpoints.is_empty(), "classification outlived the flow");
        // Not cached when no flow uses the address.
        let _ = t.endpoint_for("192.168.1.2".parse().unwrap());
        assert!(t.endpoints.is_empty());
    }

    fn tcp(t_us: u64, dir: Direction, remote: &str) -> PacketHeader {
        PacketHeader {
            protocol: Protocol::Tcp,
            ..pkt(t_us, dir, remote)
        }
    }

    /// `pps` TCP packets per second, alternating directions.
    fn steady_tcp(t: &mut FlowTracker, remote: &str, secs: u64, pps: u64) {
        let step = SEC / pps;
        for i in 0..secs * pps {
            let dir = if i % 2 == 0 {
                Direction::Outbound
            } else {
                Direction::Inbound
            };
            t.observe(&tcp(i * step, dir, remote));
        }
    }

    #[test]
    fn a_steady_tcp_flow_on_a_game_port_is_a_possible_match() {
        let mut t = FlowTracker::new();
        steady_tcp(&mut t, "203.0.113.5:5735", 4, 60);
        let c = t.match_candidate(4 * SEC).expect("TCP match candidate");
        assert_eq!(c.protocol, Protocol::Tcp);
        assert!(t.snapshot(4 * SEC)[0].sustained);
    }

    #[test]
    fn web_traffic_and_short_tcp_bursts_are_not_matches() {
        let mut t = FlowTracker::new();
        steady_tcp(&mut t, "203.0.113.5:443", 10, 60);
        steady_tcp(&mut t, "203.0.113.6:80", 10, 60);
        assert!(t.match_candidate(10 * SEC).is_none());

        let mut t = FlowTracker::new();
        steady_tcp(&mut t, "203.0.113.7:5735", 2, 60); // only 2 s old
        assert!(t.match_candidate(2 * SEC).is_none());
    }

    #[test]
    fn a_long_lived_tcp_connection_bursting_briefly_is_not_a_match() {
        let mut t = FlowTracker::new();
        // A lobby connection open for a minute at 1 packet/s...
        for i in 0..60 {
            let dir = if i % 2 == 0 {
                Direction::Outbound
            } else {
                Direction::Inbound
            };
            t.observe(&tcp(i * SEC, dir, "203.0.113.5:5735"));
        }
        // ...then one busy second.
        for i in 0..40 {
            let dir = if i % 2 == 0 {
                Direction::Outbound
            } else {
                Direction::Inbound
            };
            t.observe(&tcp(60 * SEC + i * 25_000, dir, "203.0.113.5:5735"));
        }
        assert!(t.match_candidate(61 * SEC).is_none());
    }

    #[test]
    fn udp_wins_over_tcp() {
        let mut t = FlowTracker::new();
        steady_tcp(&mut t, "203.0.113.5:5735", 4, 60);
        steady(&mut t, "203.0.113.9:6000", 0, 4, 60);
        let c = t.match_candidate(4 * SEC).unwrap();
        assert_eq!(c.protocol, Protocol::Udp);
    }
}
