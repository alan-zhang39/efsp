//! Chooses how to measure a match connection. Pure state machine:
//! the caller asks for the next probe, sends it, and reports what came back.
//!
//! ```text
//! Icmp ──10 lost, no reply ever──► RouteSearch ──hop answers──► RouteEstimate
//!   ▲                                  │                           │
//!   └──── destination answered ────────┤                10 lost in a row
//!                                      └──search exhausted──► Passive ◄┘
//! ```
//!
//! - **Icmp:** echo requests to the remote itself.
//! - **RouteSearch:** the remote doesn't answer, so look for the last router
//!   before it that does. A probe with TTL = n expires at the n-th router,
//!   which replies "TTL expired". Starting just below the remote's hop count
//!   (from received TTLs, see [`crate::capture::hops_from_ttl`]) usually finds
//!   it in 1–3 probes.
//! - **RouteEstimate:** RTT to that router, labelled "≈ ping".
//! - **Passive:** no probes at all; only the capture-based numbers.
//!
//! The rate cap (≤ 5 probes/s) is enforced here.

use serde::Serialize;

use crate::measure::{ProbeOutcome, ProbeStats, ProbeView};

/// At most one probe per 200 ms: ≤ 5/s.
pub const PROBE_INTERVAL_US: u64 = 200_000;
/// Consecutive losses (with no reply ever) before giving up on a method.
pub const GIVE_UP_AFTER_LOSSES: u32 = 10;
/// When the hop count is unknown, the route search starts here.
pub const MAX_TTL: u8 = 30;
/// How long a probe's result can take to arrive (prober timeout 1 s + slack).
pub const RESULT_TIMEOUT_US: u64 = 1_200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    Icmp,
    RouteSearch,
    RouteEstimate,
    Passive,
}

/// A probe to send. `ttl: None` = normal echo; `Some(n)` = expire at hop n.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// Identifies the method generation; results from an older one are ignored.
    pub epoch: u32,
    pub sent_us: u64,
    pub ttl: Option<u8>,
}

/// What came back for a probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResult {
    /// The destination itself answered.
    EchoReply { rtt_us: u32 },
    /// A router on the way answered "TTL expired".
    TtlExpired { rtt_us: u32 },
    /// No answer within the timeout.
    Lost,
}

/// Running counts over the whole session, across method changes. They only
/// ever go up, so "what happened since last time" is a plain subtraction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub replies: u64,
    pub lost: u64,
    /// RTT of the most recent reply.
    pub last_rtt_us: Option<u32>,
}

#[derive(Debug)]
pub struct Cascade {
    totals: Totals,
    method: Method,
    epoch: u32,
    stats: ProbeStats,
    losses_in_a_row: u32,
    ever_replied: bool,
    last_sent_us: Option<u64>,
    /// Next TTL to try during RouteSearch; the TTL in use for RouteEstimate.
    ttl: u8,
}

impl Cascade {
    /// `hops`: estimated router hops to the remote, if known from TTLs.
    pub fn new(hops: Option<u8>) -> Self {
        // The last router before the remote is one hop short of it.
        let ttl = hops.map_or(MAX_TTL, |h| h.saturating_sub(1).clamp(1, MAX_TTL));
        Self {
            totals: Totals::default(),
            method: Method::Icmp,
            epoch: 0,
            stats: ProbeStats::new(),
            losses_in_a_row: 0,
            ever_replied: false,
            last_sent_us: None,
            ttl,
        }
    }

    /// Never probes: for replays (placeholder addresses can't be pinged) and
    /// targets we can't probe yet (IPv6).
    pub fn passive_only() -> Self {
        let mut c = Self::new(None);
        c.switch(Method::Passive);
        c
    }

    pub fn method(&self) -> Method {
        self.method
    }

    /// The probe to send now, or `None` if it's too soon or we're passive.
    pub fn next_probe(&mut self, now_us: u64) -> Option<Probe> {
        if self.method == Method::Passive {
            return None;
        }
        if self
            .last_sent_us
            .is_some_and(|last| now_us.saturating_sub(last) < PROBE_INTERVAL_US)
        {
            return None;
        }
        if self.method == Method::RouteSearch && self.ttl == 0 {
            // Every TTL has been tried. Wait for the last probe's result (it
            // may still find a router) before giving up.
            if self
                .last_sent_us
                .is_some_and(|last| now_us.saturating_sub(last) >= RESULT_TIMEOUT_US)
            {
                self.switch(Method::Passive);
            }
            return None;
        }
        let ttl = match self.method {
            Method::Icmp => None,
            Method::RouteSearch => {
                let ttl = self.ttl;
                // Walk down for the next search probe; 0 means exhausted.
                self.ttl = self.ttl.saturating_sub(1);
                Some(ttl)
            }
            Method::RouteEstimate => Some(self.ttl),
            Method::Passive => unreachable!("handled above"),
        };
        self.last_sent_us = Some(now_us);
        Some(Probe {
            epoch: self.epoch,
            sent_us: now_us,
            ttl,
        })
    }

    pub fn on_result(&mut self, probe: Probe, result: ProbeResult) {
        if probe.epoch != self.epoch {
            return; // from a method we've moved on from
        }
        match (self.method, result) {
            (Method::Icmp, ProbeResult::EchoReply { rtt_us }) => self.reply(probe, rtt_us),
            (Method::Icmp, _) => {
                self.lost(probe);
                if !self.ever_replied && self.losses_in_a_row >= GIVE_UP_AFTER_LOSSES {
                    self.switch(Method::RouteSearch);
                }
            }
            (Method::RouteSearch, ProbeResult::TtlExpired { rtt_us }) => {
                // This TTL reached a router that answers: use it from now on.
                self.ttl = probe.ttl.unwrap_or(1);
                self.switch(Method::RouteEstimate);
                self.reply(probe, rtt_us);
            }
            (Method::RouteSearch, ProbeResult::EchoReply { rtt_us }) => {
                // The destination answered after all (e.g. a big enough TTL).
                self.switch(Method::Icmp);
                self.reply(probe, rtt_us);
            }
            (Method::RouteSearch, ProbeResult::Lost) => {
                // The last search probe (TTL 1) came back empty: nothing left.
                if self.ttl == 0 && probe.ttl == Some(1) {
                    self.switch(Method::Passive);
                }
            }
            (Method::RouteEstimate, ProbeResult::TtlExpired { rtt_us }) => {
                self.reply(probe, rtt_us);
            }
            (Method::RouteEstimate, ProbeResult::EchoReply { rtt_us }) => {
                // The route got shorter and our TTL now reaches the remote,
                // which evidently answers pings: measure it directly.
                self.switch(Method::Icmp);
                self.reply(probe, rtt_us);
            }
            (Method::RouteEstimate, _) => {
                self.lost(probe);
                if self.losses_in_a_row >= GIVE_UP_AFTER_LOSSES {
                    self.switch(Method::Passive);
                }
            }
            (Method::Passive, _) => {}
        }
    }

    /// Probe numbers for the current method (`None` while searching or passive).
    pub fn view(&mut self, now_us: u64) -> Option<ProbeView> {
        matches!(self.method, Method::Icmp | Method::RouteEstimate)
            .then(|| self.stats.view(now_us))
            .filter(|v| v.probes > 0)
    }

    pub fn totals(&self) -> Totals {
        self.totals
    }

    fn reply(&mut self, probe: Probe, rtt_us: u32) {
        self.totals.replies += 1;
        self.totals.last_rtt_us = Some(rtt_us);
        self.ever_replied = true;
        self.losses_in_a_row = 0;
        self.stats
            .record(probe.sent_us, ProbeOutcome::Reply { rtt_us });
    }

    fn lost(&mut self, probe: Probe) {
        self.totals.lost += 1;
        self.losses_in_a_row += 1;
        self.stats.record(probe.sent_us, ProbeOutcome::Lost);
    }

    fn switch(&mut self, method: Method) {
        self.method = method;
        self.epoch += 1;
        self.losses_in_a_row = 0;
        self.ever_replied = false;
        // Numbers from one method mean something different from another's.
        self.stats = ProbeStats::new();
        // `last_sent_us` is kept: the 5/s cap spans method changes too.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u32 = 1000;

    /// Drives the cascade like the app does: ask every 50 ms, answer each
    /// probe with `respond`. Returns how many probes were sent.
    fn drive(
        c: &mut Cascade,
        from_us: u64,
        to_us: u64,
        mut respond: impl FnMut(&Probe) -> ProbeResult,
    ) -> u32 {
        let mut sent = 0;
        let mut now = from_us;
        while now < to_us {
            if let Some(p) = c.next_probe(now) {
                sent += 1;
                let r = respond(&p);
                c.on_result(p, r);
            }
            now += 50_000;
        }
        sent
    }

    #[test]
    fn never_more_than_five_probes_a_second() {
        let mut c = Cascade::new(Some(10));
        let mut sent = 0;
        for t in (0..1_000_000).step_by(1_000) {
            if let Some(p) = c.next_probe(t) {
                sent += 1;
                c.on_result(p, ProbeResult::EchoReply { rtt_us: 30 * MS });
            }
        }
        assert_eq!(sent, 5);
    }

    #[test]
    fn icmp_that_answers_stays_icmp_even_with_loss() {
        let mut c = Cascade::new(Some(10));
        let mut i = 0;
        drive(&mut c, 0, 10_000_000, |_| {
            i += 1;
            // Every other probe lost after the first reply: that's loss, not "blocked".
            if i % 2 == 0 {
                ProbeResult::Lost
            } else {
                ProbeResult::EchoReply { rtt_us: 30 * MS }
            }
        });
        assert_eq!(c.method(), Method::Icmp);
        let v = c.view(10_000_000).unwrap();
        assert_eq!(v.median_ms, Some(30.0));
        assert!((45.0..=55.0).contains(&v.loss_pct), "{}", v.loss_pct);
    }

    #[test]
    fn blocked_icmp_finds_the_last_router_using_the_hop_count() {
        // Remote is 12 hops away and ignores pings; routers 9–11 answer.
        let mut c = Cascade::new(Some(12));
        let mut tried = Vec::new();
        drive(&mut c, 0, 5_000_000, |p| {
            tried.push(p.ttl);
            match p.ttl {
                None => ProbeResult::Lost,
                Some(t) if (9..=11).contains(&t) => ProbeResult::TtlExpired { rtt_us: 25 * MS },
                Some(_) => ProbeResult::Lost,
            }
        });
        assert_eq!(c.method(), Method::RouteEstimate);
        // 10 failed pings, then the very first search probe (TTL 11) hits.
        assert_eq!(&tried[..10], &[None; 10]);
        assert_eq!(tried[10], Some(11));
        assert!(tried[11..].iter().all(|t| *t == Some(11)));
        assert_eq!(c.view(5_000_000).unwrap().median_ms, Some(25.0));
    }

    #[test]
    fn search_walks_down_past_silent_routers() {
        let mut c = Cascade::new(Some(12));
        let mut tried = Vec::new();
        drive(&mut c, 0, 6_000_000, |p| {
            tried.push(p.ttl);
            match p.ttl {
                Some(8) => ProbeResult::TtlExpired { rtt_us: 20 * MS },
                _ => ProbeResult::Lost,
            }
        });
        assert_eq!(c.method(), Method::RouteEstimate);
        assert_eq!(&tried[10..14], &[Some(11), Some(10), Some(9), Some(8)]);
    }

    #[test]
    fn nothing_answers_ends_passive_and_stops_probing() {
        let mut c = Cascade::new(Some(4));
        let sent = drive(&mut c, 0, 20_000_000, |_| ProbeResult::Lost);
        assert_eq!(c.method(), Method::Passive);
        // 10 pings + TTL 3, 2, 1, then silence.
        assert_eq!(sent, 13);
        assert_eq!(c.view(20_000_000), None);
        assert_eq!(c.next_probe(30_000_000), None);
    }

    #[test]
    fn unknown_hop_count_searches_from_the_top() {
        let mut c = Cascade::new(None);
        let mut first_search = None;
        drive(&mut c, 0, 3_000_000, |p| {
            if p.ttl.is_some() && first_search.is_none() {
                first_search = p.ttl;
            }
            ProbeResult::Lost
        });
        assert_eq!(first_search, Some(MAX_TTL));
    }

    #[test]
    fn route_estimate_that_goes_quiet_falls_back_to_passive() {
        let mut c = Cascade::new(Some(5));
        let mut answering = true;
        let mut n = 0;
        drive(&mut c, 0, 20_000_000, |p| {
            n += 1;
            if n > 30 {
                answering = false;
            }
            match (p.ttl, answering) {
                (Some(_), true) => ProbeResult::TtlExpired { rtt_us: 15 * MS },
                _ => ProbeResult::Lost,
            }
        });
        assert_eq!(c.method(), Method::Passive);
    }

    #[test]
    fn late_results_from_an_old_method_are_ignored() {
        let mut c = Cascade::new(Some(6));
        let stale = c.next_probe(0).unwrap();
        // Ten losses move us to the route search...
        let mut t = 0;
        for _ in 0..10 {
            t += PROBE_INTERVAL_US;
            let p = c.next_probe(t).unwrap();
            c.on_result(p, ProbeResult::Lost);
        }
        assert_eq!(c.method(), Method::RouteSearch);
        // ...so a late echo reply from the very first ping changes nothing.
        c.on_result(stale, ProbeResult::EchoReply { rtt_us: 30 * MS });
        assert_eq!(c.method(), Method::RouteSearch);
    }

    /// Every 1 s window must hold at most 5 probes, through a whole
    /// blocked-ICMP -> route search -> route estimate -> passive run.
    #[test]
    fn cap_holds_across_method_changes() {
        let mut c = Cascade::new(Some(6));
        let mut sent_at = Vec::new();
        let mut n = 0;
        let mut now = 0;
        while now < 30_000_000 {
            if let Some(p) = c.next_probe(now) {
                sent_at.push(now);
                n += 1;
                let r = match p.ttl {
                    Some(4) if n < 40 => ProbeResult::TtlExpired { rtt_us: 20_000 },
                    _ => ProbeResult::Lost,
                };
                c.on_result(p, r);
            }
            now += 10_000;
        }
        assert_eq!(c.method(), Method::Passive, "should end passive");
        for (i, t) in sent_at.iter().enumerate() {
            let in_window = sent_at[i..]
                .iter()
                .take_while(|u| **u < t + 1_000_000)
                .count();
            assert!(in_window <= 5, "{in_window} probes in the second after {t}");
        }
    }

    /// Results arriving late (as the real 1 s prober does) must not be lost
    /// when the search runs out of TTLs.
    #[test]
    fn exhausted_search_waits_for_a_late_reply() {
        let mut c = Cascade::new(Some(3)); // search tries TTL 2, then 1
        let mut now = 0;
        for _ in 0..10 {
            let p = c.next_probe(now).unwrap();
            c.on_result(p, ProbeResult::Lost);
            now += PROBE_INTERVAL_US;
        }
        assert_eq!(c.method(), Method::RouteSearch);
        let ttl2 = c.next_probe(now).unwrap();
        now += PROBE_INTERVAL_US;
        let ttl1 = c.next_probe(now).unwrap();
        assert_eq!((ttl2.ttl, ttl1.ttl), (Some(2), Some(1)));
        // Out of TTLs, but results haven't arrived: keep waiting, send nothing.
        now += PROBE_INTERVAL_US;
        assert_eq!(c.next_probe(now), None);
        assert_eq!(c.method(), Method::RouteSearch);
        // The TTL 2 probe's router answers late: route estimate, not passive.
        c.on_result(ttl2, ProbeResult::TtlExpired { rtt_us: 15_000 });
        c.on_result(ttl1, ProbeResult::Lost);
        assert_eq!(c.method(), Method::RouteEstimate);
    }

    #[test]
    fn exhausted_search_gives_up_after_the_result_timeout() {
        let mut c = Cascade::new(Some(2)); // search tries TTL 1 only
        let mut now = 0;
        for _ in 0..10 {
            let p = c.next_probe(now).unwrap();
            c.on_result(p, ProbeResult::Lost);
            now += PROBE_INTERVAL_US;
        }
        let last = c.next_probe(now).unwrap();
        assert_eq!(last.ttl, Some(1));
        // Its result never arrives.
        assert_eq!(c.next_probe(now + RESULT_TIMEOUT_US - 1), None);
        assert_eq!(c.method(), Method::RouteSearch);
        assert_eq!(c.next_probe(now + RESULT_TIMEOUT_US), None);
        assert_eq!(c.method(), Method::Passive);
    }

    #[test]
    fn route_estimate_switches_back_when_the_remote_answers() {
        let mut c = Cascade::new(Some(5));
        let mut now = 0;
        for _ in 0..10 {
            let p = c.next_probe(now).unwrap();
            c.on_result(p, ProbeResult::Lost);
            now += PROBE_INTERVAL_US;
        }
        let p = c.next_probe(now).unwrap();
        c.on_result(p, ProbeResult::TtlExpired { rtt_us: 10_000 });
        assert_eq!(c.method(), Method::RouteEstimate);
        now += PROBE_INTERVAL_US;
        let p = c.next_probe(now).unwrap();
        c.on_result(p, ProbeResult::EchoReply { rtt_us: 12_000 });
        assert_eq!(c.method(), Method::Icmp);
        assert_eq!(c.view(now).unwrap().median_ms, Some(12.0));
    }
}
