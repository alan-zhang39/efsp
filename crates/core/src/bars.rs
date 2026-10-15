//! efsp's own bars, 1–5. Replaces a Good / OK / Bad verdict: the
//! card shows the ping, these bars and a colour, never verdict words.

use serde::Serialize;

/// Thresholds, in one place so a settings screen can edit them later.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarThresholds {
    /// Upper median-ping bounds for 5, 4, 3 and 2 bars; above the last is 1.
    pub ping_ms: [f64; 4],
    /// One bar off at or above this jitter...
    pub jitter_penalty_ms: f64,
    /// ...or at or above this loss.
    pub loss_penalty_pct: f64,
}

impl Default for BarThresholds {
    fn default() -> Self {
        Self {
            ping_ms: [40.0, 60.0, 80.0, 120.0],
            jitter_penalty_ms: 15.0,
            loss_penalty_pct: 2.0,
        }
    }
}

/// 1–5 bars from median ping, minus one (never below 1) for high jitter or
/// loss: a 30 ms ping that stutters shouldn't look perfect.
pub fn bars(median_ms: f64, jitter_ms: f64, loss_pct: f64, t: &BarThresholds) -> u8 {
    let from_ping = match t.ping_ms.iter().position(|limit| median_ms <= *limit) {
        Some(i) => 5 - i as u8,
        None => 1,
    };
    let penalty = jitter_ms >= t.jitter_penalty_ms || loss_pct >= t.loss_penalty_pct;
    if penalty {
        from_ping.saturating_sub(1).max(1)
    } else {
        from_ping
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(ping: f64, jitter: f64, loss: f64) -> u8 {
        bars(ping, jitter, loss, &BarThresholds::default())
    }

    #[test]
    fn ping_steps_are_inclusive_upper_bounds() {
        assert_eq!(b(12.0, 0.0, 0.0), 5);
        assert_eq!(b(40.0, 0.0, 0.0), 5);
        assert_eq!(b(40.1, 0.0, 0.0), 4);
        assert_eq!(b(60.0, 0.0, 0.0), 4);
        assert_eq!(b(80.0, 0.0, 0.0), 3);
        assert_eq!(b(120.0, 0.0, 0.0), 2);
        assert_eq!(b(120.1, 0.0, 0.0), 1);
        assert_eq!(b(400.0, 0.0, 0.0), 1);
    }

    #[test]
    fn jitter_or_loss_costs_one_bar() {
        assert_eq!(b(24.0, 4.8, 0.0), 5, "real match: 24 ms, 4.8 ms jitter");
        assert_eq!(b(24.0, 15.0, 0.0), 4);
        assert_eq!(b(24.0, 0.0, 2.0), 4);
        assert_eq!(b(24.0, 20.0, 5.0), 4, "one bar off, not two");
    }

    #[test]
    fn never_below_one() {
        assert_eq!(b(300.0, 50.0, 30.0), 1);
    }
}
