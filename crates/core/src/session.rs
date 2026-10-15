//! One measurement session per match connection: the probe cascade, the
//! passive numbers and a timeline for the live graph, for the flow the
//! tracker picked.
//!
//! The session holds the remote address so probes can be sent; what leaves
//! it ([`MeasurementView`], [`Point`]) carries numbers only.

use std::collections::VecDeque;
use std::net::IpAddr;

use serde::Serialize;

use crate::bars::{BarThresholds, bars};
use crate::capture::{Direction, PacketHeader, Protocol};
use crate::cascade::{Cascade, Method, Probe, ProbeResult, Totals};
use crate::classify::Endpoint;
use crate::flow::Candidate;
use crate::measure::{PassiveStats, PassiveView, ProbeView, whole_match};

/// The live graph shows this much (the last 60 s).
pub const LIVE_WINDOW_MS: u64 = 60_000;
/// Timeline cap: 2 hours at 4 points/s. A match is ~10–15 min.
const MAX_POINTS: usize = 4 * 60 * 60 * 2;
/// Shortest interval a point's packet rate is worked out over: the first tick
/// can come a few ms after the session starts, and 1 packet in 2 ms would
/// read as 500 packets/s.
const MIN_RATE_SPAN_S: f64 = 0.25;

/// One point on the graph, added every snapshot tick (~250 ms).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    /// Milliseconds since the session started.
    pub t_ms: u64,
    /// RTT of the newest reply since the previous point; `None` if none came.
    pub rtt_ms: Option<f64>,
    /// Probe jitter (RFC 3550) when pinging; `None` without probe numbers, so
    /// the graph never draws inbound gap jitter as if it were ping jitter.
    pub jitter_ms: Option<f64>,
    /// Probes lost since the previous point.
    pub lost: u32,
    /// Stalls (inbound gaps > 150 ms) since the previous point.
    pub stalls: u32,
    /// Inbound packets per second since the previous point.
    pub rate_pps: f64,
}

/// What the UI shows for the match connection. No addresses.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementView {
    /// Which flow in the list these numbers belong to.
    pub flow_id: u32,
    /// Always shown next to the numbers.
    pub method: Method,
    /// RTT numbers from probes; `None` while searching or when passive.
    pub ping: Option<ProbeView>,
    pub passive: PassiveView,
    /// False when this target was never probed at all (replay, IPv6), as
    /// opposed to probed with no answer.
    pub probing: bool,
    /// efsp's bars, 1–5; `None` without a ping number.
    pub bars: Option<u8>,
    pub duration_ms: u64,
    /// The last [`LIVE_WINDOW_MS`] of the timeline, for the live graph.
    pub recent: Vec<Point>,
    /// Server region or peer country; `None` until classified.
    pub endpoint: Option<Endpoint>,
    /// UDP normally; TCP for a possible TCP match.
    pub protocol: Protocol,
}

/// A match that has ended: its final numbers and whole timeline, kept until
/// the next match so the card is there when the player alt-tabs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishedMatch {
    /// Numbers as of the last moment traffic was flowing, with `recent` left
    /// empty (see `timeline`).
    pub summary: MeasurementView,
    pub timeline: Vec<Point>,
    /// Whether the flow ever counted as sustained. Short bursts that were
    /// measured briefly but never sustained aren't real matches.
    pub sustained: bool,
}

#[derive(Debug, Clone, Copy)]
struct Tick {
    at_us: u64,
    totals: Totals,
    arrivals: u64,
    stalls: u64,
}

#[derive(Debug)]
pub struct Session {
    target: Candidate,
    cascade: Cascade,
    passive: PassiveStats,
    probing: bool,
    started_us: u64,
    timeline: VecDeque<Point>,
    last_tick: Option<Tick>,
    /// Numbers at the last tick while traffic was still flowing.
    last_active: Option<MeasurementView>,
    sustained: bool,
    endpoint: Option<Endpoint>,
    /// Every reply RTT this session, in arrival order, for whole-match
    /// numbers when it finishes. ≤ 5/s, so an hour is ~18,000 values.
    all_rtts_us: Vec<u32>,
}

impl Session {
    /// Starts measuring `target` at `now_us`. `can_probe` is false when
    /// probes can't be sent at all (replay on any OS, or an IPv6 target for
    /// now): the session is then passive from the start.
    pub fn new(target: Candidate, can_probe: bool, now_us: u64) -> Self {
        let cascade = if can_probe {
            Cascade::new(target.hops)
        } else {
            Cascade::passive_only()
        };
        Self {
            target,
            cascade,
            passive: PassiveStats::new(),
            probing: can_probe,
            started_us: now_us,
            timeline: VecDeque::new(),
            last_tick: None,
            last_active: None,
            sustained: false,
            endpoint: None,
            all_rtts_us: Vec::new(),
        }
    }

    /// Where the target is (the caller classifies; the session never
    /// looks addresses up itself).
    pub fn set_endpoint(&mut self, endpoint: Endpoint) {
        self.endpoint = Some(endpoint);
    }

    pub fn flow_id(&self) -> u32 {
        self.target.flow_id
    }

    pub fn protocol(&self) -> Protocol {
        self.target.protocol
    }

    /// Records that the flow tracker considered this flow sustained.
    pub fn mark_sustained(&mut self) {
        self.sustained = true;
    }

    pub fn method(&self) -> Method {
        self.cascade.method()
    }

    /// The address being measured, for the dev-build `ping.exe` cross-check.
    /// Never shown, logged or sent to the UI.
    pub fn target_ip(&self) -> IpAddr {
        self.target.remote.get().ip()
    }

    /// Feed every captured header; only the target flow's inbound packets
    /// count.
    pub fn observe(&mut self, h: &PacketHeader) {
        if h.direction == Direction::Inbound
            && h.protocol == self.target.protocol
            && h.local_port == self.target.local_port
            && h.remote == self.target.remote
        {
            self.passive.inbound(h.timestamp_us);
        }
    }

    /// The next probe to send now, with its destination.
    pub fn next_probe(&mut self, now_us: u64) -> Option<(Probe, IpAddr)> {
        let probe = self.cascade.next_probe(now_us)?;
        Some((probe, self.target.remote.get().ip()))
    }

    pub fn on_result(&mut self, probe: Probe, result: ProbeResult) {
        let before = self.cascade.totals().replies;
        self.cascade.on_result(probe, result);
        let after = self.cascade.totals();
        // Count only replies the cascade accepted (not stale ones).
        if after.replies > before
            && let Some(rtt) = after.last_rtt_us
        {
            self.all_rtts_us.push(rtt);
        }
    }

    /// Adds one timeline point covering everything since the previous tick.
    pub fn tick(&mut self, now_us: u64) {
        let totals = self.cascade.totals();
        let (arrivals, stalls) = self.passive.totals();
        let prev = self.last_tick.unwrap_or(Tick {
            at_us: self.started_us,
            totals: Totals::default(),
            arrivals: 0,
            stalls: 0,
        });
        let dt_s = (now_us.saturating_sub(prev.at_us) as f64 / 1e6).max(MIN_RATE_SPAN_S);

        // RTT only if a new reply arrived since the previous point.
        let rtt_ms = if totals.replies > prev.totals.replies {
            totals.last_rtt_us.map(|us| f64::from(us) / 1000.0)
        } else {
            None
        };

        let view = self.view_with(now_us, Vec::new());
        let point = Point {
            t_ms: now_us.saturating_sub(self.started_us) / 1000,
            rtt_ms,
            jitter_ms: view.ping.as_ref().map(|p| p.jitter_ms),
            lost: (totals.lost - prev.totals.lost) as u32,
            stalls: (stalls - prev.stalls) as u32,
            rate_pps: ((arrivals - prev.arrivals) as f64 / dt_s * 10.0).round() / 10.0,
        };
        if self.timeline.len() == MAX_POINTS {
            self.timeline.pop_front();
        }
        self.timeline.push_back(point);
        self.last_tick = Some(Tick {
            at_us: now_us,
            totals,
            arrivals,
            stalls,
        });

        // The final "last match" numbers come from the last tick in which
        // packets still arrived, not from the ~3 s of silence before the flow
        // is declared idle.
        if arrivals > prev.arrivals {
            self.last_active = Some(view);
        }
    }

    fn view_with(&mut self, now_us: u64, recent: Vec<Point>) -> MeasurementView {
        let ping = self.cascade.view(now_us);
        let thresholds = BarThresholds::default();
        let bars = ping.as_ref().and_then(|p| {
            p.median_ms
                .map(|median| bars(median, p.jitter_ms, p.loss_pct, &thresholds))
        });
        MeasurementView {
            flow_id: self.target.flow_id,
            method: self.cascade.method(),
            ping,
            passive: self.passive.view(now_us),
            probing: self.probing,
            bars,
            duration_ms: now_us.saturating_sub(self.started_us) / 1000,
            recent,
            endpoint: self.endpoint.clone(),
            protocol: self.target.protocol,
        }
    }

    pub fn view(&mut self, now_us: u64) -> MeasurementView {
        let cutoff = now_us.saturating_sub(self.started_us) / 1000;
        let cutoff = cutoff.saturating_sub(LIVE_WINDOW_MS);
        let from = self.timeline.partition_point(|p| p.t_ms < cutoff);
        let recent = self.timeline.range(from..).copied().collect();
        self.view_with(now_us, recent)
    }

    /// The whole timeline so far.
    pub fn timeline(&self) -> Vec<Point> {
        self.timeline.iter().copied().collect()
    }

    /// Ends the session. Duration and passive figures are as of the last
    /// moment traffic was flowing (the idle tail is dropped); ping numbers
    /// and bars cover the WHOLE match, not the live card's last 10 s. The
    /// address is dropped with the session.
    pub fn finish(mut self, now_us: u64) -> FinishedMatch {
        let mut summary = match self.last_active.take() {
            Some(view) => view,
            None => self.view_with(now_us, Vec::new()),
        };
        let lost = self.cascade.totals().lost;
        if let Some(ping) = whole_match(&self.all_rtts_us, lost) {
            summary.bars = ping.median_ms.map(|median| {
                bars(
                    median,
                    ping.jitter_ms,
                    ping.loss_pct,
                    &BarThresholds::default(),
                )
            });
            summary.ping = Some(ping);
        }
        let end_ms = summary.duration_ms;
        let timeline = self
            .timeline
            .into_iter()
            .filter(|p| p.t_ms <= end_ms)
            .collect();
        FinishedMatch {
            summary,
            timeline,
            sustained: self.sustained,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{Protocol, RemoteAddr};

    fn target() -> Candidate {
        Candidate {
            flow_id: 7,
            protocol: Protocol::Udp,
            local_port: 50_000,
            remote: RemoteAddr::new("203.0.113.7:5735".parse().unwrap()),
            hops: Some(9),
        }
    }

    fn inbound(t: u64, remote: &str) -> PacketHeader {
        PacketHeader {
            timestamp_us: t,
            direction: Direction::Inbound,
            protocol: Protocol::Udp,
            local_port: 50_000,
            remote: RemoteAddr::new(remote.parse().unwrap()),
            wire_len: 120,
            ttl: Some(55),
        }
    }

    #[test]
    fn probes_the_target_and_reports_icmp_numbers() {
        let mut s = Session::new(target(), true, 0);
        let (probe, ip) = s.next_probe(0).unwrap();
        assert_eq!(ip, "203.0.113.7".parse::<IpAddr>().unwrap());
        s.on_result(probe, ProbeResult::EchoReply { rtt_us: 42_000 });
        let v = s.view(0);
        assert_eq!((v.flow_id, v.method), (7, Method::Icmp));
        assert_eq!(v.ping.unwrap().median_ms, Some(42.0));
    }

    #[test]
    fn passive_numbers_come_only_from_the_target_flow() {
        let mut s = Session::new(target(), true, 0);
        for i in 0..50 {
            s.observe(&inbound(i * 20_000, "203.0.113.7:5735")); // 50 pps
            s.observe(&inbound(i * 20_000 + 1, "198.51.100.9:5735")); // other flow
        }
        assert_eq!(s.view(1_000_000).passive.rate_pps, 50.0);
    }

    #[test]
    fn without_probing_it_is_passive_from_the_start() {
        let mut s = Session::new(target(), false, 0);
        assert_eq!(s.next_probe(0), None);
        let v = s.view(0);
        assert_eq!(
            (v.method, v.ping, v.probing),
            (Method::Passive, None, false)
        );
    }

    #[test]
    fn view_never_contains_the_address() {
        let mut s = Session::new(target(), true, 0);
        let json = serde_json::to_string(&s.view(0)).unwrap();
        assert!(!json.contains("203.0.113.7") && !format!("{s:?}").contains("203.0.113.7"));
    }

    #[test]
    fn ticks_build_a_timeline_with_deltas() {
        let mut s = Session::new(target(), true, 0);
        // Tick 1 (250 ms): one reply of 30 ms, 12 inbound packets.
        let p1 = s.next_probe(0).unwrap().0;
        s.on_result(p1, ProbeResult::EchoReply { rtt_us: 30_000 });
        for i in 0..12 {
            s.observe(&inbound(i * 20_000, "203.0.113.7:5735"));
        }
        s.tick(250_000);
        // Tick 2 (500 ms): one lost probe, no reply, 13 packets.
        let p2 = s.next_probe(250_000).unwrap().0;
        s.on_result(p2, ProbeResult::Lost);
        for i in 12..25 {
            s.observe(&inbound(i * 20_000, "203.0.113.7:5735"));
        }
        s.tick(500_000);

        let t = s.timeline();
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].t_ms, t[0].rtt_ms, t[0].lost), (250, Some(30.0), 0));
        assert_eq!(t[0].rate_pps, 48.0);
        assert_eq!((t[1].t_ms, t[1].rtt_ms, t[1].lost), (500, None, 1));
        assert_eq!(t[1].rate_pps, 52.0);
    }

    #[test]
    fn view_carries_bars_and_only_the_last_minute() {
        let mut s = Session::new(target(), true, 0);
        let mut now = 0;
        while now <= 90_000_000 {
            if let Some((p, _)) = s.next_probe(now) {
                s.on_result(p, ProbeResult::EchoReply { rtt_us: 24_000 });
            }
            s.tick(now);
            now += 250_000;
        }
        let v = s.view(90_000_000);
        assert_eq!(v.bars, Some(5));
        assert_eq!(v.duration_ms, 90_000);
        assert!(v.recent.first().unwrap().t_ms >= 30_000);
        assert!((239..=241).contains(&v.recent.len()), "{}", v.recent.len());
    }

    #[test]
    fn finishing_keeps_the_match_not_the_idle_tail() {
        let mut s = Session::new(target(), true, 0);
        // 5 s of traffic at 50 pps, ticking every 250 ms...
        let mut now = 0;
        while now <= 5_000_000 {
            s.observe(&inbound(now, "203.0.113.7:5735"));
            if now % 250_000 == 0 {
                s.tick(now);
            }
            now += 20_000;
        }
        // ...then 3 s of silence before the flow is declared idle.
        for i in 1..=12 {
            s.tick(5_000_000 + i * 250_000);
        }
        let done = s.finish(8_000_000);
        // Duration and numbers stop where the traffic stopped.
        let d = done.summary.duration_ms;
        assert!((4_750..=5_000).contains(&d), "{d}");
        assert!(
            done.summary.passive.longest_gap_ms < 100.0,
            "{:?}",
            done.summary.passive
        );
        assert!(done.timeline.iter().all(|p| p.t_ms <= d));
        assert!(done.summary.recent.is_empty());
        assert!(!done.sustained, "never marked");
        assert!(
            !serde_json::to_string(&done)
                .unwrap()
                .contains("203.0.113.7")
        );
    }

    #[test]
    fn first_point_rate_does_not_spike() {
        let mut s = Session::new(target(), true, 1_000_000);
        s.observe(&inbound(1_001_000, "203.0.113.7:5735"));
        s.tick(1_002_000); // 2 ms after the session started
        assert_eq!(
            s.timeline()[0].rate_pps,
            4.0,
            "1 packet over the 0.25 s floor"
        );
    }

    #[test]
    fn jitter_is_only_probe_jitter() {
        let mut s = Session::new(target(), false, 0); // passive: no probes
        for i in 0..20 {
            s.observe(&inbound(i * 20_000, "203.0.113.7:5735"));
        }
        s.tick(400_000);
        assert_eq!(s.timeline()[0].jitter_ms, None);

        let mut p = Session::new(target(), true, 0);
        let probe = p.next_probe(0).unwrap().0;
        p.on_result(probe, ProbeResult::EchoReply { rtt_us: 30_000 });
        p.tick(250_000);
        assert_eq!(p.timeline()[0].jitter_ms, Some(0.0));
    }

    #[test]
    fn finished_numbers_cover_the_whole_match_not_the_last_ten_seconds() {
        let mut s = Session::new(target(), true, 0);
        let mut now = 0;
        // 60 s at 120 ms, then 20 s at 20 ms (the live window would only see these).
        while now < 80_000_000 {
            s.observe(&inbound(now, "203.0.113.7:5735"));
            if let Some((p, _)) = s.next_probe(now) {
                let rtt = if now < 60_000_000 { 120_000 } else { 20_000 };
                s.on_result(p, ProbeResult::EchoReply { rtt_us: rtt });
            }
            if now % 250_000 == 0 {
                s.tick(now);
            }
            now += 50_000;
        }
        assert_eq!(
            s.view(now).ping.unwrap().median_ms,
            Some(20.0),
            "live: last 10 s"
        );
        let done = s.finish(now);
        let ping = done.summary.ping.unwrap();
        assert_eq!(ping.median_ms, Some(120.0), "saved: whole match");
        assert_eq!(done.summary.bars, Some(2));
    }
}
