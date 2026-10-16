//! Dev builds only: realistic sample history for trying the UI without
//! playing 150 matches. Deterministic (a fixed seed), no addresses, and
//! nothing here is compiled into release builds.

#![cfg(feature = "dev-tools")]

use efsp_core::capture::Protocol;
use efsp_core::cascade::Method;
use efsp_core::classify::{Endpoint, place};
use efsp_core::history::{Kind, NewRecord, sparkline};
use efsp_core::session::Point;

/// Tiny deterministic generator (xorshift), so samples look the same each time.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 10_000) as f64 / 10_000.0
    }
}

fn server(region: &str) -> Endpoint {
    Endpoint::Server {
        region: region.to_owned(),
        place: place(region),
    }
}

/// `count` matches and offers over the last 30 days before `now` (Unix s).
pub fn records(now: i64, count: usize) -> Vec<NewRecord> {
    // Where matches land, with a typical ping for each.
    let places: Vec<(Endpoint, f64)> = vec![
        (server("us-east1"), 52.0),
        (server("us-east-1"), 25.0),
        (server("northamerica-northeast2"), 30.0),
        (server("us-east-2"), 38.0),
        (server("us-central1"), 61.0),
        (server("us-west-2"), 88.0),
        (server("europe-west3"), 104.0),
        (
            Endpoint::Peer {
                country: Some("CA".into()),
            },
            26.0,
        ),
        (
            Endpoint::Peer {
                country: Some("US".into()),
            },
            41.0,
        ),
        (
            Endpoint::Peer {
                country: Some("BR".into()),
            },
            142.0,
        ),
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let (endpoint, base) = &places[(rng.next() * places.len() as f64) as usize % places.len()];
        let offer = rng.next() < 0.15;
        let duration_ms: u64 = if offer {
            5_000 + (rng.next() * 6_000.0) as u64
        } else {
            420_000 + (rng.next() * 900_000.0) as u64
        };
        // Spread over 30 days, mostly evenings.
        let day = (i as f64 / count as f64 * 30.0) as i64;
        let hour = 17 + (rng.next() * 7.0) as i64;
        let started_at = now - 30 * 86_400 + day * 86_400 + hour * 3_600 - 12 * 3_600;
        let wobble = 1.0 + (rng.next() - 0.5) * 0.3;
        let median = (base * wobble * 10.0).round() / 10.0;
        let jitter = ((1.0 + rng.next() * 6.0) * 10.0).round() / 10.0;
        let loss = if rng.next() < 0.1 {
            (rng.next() * 4.0 * 10.0).round() / 10.0
        } else {
            0.0
        };
        // A graph point every 250 ms, wandering around the median.
        let points: Vec<Point> = (0..duration_ms / 250)
            .map(|k| {
                let t = k as f64 / 40.0;
                let rtt = median + (t.sin() * jitter) + (rng.next() - 0.5) * jitter;
                Point {
                    t_ms: k * 250,
                    rtt_ms: Some((rtt.max(1.0) * 10.0).round() / 10.0),
                    jitter_ms: Some(jitter),
                    lost: u32::from(loss > 0.0 && rng.next() < 0.01),
                    stalls: 0,
                    rate_pps: 100.0,
                }
            })
            .collect();
        let bars = efsp_core::bars::bars(
            median,
            jitter,
            loss,
            &efsp_core::bars::BarThresholds::default(),
        );
        out.push(NewRecord {
            started_at,
            duration_ms,
            kind: Kind::for_duration(duration_ms),
            method: Method::Icmp,
            median_ms: Some(median),
            p95_ms: Some(median + jitter * 2.0),
            jitter_ms: Some(jitter),
            loss_pct: Some(loss),
            stalls: 0,
            bars: Some(bars),
            rate_pps: 100.0,
            spark_json: serde_json::to_string(&sparkline(&points)).unwrap_or_else(|_| "[]".into()),
            timeline_json: serde_json::to_string(&points).unwrap_or_else(|_| "[]".into()),
            endpoint: Some(endpoint.clone()),
            protocol: Protocol::Udp,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_deterministic_and_varied() {
        let a = records(1_800_000_000, 150);
        assert_eq!(a.len(), 150);
        assert_eq!(a, records(1_800_000_000, 150));
        assert!(a.iter().any(|r| r.kind == Kind::Offer));
        assert!(
            a.iter()
                .any(|r| matches!(r.endpoint, Some(Endpoint::Peer { .. })))
        );
        assert!(a.iter().all(|r| r.started_at < 1_800_000_000));
    }
}
