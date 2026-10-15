//! Match history: what gets saved per match, and the pure calculations on it.
//! Storage itself (SQLite) lives in the app crate.
//!
//! Privacy: no address, and nothing derived from one, is
//! saved: only the numbers, and where the match was (server city or the
//! peer's country).

use serde::Serialize;

use crate::capture::Protocol;
use crate::cascade::Method;
use crate::classify::Endpoint;
use crate::session::FinishedMatch;

/// Shorter than this = a match offer that wasn't played (declined or timed
/// out). Real offers last ~8–11 s; a played match is many minutes.
pub const OFFER_BELOW_MS: u64 = 60_000;

/// Points in a history row's sparkline.
pub const SPARK_POINTS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Offer,
    Match,
}

impl Kind {
    pub fn for_duration(duration_ms: u64) -> Self {
        if duration_ms < OFFER_BELOW_MS {
            Kind::Offer
        } else {
            Kind::Match
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Offer => "offer",
            Kind::Match => "match",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "offer" => Some(Kind::Offer),
            "match" => Some(Kind::Match),
            _ => None,
        }
    }
}

/// Method names as stored (stable strings, independent of serde naming).
pub fn method_str(m: Method) -> &'static str {
    match m {
        Method::Icmp => "icmp",
        Method::RouteSearch => "routeSearch",
        Method::RouteEstimate => "routeEstimate",
        Method::Passive => "passive",
    }
}

pub fn parse_method(s: &str) -> Option<Method> {
    match s {
        "icmp" => Some(Method::Icmp),
        "routeSearch" => Some(Method::RouteSearch),
        "routeEstimate" => Some(Method::RouteEstimate),
        "passive" => Some(Method::Passive),
        _ => None,
    }
}

/// One saved match or offer, without its timeline. What the history list
/// shows. Contains no address.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchRecord {
    pub id: i64,
    /// Unix seconds (UTC). The UI shows it in local time.
    pub started_at: i64,
    pub duration_ms: u64,
    pub kind: Kind,
    pub method: Method,
    pub median_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: Option<f64>,
    pub stalls: u32,
    pub bars: Option<u8>,
    pub rate_pps: f64,
    /// How many earlier saved matches were with the same connection.
    /// Ping over the match in [`SPARK_POINTS`] buckets (median RTT per
    /// bucket; `None` where no reply came), for the list's sparkline.
    pub spark: Vec<Option<f64>>,
    /// Server region or peer country; `None` for records saved
    /// before locations were recorded.
    pub endpoint: Option<Endpoint>,
    pub protocol: Protocol,
}

/// A record about to be saved, built from a finished session.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRecord {
    pub started_at: i64,
    pub duration_ms: u64,
    pub kind: Kind,
    pub method: Method,
    pub median_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: Option<f64>,
    pub stalls: u32,
    pub bars: Option<u8>,
    pub rate_pps: f64,
    /// Timeline as JSON, for the detail view's graph.
    pub timeline_json: String,
    /// Sparkline as JSON (see [`MatchRecord::spark`]).
    pub spark_json: String,
    pub endpoint: Option<Endpoint>,
    pub protocol: Protocol,
}

impl NewRecord {
    /// `ended_at`: Unix seconds when the match's traffic stopped.
    pub fn from_finished(m: &FinishedMatch, ended_at: i64) -> Self {
        let s = &m.summary;
        let duration_ms = s.duration_ms;
        let ping = s.ping.as_ref();
        Self {
            started_at: ended_at - (duration_ms / 1000) as i64,
            duration_ms,
            kind: Kind::for_duration(duration_ms),
            method: s.method,
            median_ms: ping.and_then(|p| p.median_ms),
            p95_ms: ping.and_then(|p| p.p95_ms),
            jitter_ms: ping.map(|p| p.jitter_ms),
            loss_pct: ping.map(|p| p.loss_pct),
            stalls: s.passive.stalls,
            bars: s.bars,
            rate_pps: s.passive.rate_pps,
            // Serialising plain numbers can't fail.
            timeline_json: serde_json::to_string(&m.timeline).unwrap_or_else(|_| "[]".into()),
            spark_json: serde_json::to_string(&sparkline(&m.timeline))
                .unwrap_or_else(|_| "[]".into()),
            endpoint: s.endpoint.clone(),
            protocol: s.protocol,
        }
    }
}

/// Downsamples a timeline to [`SPARK_POINTS`] buckets of median RTT.
pub fn sparkline(points: &[crate::session::Point]) -> Vec<Option<f64>> {
    if points.is_empty() {
        return Vec::new();
    }
    let n = SPARK_POINTS.min(points.len());
    (0..n)
        .map(|i| {
            let from = i * points.len() / n;
            let to = ((i + 1) * points.len() / n).max(from + 1);
            let mut rtts: Vec<f64> = points[from..to].iter().filter_map(|p| p.rtt_ms).collect();
            if rtts.is_empty() {
                return None;
            }
            rtts.sort_by(f64::total_cmp);
            Some((rtts[rtts.len() / 2] * 10.0).round() / 10.0)
        })
        .collect()
}

/// CSV of the history for export. Contains no addresses (records have
/// none). Times are Unix seconds (UTC) so
/// spreadsheets can convert them in any time zone.
pub fn to_csv(records: &[MatchRecord]) -> String {
    let mut out = String::from(
        "started_at_unix,duration_s,kind,method,median_ms,p95_ms,jitter_ms,loss_pct,stalls,bars,packets_in_per_s,protocol,location,region,country\n",
    );
    let opt = |v: Option<f64>| v.map(|x| x.to_string()).unwrap_or_default();
    for r in records {
        let cols = r
            .endpoint
            .as_ref()
            .map(Endpoint::to_columns)
            .unwrap_or_default();
        let fields = [
            r.started_at.to_string(),
            (r.duration_ms / 1000).to_string(),
            r.kind.as_str().to_owned(),
            method_str(r.method).to_owned(),
            opt(r.median_ms),
            opt(r.p95_ms),
            opt(r.jitter_ms),
            opt(r.loss_pct),
            r.stalls.to_string(),
            r.bars.map(|b| b.to_string()).unwrap_or_default(),
            format!("{:.1}", r.rate_pps),
            protocol_str(r.protocol).to_owned(),
            csv_field(&r.endpoint.as_ref().map(Endpoint::label).unwrap_or_default()),
            csv_field(&cols.region.unwrap_or_default()),
            csv_field(&cols.country.unwrap_or_default()),
        ];
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

/// Protocol names as stored and exported.
pub fn protocol_str(p: Protocol) -> &'static str {
    match p {
        Protocol::Udp => "udp",
        Protocol::Tcp => "tcp",
    }
}

pub fn parse_protocol(s: &str) -> Option<Protocol> {
    match s {
        "udp" => Some(Protocol::Udp),
        "tcp" => Some(Protocol::Tcp),
        _ => None,
    }
}

/// Quotes a field if needed (RFC 4180). Also defuses spreadsheet formulas:
/// a note starting with = + - @ is prefixed with ' so Excel won't run it.
fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_owned()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

/// One server city as learned from your own saved matches.
/// Per city (every region there together), never per address.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionStats {
    pub endpoint: Endpoint,
    /// Played matches there.
    pub matches: u32,
    /// Offers (declined or timed out) that would have been there.
    pub offers: u32,
    /// Median of the played matches' medians; offers if none were played.
    pub median_ms: Option<f64>,
    /// Unix seconds.
    pub first_seen: i64,
    pub last_seen: i64,
}

/// One saved record as the catalogue needs it:
/// (endpoint, kind, started_at, median_ms).
pub type ServerRow = (Endpoint, Kind, i64, Option<f64>);

/// Saved server matches → one entry per region, most played first.
pub fn region_stats(rows: &[ServerRow]) -> Vec<RegionStats> {
    struct Acc {
        stats: RegionStats,
        played: Vec<f64>,
        offered: Vec<f64>,
    }
    let mut acc: Vec<Acc> = Vec::new();
    for (endpoint, kind, at, median) in rows {
        if !matches!(endpoint, Endpoint::Server { .. }) {
            continue;
        }
        let area = endpoint.area_key();
        let i = match acc.iter().position(|a| a.stats.endpoint.area_key() == area) {
            Some(i) => i,
            None => {
                acc.push(Acc {
                    stats: RegionStats {
                        endpoint: endpoint.clone(),
                        matches: 0,
                        offers: 0,
                        median_ms: None,
                        first_seen: *at,
                        last_seen: *at,
                    },
                    played: Vec::new(),
                    offered: Vec::new(),
                });
                acc.len() - 1
            }
        };
        let a = &mut acc[i];
        a.stats.first_seen = a.stats.first_seen.min(*at);
        a.stats.last_seen = a.stats.last_seen.max(*at);
        match kind {
            Kind::Match => {
                a.stats.matches += 1;
                a.played.extend(median);
            }
            Kind::Offer => {
                a.stats.offers += 1;
                a.offered.extend(median);
            }
        }
    }
    let mut out: Vec<RegionStats> = acc
        .into_iter()
        .map(|mut a| {
            let pool = if a.played.is_empty() {
                a.offered
            } else {
                a.played
            };
            a.stats.median_ms = median_of(pool);
            a.stats
        })
        .collect();
    out.sort_by(|a, b| {
        (b.matches + b.offers)
            .cmp(&(a.matches + a.offers))
            .then(b.last_seen.cmp(&a.last_seen))
    });
    out
}

fn median_of(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let m = if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    };
    Some((m * 10.0).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> MatchRecord {
        MatchRecord {
            id: 1,
            started_at: 1_791_786_962,
            duration_ms: 612_000,
            kind: Kind::Match,
            method: Method::Icmp,
            median_ms: Some(24.4),
            p95_ms: Some(31.0),
            jitter_ms: Some(4.8),
            loss_pct: Some(0.0),
            stalls: 0,
            bars: Some(5),
            rate_pps: 52.4,
            spark: vec![Some(24.0)],
            endpoint: Some(Endpoint::Peer {
                country: Some("DE".into()),
            }),
            protocol: Protocol::Udp,
        }
    }

    #[test]
    fn sparkline_buckets_medians_and_gaps() {
        let p = |rtt: Option<f64>| crate::session::Point {
            t_ms: 0,
            rtt_ms: rtt,
            jitter_ms: None,
            lost: 0,
            stalls: 0,
            rate_pps: 0.0,
        };
        assert!(sparkline(&[]).is_empty());
        // 48 points -> 24 buckets of 2.
        let mut pts: Vec<_> = (0..48).map(|i| p(Some(f64::from(i)))).collect();
        pts[0] = p(None);
        pts[1] = p(None);
        let s = sparkline(&pts);
        assert_eq!(s.len(), SPARK_POINTS);
        assert_eq!(s[0], None, "no replies in the first bucket");
        assert_eq!(s[1], Some(3.0));
        // Fewer points than buckets: one bucket per point.
        assert_eq!(
            sparkline(&[p(Some(5.0)), p(Some(7.0))]),
            [Some(5.0), Some(7.0)]
        );
    }

    #[test]
    fn offers_are_short() {
        assert_eq!(Kind::for_duration(9_000), Kind::Offer);
        assert_eq!(Kind::for_duration(59_999), Kind::Offer);
        assert_eq!(Kind::for_duration(60_000), Kind::Match);
        assert_eq!(Kind::parse(Kind::Offer.as_str()), Some(Kind::Offer));
        assert_eq!(
            parse_method(method_str(Method::RouteEstimate)),
            Some(Method::RouteEstimate)
        );
    }

    #[test]
    fn csv_has_the_numbers_and_where_but_no_address() {
        let csv = to_csv(&[record()]);
        let lines: Vec<&str> = csv.lines().collect();
        assert!(lines[0].starts_with("started_at_unix,duration_s,kind"));
        assert_eq!(
            lines[1],
            "1791786962,612,match,icmp,24.4,31,4.8,0,0,5,52.4,udp,DE (peer),,DE"
        );
    }

    #[test]
    fn csv_fields_are_quoted_and_formulas_defused() {
        assert_eq!(
            csv_field("lag, then \"fine\""),
            "\"lag, then \"\"fine\"\"\""
        );
        assert_eq!(csv_field("=HYPERLINK(1)"), "'=HYPERLINK(1)");
    }

    #[test]
    fn records_serialise_without_an_address() {
        let json = serde_json::to_string(&record()).unwrap();
        assert!(!json.contains("203.0.113"));
    }

    #[test]
    fn region_stats_group_by_region_and_skip_peers() {
        use crate::classify::place;
        let server = |region: &str| Endpoint::Server {
            region: region.into(),
            place: place(region),
        };
        let fra = server("europe-west3");
        let fra2 = server("eu-central-1");
        let rows = [
            (fra.clone(), Kind::Match, 100, Some(20.0)),
            (fra.clone(), Kind::Match, 300, Some(30.0)),
            (fra.clone(), Kind::Offer, 50, Some(90.0)),
            (fra2.clone(), Kind::Offer, 200, Some(40.0)),
            (
                Endpoint::Peer {
                    country: Some("DE".into()),
                },
                Kind::Match,
                400,
                Some(10.0),
            ),
        ];
        let stats = region_stats(&rows);
        assert_eq!(
            stats.len(),
            1,
            "one Frankfurt for both regions; peers skipped"
        );
        assert_eq!(stats[0].endpoint, fra);
        assert_eq!((stats[0].matches, stats[0].offers), (2, 2));
        assert_eq!(stats[0].median_ms, Some(25.0), "played matches only");
        assert_eq!((stats[0].first_seen, stats[0].last_seen), (50, 300));

        let only_offers = region_stats(&[(fra2, Kind::Offer, 200, Some(40.0))]);
        assert_eq!(
            only_offers[0].median_ms,
            Some(40.0),
            "offers when nothing played"
        );
    }
}
