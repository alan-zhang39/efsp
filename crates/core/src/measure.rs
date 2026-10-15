//! Turns probe results and captured packets into connection numbers.
//! Pure: callers pass every timestamp in.
//!
//! - [`ProbeStats`]: ping RTT (last / median / p95 over a rolling 10 s
//!   window), RFC 3550 jitter and loss, from our own probes.
//! - [`PassiveStats`]: packet rate, inter-arrival jitter and stalls, from the
//!   match flow's inbound packets when no probe gets an answer.

use std::collections::VecDeque;

use serde::Serialize;

/// Window for RTT median/p95, loss and packet rate.
pub const WINDOW_US: u64 = 10_000_000;
/// An inbound gap longer than this counts as a stall.
pub const STALL_GAP_US: u64 = 150_000;

/// What happened to one probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    Reply { rtt_us: u32 },
    Lost,
}

/// Rounds microseconds to milliseconds with one decimal, for display.
fn ms(us: f64) -> f64 {
    (us / 100.0).round() / 10.0
}

/// RFC 3550 §6.4.1 running jitter: J += (|D| - J) / 16, where D is the change
/// between consecutive values. Smooths noise; reacts within ~16 samples.
/// Values are fed in the order results arrive, which at ≤ 5 probes/s with
/// sub-second RTTs is almost always send order.
#[derive(Debug, Clone, Copy, Default)]
struct Rfc3550Jitter {
    previous: Option<f64>,
    jitter: f64,
}

impl Rfc3550Jitter {
    fn add(&mut self, value: f64) {
        if let Some(prev) = self.previous {
            let d = (value - prev).abs();
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.previous = Some(value);
    }
}

/// Numbers from our own probes. Shown with the method that produced them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeView {
    pub last_ms: Option<f64>,
    pub median_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub jitter_ms: f64,
    /// Lost probes as a percentage of probes in the window (0–100).
    pub loss_pct: f64,
    /// Probes with a known outcome in the window.
    pub probes: u32,
}

#[derive(Debug, Default)]
pub struct ProbeStats {
    /// (sent time, outcome), oldest first, within the window.
    window: VecDeque<(u64, ProbeOutcome)>,
    jitter: Rfc3550Jitter,
    last_rtt_us: Option<u32>,
}

impl ProbeStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a probe sent at `sent_us`. Results may arrive out of order;
    /// the window is ordered by send time.
    pub fn record(&mut self, sent_us: u64, outcome: ProbeOutcome) {
        let at = self.window.partition_point(|(t, _)| *t <= sent_us);
        self.window.insert(at, (sent_us, outcome));
        if let ProbeOutcome::Reply { rtt_us } = outcome {
            self.jitter.add(f64::from(rtt_us));
            self.last_rtt_us = Some(rtt_us);
        }
    }

    /// Replies seen so far in the window (used by the method cascade).
    pub fn replies(&self) -> usize {
        self.window
            .iter()
            .filter(|(_, o)| matches!(o, ProbeOutcome::Reply { .. }))
            .count()
    }

    pub fn view(&mut self, now_us: u64) -> ProbeView {
        let start = now_us.saturating_sub(WINDOW_US);
        while self.window.front().is_some_and(|(t, _)| *t < start) {
            self.window.pop_front();
        }
        let mut rtts: Vec<u32> = self
            .window
            .iter()
            .filter_map(|(_, o)| match o {
                ProbeOutcome::Reply { rtt_us } => Some(*rtt_us),
                ProbeOutcome::Lost => None,
            })
            .collect();
        rtts.sort_unstable();
        let probes = self.window.len() as u32;
        let lost = probes - rtts.len() as u32;
        ProbeView {
            last_ms: self.last_rtt_us.map(|r| ms(f64::from(r))),
            median_ms: median(&rtts).map(ms),
            p95_ms: nearest_rank(&rtts, 95).map(|r| ms(f64::from(r))),
            jitter_ms: ms(self.jitter.jitter),
            loss_pct: if probes == 0 {
                0.0
            } else {
                (f64::from(lost) * 1000.0 / f64::from(probes)).round() / 10.0
            },
            probes,
        }
    }
}

/// Numbers for a whole match from every reply RTT (in arrival order) and the
/// number of lost probes. The live card uses a rolling 10 s window instead;
/// saved history uses this, so a match is judged on all of it.
pub fn whole_match(rtts_us: &[u32], lost: u64) -> Option<ProbeView> {
    if rtts_us.is_empty() {
        return None;
    }
    let mut jitter = Rfc3550Jitter::default();
    for r in rtts_us {
        jitter.add(f64::from(*r));
    }
    let mut sorted = rtts_us.to_vec();
    sorted.sort_unstable();
    let probes = rtts_us.len() as u64 + lost;
    Some(ProbeView {
        last_ms: rtts_us.last().map(|r| ms(f64::from(*r))),
        median_ms: median(&sorted).map(ms),
        p95_ms: nearest_rank(&sorted, 95).map(|r| ms(f64::from(r))),
        jitter_ms: ms(jitter.jitter),
        loss_pct: (lost as f64 * 1000.0 / probes as f64).round() / 10.0,
        probes: u32::try_from(probes).unwrap_or(u32::MAX),
    })
}

/// Median of sorted values; the mean of the two middle ones for even counts.
fn median(sorted: &[u32]) -> Option<f64> {
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(f64::from(sorted[n / 2])),
        _ => Some((f64::from(sorted[n / 2 - 1]) + f64::from(sorted[n / 2])) / 2.0),
    }
}

/// Nearest-rank percentile of sorted values: the smallest value with at
/// least `pct`% of samples at or below it.
fn nearest_rank(sorted: &[u32], pct: usize) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (pct * sorted.len()).div_ceil(100).max(1);
    Some(sorted[rank - 1])
}

/// Numbers from captured packets alone: no ping, so no RTT.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassiveView {
    /// Inbound packets per second over the window.
    pub rate_pps: f64,
    /// RFC 3550-style jitter on the gaps between inbound packets.
    pub jitter_ms: f64,
    /// Completed gaps longer than [`STALL_GAP_US`] in the window.
    pub stalls: u32,
    /// Longest gap in the window, including one still open right now.
    pub longest_gap_ms: f64,
}

/// Inbound timing for one flow. Feed it inbound packet times only.
#[derive(Debug, Default)]
pub struct PassiveStats {
    first_us: Option<u64>,
    last_us: Option<u64>,
    arrivals: VecDeque<u64>,
    /// (when the gap ended, gap length), for gaps over the stall threshold
    /// and for the longest-gap figure.
    gaps: VecDeque<(u64, u64)>,
    jitter: Rfc3550Jitter,
    /// Running counts for the whole flow (never go down).
    total_arrivals: u64,
    total_stalls: u64,
}

impl PassiveStats {
    /// (inbound packets, stalls) since the flow started.
    pub fn totals(&self) -> (u64, u64) {
        (self.total_arrivals, self.total_stalls)
    }

    /// When the latest inbound packet arrived, if any.
    pub fn last_arrival_us(&self) -> Option<u64> {
        self.last_us
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn inbound(&mut self, t_us: u64) {
        self.first_us.get_or_insert(t_us);
        if let Some(last) = self.last_us {
            if t_us < last {
                return; // out of order: ignore rather than invent a negative gap
            }
            let gap = t_us - last;
            self.jitter.add(gap as f64);
            self.gaps.push_back((t_us, gap));
            if gap > STALL_GAP_US {
                self.total_stalls += 1;
            }
        }
        self.last_us = Some(t_us);
        self.arrivals.push_back(t_us);
        self.total_arrivals += 1;
    }

    pub fn view(&mut self, now_us: u64) -> PassiveView {
        let start = now_us.saturating_sub(WINDOW_US);
        while self.arrivals.front().is_some_and(|t| *t < start) {
            self.arrivals.pop_front();
        }
        while self.gaps.front().is_some_and(|(t, _)| *t < start) {
            self.gaps.pop_front();
        }
        // Rate over the window, or over the flow's life if it's younger, but
        // never over less than a second: a few ms of data would read as
        // hundreds of packets per second.
        let span_us = self
            .first_us
            .map_or(0, |f| now_us.saturating_sub(f.max(start)))
            .max(1_000_000);
        let open_gap = self.last_us.map_or(0, |l| now_us.saturating_sub(l));
        let longest = self
            .gaps
            .iter()
            .map(|(_, g)| *g)
            .max()
            .unwrap_or(0)
            .max(open_gap);
        PassiveView {
            rate_pps: (self.arrivals.len() as f64 * 1e6 / span_us as f64 * 10.0).round() / 10.0,
            jitter_ms: ms(self.jitter.jitter),
            stalls: self.gaps.iter().filter(|(_, g)| *g > STALL_GAP_US).count() as u32,
            longest_gap_ms: ms(longest as f64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u32 = 1000;
    const SEC: u64 = 1_000_000;

    fn reply(ms: u32) -> ProbeOutcome {
        ProbeOutcome::Reply { rtt_us: ms * MS }
    }

    fn feed(stats: &mut ProbeStats, outcomes: &[ProbeOutcome]) {
        for (i, o) in outcomes.iter().enumerate() {
            stats.record(i as u64 * SEC / 5, *o); // 5 probes/s
        }
    }

    #[test]
    fn hand_computed_example() {
        // RTTs 30, 32, 31, 35, 30 ms. Sorted: 30 30 31 32 35.
        // Median 31. p95 nearest rank: ceil(0.95 × 5) = 5th → 35.
        // Jitter, D = 2, 1, 4, 5:
        //   0.125 → 0.1797 → 0.4185 → 0.7048 ms → 0.7
        let mut s = ProbeStats::new();
        feed(
            &mut s,
            &[reply(30), reply(32), reply(31), reply(35), reply(30)],
        );
        let v = s.view(SEC);
        assert_eq!(v.last_ms, Some(30.0));
        assert_eq!(v.median_ms, Some(31.0));
        assert_eq!(v.p95_ms, Some(35.0));
        assert_eq!(v.jitter_ms, 0.7);
        assert_eq!(v.loss_pct, 0.0);
        assert_eq!(v.probes, 5);
    }

    #[test]
    fn even_count_median_and_loss() {
        // Replies 20, 40, 30, 50 plus 1 lost: median (30+40)/2 = 35,
        // loss 1/5 = 20 %. Lost probes don't touch jitter.
        let mut s = ProbeStats::new();
        feed(
            &mut s,
            &[
                reply(20),
                ProbeOutcome::Lost,
                reply(40),
                reply(30),
                reply(50),
            ],
        );
        let v = s.view(SEC);
        assert_eq!(v.median_ms, Some(35.0));
        assert_eq!(v.loss_pct, 20.0);
        assert_eq!(s.replies(), 4);
    }

    #[test]
    fn p95_of_twenty_is_the_nineteenth() {
        // 1..=20 ms: ceil(0.95 × 20) = 19th value = 19 ms.
        let mut s = ProbeStats::new();
        let outcomes: Vec<_> = (1..=20).map(reply).collect();
        feed(&mut s, &outcomes);
        assert_eq!(s.view(4 * SEC).p95_ms, Some(19.0));
    }

    #[test]
    fn steady_rtt_has_zero_jitter_and_converges_after_a_step() {
        let mut s = ProbeStats::new();
        feed(&mut s, &[reply(40); 10]);
        assert_eq!(s.view(2 * SEC).jitter_ms, 0.0);

        // Alternating 40/50 ms: |D| = 10 every time, so J → 10 ms.
        let mut alt = ProbeStats::new();
        let outcomes: Vec<_> = (0..200)
            .map(|i| reply(if i % 2 == 0 { 40 } else { 50 }))
            .collect();
        feed(&mut alt, &outcomes);
        assert!((alt.view(40 * SEC).jitter_ms - 10.0).abs() < 0.1);
    }

    #[test]
    fn old_probes_leave_the_ten_second_window() {
        let mut s = ProbeStats::new();
        s.record(0, reply(100));
        s.record(0, ProbeOutcome::Lost);
        s.record(11 * SEC, reply(20));
        let v = s.view(11 * SEC);
        assert_eq!(v.median_ms, Some(20.0));
        assert_eq!(v.loss_pct, 0.0);
        assert_eq!(v.probes, 1);
    }

    #[test]
    fn out_of_order_results_are_placed_by_send_time() {
        let mut s = ProbeStats::new();
        s.record(2 * SEC, reply(30));
        s.record(SEC, ProbeOutcome::Lost); // timed out, reported late
        s.record(3 * SEC, reply(30));
        // At 11.5 s the 1 s probe has left the window; only the two replies stay.
        let v = s.view(11 * SEC + SEC / 2);
        assert_eq!((v.probes, v.loss_pct), (2, 0.0));
    }

    #[test]
    fn nothing_yet_is_empty_not_zero() {
        let v = ProbeStats::new().view(0);
        assert_eq!((v.last_ms, v.median_ms, v.p95_ms), (None, None, None));
        assert_eq!((v.loss_pct, v.probes), (0.0, 0));
    }

    #[test]
    fn display_rounds_to_a_tenth_of_a_millisecond() {
        let mut s = ProbeStats::new();
        s.record(0, ProbeOutcome::Reply { rtt_us: 31_449 });
        assert_eq!(s.view(0).last_ms, Some(31.4));
    }

    #[test]
    fn whole_match_uses_every_reply() {
        assert_eq!(whole_match(&[], 3), None);
        // Same hand-worked numbers as above, plus 1 lost of 6 probes.
        let v = whole_match(&[30 * MS, 32 * MS, 31 * MS, 35 * MS, 30 * MS], 1).unwrap();
        assert_eq!(v.median_ms, Some(31.0));
        assert_eq!(v.p95_ms, Some(35.0));
        assert_eq!(v.jitter_ms, 0.7);
        assert_eq!(v.loss_pct, 16.7);
        assert_eq!(v.probes, 6);
    }

    #[test]
    fn passive_steady_stream() {
        // 50 pps for 4 s, perfectly even: 50 pps, no jitter, no stalls.
        let mut p = PassiveStats::new();
        for i in 0..200 {
            p.inbound(i * 20_000);
        }
        let v = p.view(4 * SEC);
        assert_eq!(v.rate_pps, 50.0);
        assert_eq!(v.jitter_ms, 0.0);
        assert_eq!(v.stalls, 0);
        assert_eq!(v.longest_gap_ms, 20.0);
    }

    #[test]
    fn passive_counts_a_stall_and_an_open_gap() {
        let mut p = PassiveStats::new();
        for t in [0, 20_000, 40_000, 240_000, 260_000] {
            p.inbound(t); // one 200 ms gap
        }
        let v = p.view(260_000);
        assert_eq!((v.stalls, v.longest_gap_ms), (1, 200.0));

        // Nothing for 500 ms more: longest gap reflects the open one, but it
        // isn't a completed stall yet.
        let v = p.view(760_000);
        assert_eq!((v.stalls, v.longest_gap_ms), (1, 500.0));
    }

    #[test]
    fn passive_rate_does_not_spike_on_a_new_flow() {
        let mut p = PassiveStats::new();
        p.inbound(0);
        p.inbound(5_000);
        assert_eq!(p.view(5_000).rate_pps, 2.0, "2 packets over a 1 s floor");
    }

    #[test]
    fn passive_ignores_out_of_order_arrivals() {
        let mut p = PassiveStats::new();
        p.inbound(100_000);
        p.inbound(50_000);
        p.inbound(120_000);
        assert_eq!(p.view(120_000).longest_gap_ms, 20.0);
    }
}
