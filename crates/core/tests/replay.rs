//! Replay-driven integration tests: traces in, flows out.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use efsp_core::capture::{Direction, Next, PacketSource, Protocol};
use efsp_core::flow::{FlowTracker, FlowView};
use efsp_core::measure::{PassiveStats, PassiveView};
use efsp_core::trace::ReplaySource;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/traces")
}

fn open(name: &str) -> ReplaySource<BufReader<File>> {
    ReplaySource::new(BufReader::new(File::open(fixtures().join(name)).unwrap()))
}

/// Replays a trace, taking a snapshot at each requested time (µs).
fn snapshots_at(name: &str, times: &[u64]) -> Vec<Vec<FlowView>> {
    let mut src = open(name);
    let mut tracker = FlowTracker::new();
    let mut out = Vec::new();
    let mut pending = times.iter().copied().peekable();
    loop {
        match src.next().unwrap() {
            Next::Packet(p) => {
                while let Some(t) = pending.next_if(|t| *t < p.timestamp_us) {
                    out.push(tracker.snapshot(t));
                }
                tracker.observe(&p);
            }
            Next::Idle => {}
            Next::End => break,
        }
    }
    out.extend(pending.map(|t| tracker.snapshot(t)));
    out
}

const SEC: u64 = 1_000_000;

#[test]
fn match_flow_appears_then_becomes_sustained_while_lobby_does_not() {
    let snaps = snapshots_at(
        "synthetic-lobby-then-match.jsonl",
        &[5 * SEC, 10 * SEC + 300_000, 12 * SEC],
    );

    // Before the match: only the lobby connection.
    assert_eq!(snaps[0].len(), 1);
    assert_eq!(snaps[0][0].protocol, Protocol::Tcp);
    assert!(!snaps[0][0].sustained);

    // Just after the match starts: new UDP flow, listed first, not yet sustained.
    let new_flow = &snaps[1][0];
    assert_eq!(new_flow.protocol, Protocol::Udp);
    assert!(!new_flow.sustained);

    // Two seconds in: sustained at ~60 pps; the lobby stays unsustained.
    let m = snaps[2]
        .iter()
        .find(|f| f.protocol == Protocol::Udp)
        .unwrap();
    let lobby = snaps[2]
        .iter()
        .find(|f| f.protocol == Protocol::Tcp)
        .unwrap();
    assert!(m.sustained, "{m:?}");
    assert!((55..=65).contains(&m.rate_pps), "{}", m.rate_pps);
    assert!(!lobby.sustained);
    assert_ne!(m.remote_label, lobby.remote_label);
}

/// Guard for "never commit real IP addresses" (docs/DEVELOPING.md): every committed
/// trace must parse, and the reader rejects any non-documentation address.
#[test]
fn every_committed_trace_is_anonymised_and_valid() {
    let mut count = 0;
    for entry in std::fs::read_dir(fixtures()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            let mut src = ReplaySource::new(BufReader::new(File::open(&path).unwrap()));
            loop {
                match src.next() {
                    Ok(Next::End) => break,
                    Ok(_) => {}
                    Err(e) => panic!("{}: {e}", path.display()),
                }
            }
            count += 1;
        }
    }
    assert!(count > 0, "no fixtures found");
}

/// Real trace from a Windows PC (anonymised): ~99 s of lobby
/// noise plus two match offers ("4 bars"), neither accepted. Both offers go
/// to the same server on remote port 5735 at ~106 packets/s.
const REAL: &str = "real-two-match-offers.jsonl";

/// Every flow that was ever sustained, with how long after its first packet
/// it first was (snapshots every 250 ms, like the app).
fn sustained_flows(name: &str) -> Vec<(FlowView, u64)> {
    let end = 100 * SEC;
    let times: Vec<u64> = (0..end / 250_000).map(|i| i * 250_000).collect();
    let mut first_sustained: Vec<(FlowView, u64)> = Vec::new();
    for snap in snapshots_at(name, &times) {
        for f in snap.into_iter().filter(|f| f.sustained) {
            if !first_sustained.iter().any(|(g, _)| g.id == f.id) {
                let after_ms = f.age_ms;
                first_sustained.push((f, after_ms));
            }
        }
    }
    first_sustained
}

#[test]
fn real_trace_flags_exactly_the_two_match_offers_within_two_seconds() {
    let steady = sustained_flows(REAL);
    assert_eq!(
        steady.len(),
        2,
        "only the match flows may be sustained: {steady:?}"
    );
    for (flow, after_ms) in &steady {
        assert_eq!(flow.protocol, Protocol::Udp);
        assert_eq!(flow.remote_port, 5735);
        // Target: ping and bars within ~2 s of the match being found.
        assert!(*after_ms <= 2_250, "steady only after {after_ms} ms");
    }
    // Both offers are separate connections, from different local ports.
    assert_ne!(steady[0].0.local_port, steady[1].0.local_port);
}

#[test]
fn real_trace_noise_stays_quiet() {
    // At 60 s, between the two offers, there are many lobby/probe flows and
    // none of them is anywhere near the sustained threshold.
    let snap = snapshots_at(REAL, &[60 * SEC]).remove(0);
    assert!(
        snap.len() > 20,
        "expected lots of background flows, got {}",
        snap.len()
    );
    assert!(snap.iter().all(|f| !f.sustained));
    assert!(
        snap.iter()
            .filter(|f| f.protocol == Protocol::Udp)
            .all(|f| f.rate_pps < 10)
    );
}

/// Feeds one real match flow's inbound packets into `PassiveStats`.
fn passive_for_match(local_port: u16) -> (PassiveView, u64) {
    let mut src = open(REAL);
    let mut stats = PassiveStats::new();
    let mut last = 0;
    loop {
        match src.next().unwrap() {
            Next::Packet(p)
                if p.local_port == local_port
                    && p.remote.get().port() == 5735
                    && p.direction == Direction::Inbound =>
            {
                stats.inbound(p.timestamp_us);
                last = p.timestamp_us;
            }
            Next::End => break,
            _ => {}
        }
    }
    (stats.view(last), last)
}

#[test]
fn real_match_flows_passive_numbers() {
    // Measured on the real trace: ~52 pps inbound, ~4-4.5 ms gap jitter,
    // longest gap ~116 ms (under the 150 ms stall threshold), no stalls.
    for lp in [51751, 61584] {
        let (v, _) = passive_for_match(lp);
        assert!((50.0..=55.0).contains(&v.rate_pps), "{lp}: {v:?}");
        assert!((2.0..=7.0).contains(&v.jitter_ms), "{lp}: {v:?}");
        assert_eq!(v.stalls, 0, "{lp}: {v:?}");
        assert!(v.longest_gap_ms < 150.0, "{lp}: {v:?}");
    }
}

#[test]
fn real_trace_probing_starts_no_later_than_steady() {
    // Walk the trace in 100 ms steps; note when each match flow first becomes
    // the candidate and when it is first sustained (both relative to its
    // first packet).
    let mut src = open(REAL);
    let mut tracker = FlowTracker::new();
    let mut packets = Vec::new();
    while let Next::Packet(p) = src.next().unwrap() {
        packets.push(p);
    }
    let mut first_candidate: Vec<(u32, u64)> = Vec::new();
    let mut first_sustained: Vec<(u32, u64)> = Vec::new();
    let mut i = 0;
    for step in 0..1000u64 {
        let now = step * 100_000;
        while i < packets.len() && packets[i].timestamp_us <= now {
            tracker.observe(&packets[i]);
            i += 1;
        }
        if let Some(c) = tracker.match_candidate(now)
            && !first_candidate.iter().any(|(id, _)| *id == c.flow_id)
        {
            first_candidate.push((c.flow_id, now));
        }
        for f in tracker.snapshot(now).into_iter().filter(|f| f.sustained) {
            if !first_sustained.iter().any(|(id, _)| *id == f.id) {
                first_sustained.push((f.id, now));
            }
        }
    }
    assert_eq!(
        first_candidate.len(),
        2,
        "only the two match flows: {first_candidate:?}"
    );
    for ((cid, ct), (sid, st)) in first_candidate.iter().zip(&first_sustained) {
        assert_eq!(cid, sid);
        assert!(
            ct <= st,
            "candidate at {ct} should not be later than steady at {st}"
        );
    }
}

#[cfg(feature = "dev-tools")]
#[test]
fn real_trace_game_pings_pair_up() {
    use efsp_core::gameping::GamePings;
    let mut src = open(REAL);
    let mut pings = GamePings::new();
    while let Next::Packet(p) = src.next().unwrap() {
        pings.observe(&p);
    }
    let v = pings.view();
    // ~60-70 servers answered; the nearest ~20 ms, typical well over 100 ms.
    assert!((50..=75).contains(&v.servers), "{v:?}");
    let nearest = v.nearest_ms.unwrap();
    assert!((10.0..=30.0).contains(&nearest), "{v:?}");
    assert!(v.typical_ms.unwrap() > 100.0, "{v:?}");
    // The match server must not show up as a game ping: its flow is busy.
    let match_server: std::net::IpAddr = "203.0.113.71".parse().unwrap();
    assert!(
        pings
            .nearest(1000)
            .iter()
            .all(|(ip, _)| *ip != match_server)
    );
}

/// Second real trace (v2, with TTLs): one ~85 s match to a server
/// whose packets arrive with TTL 56, i.e. ~8 router hops away.
const REAL_V2: &str = "real-match-v2-ttl.jsonl";

#[test]
fn real_v2_trace_gives_a_hop_count_for_the_match() {
    let mut src = open(REAL_V2);
    let mut tracker = FlowTracker::new();
    let mut candidate = None;
    while let Next::Packet(p) = src.next().unwrap() {
        let now = p.timestamp_us;
        tracker.observe(&p);
        if candidate.is_none() {
            candidate = tracker.match_candidate(now);
        }
    }
    let c = candidate.expect("the match should become a candidate");
    assert_eq!(c.remote.get().port(), 5735);
    // TTL 56 from a sender that started at 64: 8 hops. The route search
    // would start at TTL 7, one hop short of the server.
    assert_eq!(c.hops, Some(8));
}
