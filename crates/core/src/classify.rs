//! Who is on the other end of a connection: a cloud **server**
//! (with its region and city), an opponent's own connection
//! (**peer**, country only), something on the local network, or unknown.
//!
//! Everything here is offline. Cloud ranges come from Google Cloud's and AWS's
//! published lists, compiled into the binary (`data/cloud-ranges.tsv`,
//! refreshed by `scripts/update-ip-data.mjs`). Country and network names come
//! from a [`GeoLookup`], which the app backs with the DB-IP Lite databases and
//! tests back with a fake. The address itself never leaves Rust: callers keep
//! only the [`Endpoint`].

use std::net::{IpAddr, Ipv4Addr};
use std::sync::LazyLock;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Continent {
    Europe,
    NorthAmerica,
    SouthAmerica,
    Asia,
    Oceania,
    Africa,
    MiddleEast,
}

/// Where a cloud region is. `city` is what players see ("Frankfurt").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub city: &'static str,
    /// ISO 3166-1 alpha-2.
    pub country: &'static str,
    pub continent: Continent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Endpoint {
    /// In a published cloud range (Google Cloud or AWS).
    Server {
        /// Cloud region code, e.g. `europe-west3`.
        region: String,
        /// `None` for a region we can't place yet (shown by its code).
        place: Option<Place>,
    },
    /// Looks like a home or mobile connection: the opponent in a
    /// peer-to-peer match. Country only, never anything finer.
    Peer { country: Option<String> },
    /// Private, loopback, link-local or carrier-grade NAT address.
    Local,
    /// Public, but not in a known cloud range. For known hosting operators
    /// (likely a server) `network` and `country` are filled; otherwise both
    /// are `None`, since the network could be someone's home ISP.
    Unknown {
        network: Option<String>,
        country: Option<String>,
    },
}

impl Endpoint {
    /// What players think of as "the same place": a server's city, so every
    /// region in Frankfurt is one Frankfurt. Regions without a known city
    /// stay separate by code. Other endpoints group by label.
    pub fn area_key(&self) -> String {
        match self {
            Endpoint::Server { place: Some(p), .. } => format!("server:{}|{}", p.city, p.country),
            Endpoint::Server { region, .. } => format!("server:{region}"),
            other => other.label(),
        }
    }

    /// Short label for cards and lists: the server's city ("Toronto"), or
    /// the peer's country ("CA (peer)"; the UI shows the country's name).
    pub fn label(&self) -> String {
        match self {
            Endpoint::Server { region, place, .. } => match place {
                Some(p) => p.city.to_owned(),
                None => region.clone(),
            },
            Endpoint::Peer { country: Some(c) } => format!("{c} (peer)"),
            Endpoint::Peer { country: None } => "Peer".to_owned(),
            Endpoint::Local => "Local network".to_owned(),
            Endpoint::Unknown {
                network: Some(n),
                country,
            } => match country {
                Some(c) => format!("Unclassified · {n}, {c}"),
                None => format!("Unclassified · {n}"),
            },
            Endpoint::Unknown { network: None, .. } => "Unclassified".to_owned(),
        }
    }
}

/// An [`Endpoint`] as flat columns, for storage: kind, region,
/// country, network. The city is not stored; it is looked up from the
/// region when read, so fixing the region map fixes old records too.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EndpointColumns {
    pub kind: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub network: Option<String>,
}

impl Endpoint {
    pub fn to_columns(&self) -> EndpointColumns {
        let kind = |k: &str| Some(k.to_owned());
        match self {
            Endpoint::Server { region, .. } => EndpointColumns {
                kind: kind("server"),
                region: Some(region.clone()),
                ..Default::default()
            },
            Endpoint::Peer { country } => EndpointColumns {
                kind: kind("peer"),
                country: country.clone(),
                ..Default::default()
            },
            Endpoint::Local => EndpointColumns {
                kind: kind("local"),
                ..Default::default()
            },
            Endpoint::Unknown { network, country } => EndpointColumns {
                kind: kind("unknown"),
                network: network.clone(),
                country: country.clone(),
                ..Default::default()
            },
        }
    }

    /// `None` for records saved before locations existed, or unreadable ones.
    pub fn from_columns(c: EndpointColumns) -> Option<Endpoint> {
        Some(match c.kind.as_deref()? {
            "server" => {
                let region = c.region?;
                let place = place(&region);
                Endpoint::Server { region, place }
            }
            "peer" => Endpoint::Peer { country: c.country },
            "local" => Endpoint::Local,
            "unknown" => Endpoint::Unknown {
                network: c.network,
                country: c.country,
            },
            _ => return None,
        })
    }
}

/// Offline country and network-owner lookups (DB-IP Lite in the app).
pub trait GeoLookup {
    /// ISO 3166-1 alpha-2 country code.
    fn country(&self, ip: IpAddr) -> Option<String>;
    /// Name of the organisation that owns the address's network (ASN owner).
    fn network(&self, ip: IpAddr) -> Option<String>;
}

/// No database available: cloud ranges still work, everything else is
/// unclassified.
pub struct NoGeo;

impl GeoLookup for NoGeo {
    fn country(&self, _: IpAddr) -> Option<String> {
        None
    }
    fn network(&self, _: IpAddr) -> Option<String> {
        None
    }
}

/// How a public, non-cloud address is classified (the heuristic's choice of
/// error): a network whose owner is on the curated [`HOSTING`] list is a
/// likely server (`Unknown` with its name and country); any other network
/// is assumed to be a home or mobile ISP, so a **peer** (country only).
/// A server on an unlisted hosting company therefore reads as a peer; the
/// fix is adding that company to [`HOSTING`], never matching generic words
/// like "cloud", which home ISPs use too ("Google Fiber", "Cloudlasers
/// Broadband", "Amazonet Telecom").
pub fn classify(ip: IpAddr, geo: &dyn GeoLookup) -> Endpoint {
    let ip = unwrap_nat64(ip.to_canonical()); // IPv4-mapped / NAT64 → IPv4
    if is_local(ip) {
        return Endpoint::Local;
    }
    if let Some(range) = cloud_range(ip) {
        return Endpoint::Server {
            region: range.region.to_owned(),
            place: place(range.region),
        };
    }
    match geo.network(ip) {
        Some(org) if is_hosting(&org) => Endpoint::Unknown {
            network: Some(org),
            country: geo.country(ip),
        },
        Some(_) => Endpoint::Peer {
            country: geo.country(ip),
        },
        // No network data: we can't tell a peer from a server.
        None => Endpoint::Unknown {
            network: None,
            country: None,
        },
    }
}

/// IPv6-only mobile networks reach IPv4 servers through NAT64, which embeds
/// the IPv4 address in the last 32 bits of `64:ff9b::/96`.
fn unwrap_nat64(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) if v6.segments()[..6] == [0x64, 0xff9b, 0, 0, 0, 0] => {
            let [.., a, b, c, d] = v6.octets();
            IpAddr::V4(Ipv4Addr::new(a, b, c, d))
        }
        other => other,
    }
}

/// Addresses that never belong to a remote server or player: private,
/// loopback, link-local, CGNAT and other special ranges.
pub fn is_local(ip: IpAddr) -> bool {
    let ip = unwrap_nat64(ip.to_canonical());
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || a == 0 // "this network" 0.0.0.0/8
                || a >= 240 // reserved 240.0.0.0/4
                || (a == 100 && (64..128).contains(&b)) // CGNAT 100.64.0.0/10
                || (a == 192 && b == 0 && c == 0) // IETF 192.0.0.0/24
                || (a == 198 && (b == 18 || b == 19)) // benchmarking 198.18.0.0/15
        }
        IpAddr::V6(v6) => {
            let [first, second, ..] = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (first & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (first == 0x2001 && second == 0x0db8) // documentation
        }
    }
}

/// Hosting operators, matched as whole words in the network owner's name.
/// Built from a scan of every owner name in DB-IP ASN Lite (2026-10):
/// each entry hits only that operator, or (for the generic phrases at the
/// end) companies that sell hosting. Words that home ISPs also use
/// ("cloud", "server", "google", "amazon", "vps") are deliberately absent.
const HOSTING: &[&str] = &[
    "amazon.com",
    "amazon technologies",
    "amazon data services",
    "google llc",
    "google cloud",
    "microsoft corporation",
    "microsoft limited",
    "akamai",
    "cloudflare",
    "fastly",
    "oracle corporation",
    "alibaba cloud",
    "alibaba (us) technology",
    "tencent",
    "ovh",
    "hetzner",
    "digitalocean",
    "leaseweb",
    "the constant company",
    "contabo",
    "m247",
    "zenlayer",
    "scaleway",
    "equinix",
    "psychz",
    "i3d.net",
    "g-core",
    "datacamp",
    "hostinger",
    "ionos",
    "servers.com",
    "limestone networks",
    "edgenext",
    "konami",
    "hosting",
    "data center",
    "datacenter",
    "data centre",
    "colocation",
];

fn is_hosting(org: &str) -> bool {
    let org = org.to_lowercase();
    HOSTING.iter().any(|w| contains_word(&org, w))
}

/// `needle` occurs in `hay` with no letter or digit right before or after.
fn contains_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

// ---------------------------------------------------------------- cloud ranges

struct Range {
    /// IPv4 addresses are stored as IPv4-mapped values so one compare works.
    base: u128,
    /// Prefix length on the 128-bit value.
    bits: u8,
    region: &'static str,
}

const RANGES_TSV: &str = include_str!("../data/cloud-ranges.tsv");

static RANGES: LazyLock<Vec<Range>> = LazyLock::new(|| {
    RANGES_TSV
        .lines()
        .filter_map(|l| parse_line(l).ok().flatten())
        .collect()
});

/// `Ok(None)` for comments and blank lines, `Err` for a malformed line.
fn parse_line(line: &'static str) -> Result<Option<Range>, String> {
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let mut cols = line.split('\t');
    let (Some(cidr), Some(region), None) = (cols.next(), cols.next(), cols.next()) else {
        return Err(format!("expected 2 columns: {line}"));
    };
    let (addr, len) = cidr
        .split_once('/')
        .ok_or_else(|| format!("no prefix: {cidr}"))?;
    let len: u8 = len.parse().map_err(|_| format!("bad prefix: {cidr}"))?;
    let addr: IpAddr = addr.parse().map_err(|_| format!("bad address: {cidr}"))?;
    let bits = match addr {
        IpAddr::V4(_) if len <= 32 => 96 + len,
        IpAddr::V6(_) if len <= 128 => len,
        _ => return Err(format!("prefix too long: {cidr}")),
    };
    Ok(Some(Range {
        base: to_u128(addr) & mask(bits),
        bits,
        region,
    }))
}

fn to_u128(ip: IpAddr) -> u128 {
    match ip {
        IpAddr::V4(v4) => u128::from(v4.to_ipv6_mapped()),
        IpAddr::V6(v6) => u128::from(v6),
    }
}

fn mask(bits: u8) -> u128 {
    if bits == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(bits))
    }
}

/// Regions whose city (or code, for unplaced regions) contains `query`,
/// ignoring case: what a city search in history matches.
pub fn regions_matching(query: &str) -> Vec<&'static str> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<&'static str> = Vec::new();
    for r in RANGES.iter() {
        if out.contains(&r.region) {
            continue;
        }
        let name = place(r.region).map_or(r.region, |p| p.city);
        if name.to_lowercase().contains(&q) {
            out.push(r.region);
        }
    }
    out
}

/// Every published range in a city (`area` from [`Endpoint::area_key`]),
/// as (address, prefix length). For the region picker.
pub fn ranges_in_area(area: &str) -> Vec<(IpAddr, u8)> {
    RANGES
        .iter()
        .filter(|r| {
            Endpoint::Server {
                region: r.region.to_owned(),
                place: place(r.region),
            }
            .area_key()
                == area
        })
        .map(|r| {
            if r.base >> 32 == 0xffff && r.bits >= 96 {
                let v4 = std::net::Ipv4Addr::from(
                    u32::try_from(r.base & u128::from(u32::MAX)).unwrap_or(0),
                );
                (IpAddr::V4(v4), r.bits - 96)
            } else {
                (IpAddr::V6(std::net::Ipv6Addr::from(r.base)), r.bits)
            }
        })
        .collect()
}

/// The most specific published range containing `ip` (AWS nests prefixes).
/// A linear scan (~12k compares) is fine: results are cached per address
/// while a flow uses it, so this runs about once per new remote.
fn cloud_range(ip: IpAddr) -> Option<&'static Range> {
    best_range(&RANGES, ip)
}

fn best_range(ranges: &[Range], ip: IpAddr) -> Option<&Range> {
    let v = to_u128(ip);
    ranges
        .iter()
        .filter(|r| v & mask(r.bits) == r.base)
        .max_by_key(|r| r.bits)
}

// ----------------------------------------------------------------- region map

use Continent::*;

const fn p(city: &'static str, country: &'static str, continent: Continent) -> Place {
    Place {
        city,
        country,
        continent,
    }
}

/// Regions whose location isn't published (yet). They show by region code.
/// A refreshed range file with a region in neither list fails the tests.
pub const UNPLACED: &[&str] = &[
    // Google Cloud
    "europe-west15",
    "us-central2",
    "us-east7",
    "us-west8",
    // AWS
    "me-west-1",
    "sa-west-1",
    "us-south-1",
];

/// The city for a region code. Google Cloud and AWS name regions
/// differently (`europe-west3` vs `eu-central-1`), so a code alone is enough.
pub fn place(region: &str) -> Option<Place> {
    Some(match region {
        // Google Cloud
        "africa-south1" => p("Johannesburg", "ZA", Africa),
        "asia-east1" => p("Taiwan", "TW", Asia),
        "asia-east2" => p("Hong Kong", "HK", Asia),
        "asia-northeast1" => p("Tokyo", "JP", Asia),
        "asia-northeast2" => p("Osaka", "JP", Asia),
        "asia-northeast3" => p("Seoul", "KR", Asia),
        "asia-south1" => p("Mumbai", "IN", Asia),
        "asia-south2" => p("Delhi", "IN", Asia),
        "asia-southeast1" => p("Singapore", "SG", Asia),
        "asia-southeast2" => p("Jakarta", "ID", Asia),
        "asia-southeast3" => p("Bangkok", "TH", Asia),
        "australia-southeast1" => p("Sydney", "AU", Oceania),
        "australia-southeast2" => p("Melbourne", "AU", Oceania),
        "europe-central2" => p("Warsaw", "PL", Europe),
        "europe-north1" => p("Finland", "FI", Europe),
        "europe-north2" => p("Stockholm", "SE", Europe),
        "europe-southwest1" => p("Madrid", "ES", Europe),
        "europe-west1" => p("Belgium", "BE", Europe),
        "europe-west2" => p("London", "GB", Europe),
        "europe-west3" => p("Frankfurt", "DE", Europe),
        "europe-west4" => p("Netherlands", "NL", Europe),
        "europe-west6" => p("Zurich", "CH", Europe),
        "europe-west8" => p("Milan", "IT", Europe),
        "europe-west9" => p("Paris", "FR", Europe),
        "europe-west10" => p("Berlin", "DE", Europe),
        "europe-west12" => p("Turin", "IT", Europe),
        "me-central1" => p("Doha", "QA", MiddleEast),
        "me-central2" => p("Dammam", "SA", MiddleEast),
        "me-west1" => p("Tel Aviv", "IL", MiddleEast),
        "northamerica-northeast1" => p("Montréal", "CA", NorthAmerica),
        "northamerica-northeast2" => p("Toronto", "CA", NorthAmerica),
        "northamerica-south1" => p("Querétaro", "MX", NorthAmerica),
        "southamerica-east1" => p("São Paulo", "BR", SouthAmerica),
        "southamerica-west1" => p("Santiago", "CL", SouthAmerica),
        "us-central1" => p("Iowa", "US", NorthAmerica),
        "us-east1" => p("South Carolina", "US", NorthAmerica),
        "us-east4" => p("Virginia", "US", NorthAmerica),
        "us-east5" => p("Columbus", "US", NorthAmerica),
        "us-south1" => p("Dallas", "US", NorthAmerica),
        "us-west1" => p("Oregon", "US", NorthAmerica),
        "us-west2" => p("Los Angeles", "US", NorthAmerica),
        "us-west3" => p("Salt Lake City", "US", NorthAmerica),
        "us-west4" => p("Las Vegas", "US", NorthAmerica),
        // AWS
        "af-south-1" => p("Cape Town", "ZA", Africa),
        "ap-east-1" => p("Hong Kong", "HK", Asia),
        "ap-east-2" => p("Taipei", "TW", Asia),
        "ap-northeast-1" => p("Tokyo", "JP", Asia),
        "ap-northeast-2" => p("Seoul", "KR", Asia),
        "ap-northeast-3" => p("Osaka", "JP", Asia),
        "ap-south-1" => p("Mumbai", "IN", Asia),
        "ap-south-2" => p("Hyderabad", "IN", Asia),
        "ap-southeast-1" => p("Singapore", "SG", Asia),
        "ap-southeast-2" => p("Sydney", "AU", Oceania),
        "ap-southeast-3" => p("Jakarta", "ID", Asia),
        "ap-southeast-4" => p("Melbourne", "AU", Oceania),
        "ap-southeast-5" => p("Kuala Lumpur", "MY", Asia),
        "ap-southeast-6" => p("Auckland", "NZ", Oceania),
        "ap-southeast-7" => p("Bangkok", "TH", Asia),
        "ca-central-1" => p("Montréal", "CA", NorthAmerica),
        "ca-west-1" => p("Calgary", "CA", NorthAmerica),
        "cn-north-1" => p("Beijing", "CN", Asia),
        "cn-northwest-1" => p("Ningxia", "CN", Asia),
        "eu-central-1" => p("Frankfurt", "DE", Europe),
        "eu-central-2" => p("Zurich", "CH", Europe),
        "eu-north-1" => p("Stockholm", "SE", Europe),
        "eu-south-1" => p("Milan", "IT", Europe),
        "eu-south-2" => p("Spain", "ES", Europe),
        "eu-west-1" => p("Ireland", "IE", Europe),
        "eu-west-2" => p("London", "GB", Europe),
        "eu-west-3" => p("Paris", "FR", Europe),
        "eusc-de-east-1" => p("Brandenburg", "DE", Europe),
        "il-central-1" => p("Tel Aviv", "IL", MiddleEast),
        "me-central-1" => p("UAE", "AE", MiddleEast),
        "me-south-1" => p("Bahrain", "BH", MiddleEast),
        "mx-central-1" => p("Mexico", "MX", NorthAmerica),
        "sa-east-1" => p("São Paulo", "BR", SouthAmerica),
        "us-east-1" => p("N. Virginia", "US", NorthAmerica),
        "us-east-2" => p("Ohio", "US", NorthAmerica),
        "us-gov-east-1" => p("GovCloud East", "US", NorthAmerica),
        "us-gov-west-1" => p("GovCloud West", "US", NorthAmerica),
        "us-west-1" => p("N. California", "US", NorthAmerica),
        "us-west-2" => p("Oregon", "US", NorthAmerica),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::net::Ipv6Addr;

    struct FakeGeo {
        org: Option<&'static str>,
        country: Option<&'static str>,
    }
    impl GeoLookup for FakeGeo {
        fn country(&self, _: IpAddr) -> Option<String> {
            self.country.map(str::to_owned)
        }
        fn network(&self, _: IpAddr) -> Option<String> {
            self.org.map(str::to_owned)
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// A Google Cloud `europe-west3` IPv4 address from the committed table.
    fn frankfurt_ip() -> IpAddr {
        let r = RANGES
            .iter()
            .find(|r| {
                r.region == "europe-west3"
                    && r.base >> 32 == 0xffff // IPv4-mapped
                    && r.bits <= 96 + 30 // room for a host address
            })
            .expect("europe-west3 range");
        let v4 = u32::try_from(r.base & u128::from(u32::MAX)).unwrap();
        IpAddr::V4(Ipv4Addr::from(v4 + 1))
    }

    #[test]
    fn range_table_parses_cleanly() {
        let mut bad = Vec::new();
        let mut count = 0;
        for line in RANGES_TSV.lines() {
            match parse_line(line) {
                Ok(Some(_)) => count += 1,
                Ok(None) => {}
                Err(e) => bad.push(e),
            }
        }
        assert!(bad.is_empty(), "malformed lines: {bad:?}");
        assert!(count >= 4_000, "only {count} ranges");
        assert_eq!(RANGES.len(), count);
    }

    #[test]
    fn every_region_is_placed_or_explicitly_unplaced() {
        let regions: HashSet<&str> = RANGES.iter().map(|r| r.region).collect();
        let missing: Vec<_> = regions
            .iter()
            .filter(|rg| place(rg).is_none() && !UNPLACED.contains(rg))
            .collect();
        assert!(
            missing.is_empty(),
            "add these regions to place(): {missing:?}"
        );
        for rg in UNPLACED {
            assert!(place(rg).is_none(), "{rg} is placed, drop it from UNPLACED");
        }
    }

    #[test]
    fn cloud_address_is_a_server_with_its_city() {
        let e = classify(frankfurt_ip(), &NoGeo);
        let Endpoint::Server { region, place } = &e else {
            panic!("{e:?}")
        };
        assert_eq!(region, "europe-west3");
        assert_eq!(place.unwrap().city, "Frankfurt");
        assert_eq!(e.label(), "Frankfurt");
    }

    #[test]
    fn ipv4_mapped_ipv6_is_treated_as_ipv4() {
        let IpAddr::V4(v4) = frankfurt_ip() else {
            unreachable!()
        };
        let e = classify(IpAddr::V6(v4.to_ipv6_mapped()), &NoGeo);
        assert!(matches!(e, Endpoint::Server { .. }), "{e:?}");
    }

    #[test]
    fn most_specific_prefix_wins() {
        let wide = parse_line("10.0.0.0/8\tus-east-1").unwrap().unwrap();
        let narrow = parse_line("10.1.2.0/24\teu-central-1").unwrap().unwrap();
        let ranges = [wide, narrow];
        assert_eq!(
            best_range(&ranges, ip("10.1.2.3")).unwrap().region,
            "eu-central-1"
        );
        assert_eq!(
            best_range(&ranges, ip("10.9.0.1")).unwrap().region,
            "us-east-1"
        );
        assert!(best_range(&ranges, ip("11.0.0.1")).is_none());
    }

    #[test]
    fn local_and_special_addresses_are_local() {
        for s in [
            "192.168.1.1",
            "10.0.0.1",
            "172.16.5.4",
            "127.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "100.127.255.254",
            "::1",
            "fe80::1",
            "fd00::1",
        ] {
            assert_eq!(classify(ip(s), &NoGeo), Endpoint::Local, "{s}");
        }
        for s in [
            "0.1.2.3",
            "192.0.0.8",
            "198.18.0.1",
            "240.0.0.1",
            "2001:db8::1",
        ] {
            assert_eq!(classify(ip(s), &NoGeo), Endpoint::Local, "{s}");
        }
        // Just outside CGNAT is public.
        assert_ne!(classify(ip("100.128.0.1"), &NoGeo), Endpoint::Local);
    }

    #[test]
    fn residential_network_is_a_peer_with_country_only() {
        let geo = FakeGeo {
            org: Some("Deutsche Telekom AG"),
            country: Some("DE"),
        };
        let e = classify(ip("80.128.0.1"), &geo);
        assert_eq!(
            e,
            Endpoint::Peer {
                country: Some("DE".into())
            }
        );
        assert_eq!(e.label(), "DE (peer)");
        // The ISP name is never part of a peer.
        assert!(!serde_json::to_string(&e).unwrap().contains("Telekom"));
    }

    #[test]
    fn hosting_network_is_unclassified_with_its_name() {
        let geo = FakeGeo {
            org: Some("Hetzner Online GmbH"),
            country: Some("DE"),
        };
        let e = classify(ip("88.198.0.1"), &geo);
        assert_eq!(
            e,
            Endpoint::Unknown {
                network: Some("Hetzner Online GmbH".into()),
                country: Some("DE".into()),
            }
        );
        assert_eq!(e.label(), "Unclassified · Hetzner Online GmbH, DE");
    }

    #[test]
    fn home_isps_with_hosting_like_names_stay_peers_without_their_name() {
        // Real owner names from DB-IP that a plain word match got wrong.
        for org in [
            "Google Fiber Inc.",
            "AMAZONET TELECOM LTDA",
            "Cloudlasers Broadband",
            "IWserver Internet Banda Larga Ltda.",
            "VPS Broadband And Telecommunications Pvt Ltd",
            "CHINANET SHAANXI province Cloud Base network",
        ] {
            let geo = FakeGeo {
                org: Some(org),
                country: Some("BR"),
            };
            let e = classify(ip("80.128.0.1"), &geo);
            assert_eq!(
                e,
                Endpoint::Peer {
                    country: Some("BR".into())
                },
                "{org}"
            );
            let json = serde_json::to_string(&e).unwrap();
            assert!(!json.contains(&org[..6]), "{org} leaked: {json}");
        }
    }

    #[test]
    fn hosting_words_match_whole_words_only() {
        assert!(is_hosting("OVH SAS"));
        assert!(is_hosting("i3D.net B.V"));
        assert!(is_hosting("Amazon.com, Inc."));
        assert!(!is_hosting("Movhe Telecom")); // "ovh" inside a word
        assert!(!is_hosting("Hostingtonia Cable")); // "hosting" inside a word
    }

    #[test]
    fn nat64_addresses_classify_as_their_ipv4() {
        let IpAddr::V4(v4) = frankfurt_ip() else {
            unreachable!()
        };
        let [a, b, c, d] = v4.octets();
        let nat64 = Ipv6Addr::new(
            0x64,
            0xff9b,
            0,
            0,
            0,
            0,
            u16::from_be_bytes([a, b]),
            u16::from_be_bytes([c, d]),
        );
        assert!(matches!(
            classify(IpAddr::V6(nat64), &NoGeo),
            Endpoint::Server { .. }
        ));
    }

    #[test]
    fn without_a_database_public_non_cloud_is_unclassified() {
        assert_eq!(
            classify(ip("80.128.0.1"), &NoGeo),
            Endpoint::Unknown {
                network: None,
                country: None
            }
        );
    }

    #[test]
    fn unplaced_region_label_uses_the_code() {
        let e = Endpoint::Server {
            region: "us-west8".into(),
            place: None,
        };
        assert_eq!(e.label(), "us-west8");
    }

    #[test]
    fn serialises_with_a_kind_tag() {
        let json = serde_json::to_value(classify(frankfurt_ip(), &NoGeo)).unwrap();
        assert_eq!(json["kind"], "server");
        assert!(json.get("provider").is_none());
        assert_eq!(json["place"]["city"], "Frankfurt");
        assert_eq!(json["place"]["continent"], "europe");
    }

    #[test]
    fn endpoints_round_trip_through_columns() {
        let cases = [
            classify(frankfurt_ip(), &NoGeo),
            Endpoint::Peer {
                country: Some("BR".into()),
            },
            Endpoint::Peer { country: None },
            Endpoint::Local,
            Endpoint::Unknown {
                network: Some("OVH SAS".into()),
                country: Some("FR".into()),
            },
            Endpoint::Unknown {
                network: None,
                country: None,
            },
        ];
        for e in cases {
            assert_eq!(Endpoint::from_columns(e.to_columns()), Some(e.clone()));
        }
        assert_eq!(Endpoint::from_columns(EndpointColumns::default()), None);
        let bad = EndpointColumns {
            kind: Some("server".into()),
            ..Default::default()
        };
        assert_eq!(Endpoint::from_columns(bad), None);
    }

    #[test]
    fn city_search_finds_every_region_there() {
        let found = regions_matching("frank");
        assert!(found.contains(&"europe-west3"));
        assert!(found.contains(&"eu-central-1"));
        assert!(regions_matching("FRANKFURT").len() == found.len());
        assert!(regions_matching("atlantis").is_empty());
    }

    #[test]
    fn public_ipv6_is_not_local() {
        assert!(is_local(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_local(ip("2a00:1450::1")));
    }
}
