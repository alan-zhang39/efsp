//! The region picker: turns the cities a player switched
//! off into Windows Firewall rules, and back.
//!
//! Pure. The app builds a [`HelperRequest`]; the small elevated helper
//! (`crates/helper`) re-validates it and runs the netsh commands from
//! [`crate::firewall::netsh`]. Rules block outbound **UDP only**, from
//! eFootball.exe only, to the published Google Cloud/AWS ranges of each
//! switched-off city. Login, lobby and store traffic (HTTPS, TCP) is never
//! affected, and neither are peers (they're not in cloud ranges).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::classify::ranges_in_area;
use crate::firewall::{
    Cidr, FirewallError, MAX_RANGES_PER_RULE, PICKER_RULE_LABEL, Rule, RuleName, validate,
};

/// Upper bound on rules in one request (every city off is ~80 rules).
pub const MAX_RULES: usize = 400;

/// A city to switch off: its key ([`crate::classify::Endpoint::area_key`])
/// and the name shown in Windows Firewall's description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Area {
    pub key: String,
    pub name: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PickerError {
    #[error("no published ranges for {0}")]
    UnknownArea(String),
    #[error("too many rules ({0})")]
    TooManyRules(usize),
    #[error(transparent)]
    Firewall(#[from] FirewallError),
}

/// Merges ranges that touch or nest, per address family, so fewer and
/// shorter rules cover exactly the same addresses. Never wider than the
/// [`Cidr`] limits (/8 for IPv4, /16 for IPv6).
pub fn collapse(ranges: &[(IpAddr, u8)]) -> Vec<Cidr> {
    let mut v4: Vec<(u128, u8)> = Vec::new();
    let mut v6: Vec<(u128, u8)> = Vec::new();
    for &(addr, len) in ranges {
        match addr {
            IpAddr::V4(a) => v4.push((u128::from(u32::from(a)) << 96, len)),
            IpAddr::V6(a) => v6.push((u128::from(a), len)),
        }
    }
    let mut out = Vec::new();
    for (net, len) in merge(v4, crate::firewall::MIN_PREFIX_V4) {
        let a = Ipv4Addr::from(u32::try_from(net >> 96).unwrap_or(0));
        out.extend(Cidr::new(IpAddr::V4(a), len).ok());
    }
    for (net, len) in merge(v6, crate::firewall::MIN_PREFIX_V6) {
        out.extend(Cidr::new(IpAddr::V6(Ipv6Addr::from(net)), len).ok());
    }
    out
}

fn mask(bits: u8) -> u128 {
    if bits == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(bits))
    }
}

/// `nets`: left-aligned network values with prefix lengths (0–32 for IPv4
/// shifted up, 0–128 for IPv6). Removes contained ranges and joins sibling
/// halves until nothing changes.
fn merge(nets: Vec<(u128, u8)>, min_len: u8) -> Vec<(u128, u8)> {
    let mut v: Vec<(u128, u8)> = nets.into_iter().map(|(n, l)| (n & mask(l), l)).collect();
    loop {
        v.sort_unstable();
        // Drop anything inside the range before it.
        let mut kept: Vec<(u128, u8)> = Vec::with_capacity(v.len());
        for (n, l) in v {
            if let Some(&(pn, pl)) = kept.last()
                && pl <= l
                && n & mask(pl) == pn
            {
                continue;
            }
            kept.push((n, l));
        }
        // Join two halves of the same parent.
        let mut joined = false;
        let mut next = Vec::with_capacity(kept.len());
        let mut i = 0;
        while i < kept.len() {
            let (n, l) = kept[i];
            if l > min_len
                && let Some(&(m, ml)) = kept.get(i + 1)
                && ml == l
                && n & mask(l - 1) == n
                && m == n | (1u128 << (128 - u32::from(l)))
            {
                next.push((n, l - 1));
                i += 2;
                joined = true;
                continue;
            }
            next.push((n, l));
            i += 1;
        }
        v = next;
        if !joined {
            return v;
        }
    }
}

/// Rules blocking every switched-off city, at most
/// [`MAX_RANGES_PER_RULE`] ranges each. One city per rule, so Windows
/// Firewall's description says which city a rule is for.
pub fn build_rules(off: &[Area], program: &str) -> Result<Vec<Rule>, PickerError> {
    let name = RuleName::new(PICKER_RULE_LABEL)?;
    let mut rules = Vec::new();
    for area in off {
        let ranges = collapse(&ranges_in_area(&area.key));
        if ranges.is_empty() {
            return Err(PickerError::UnknownArea(area.name.clone()));
        }
        for chunk in ranges.chunks(MAX_RANGES_PER_RULE) {
            let rule = Rule {
                name: name.clone(),
                description: format!("efsp: blocks {}", safe_text(&area.name)),
                program: PathBuf::from(program),
                remote: chunk.to_vec(),
            };
            validate(&rule)?;
            rules.push(rule);
        }
    }
    if rules.len() > MAX_RULES {
        return Err(PickerError::TooManyRules(rules.len()));
    }
    Ok(rules)
}

/// Keeps only characters a rule description may carry.
fn safe_text(s: &str) -> String {
    s.chars()
        .filter(|c| crate::firewall::description_char(*c))
        .take(120)
        .collect()
}

/// Most cities in one request (there are fewer than 100).
pub const MAX_AREAS: usize = 200;

/// What the app asks the elevated helper to do. It carries **cities, not
/// address ranges**: the helper looks the ranges up in its own built-in
/// table (via [`build_rules`]), so a tampered request can't make it block
/// anything except published cloud ranges of known cities, for
/// eFootball.exe, outbound UDP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HelperRequest {
    /// Remove all of efsp's rules, then block these cities.
    Apply { program: String, areas: Vec<Area> },
    /// Remove all of efsp's rules.
    RemoveAll,
}

impl HelperRequest {
    /// The rules to add (empty for [`HelperRequest::RemoveAll`]), built
    /// and validated here.
    pub fn rules(&self) -> Result<Vec<Rule>, PickerError> {
        let HelperRequest::Apply { program, areas } = self else {
            return Ok(Vec::new());
        };
        if areas.len() > MAX_AREAS {
            return Err(PickerError::TooManyRules(areas.len()));
        }
        build_rules(areas, program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = r"C:\Program Files (x86)\Steam\steamapps\common\eFootball\eFootball.exe";

    fn frankfurt() -> Area {
        Area {
            key: "server:Frankfurt|DE".into(),
            name: "Frankfurt".into(),
        }
    }

    fn contains(c: &Cidr, ip: IpAddr) -> bool {
        match (c.addr(), ip) {
            (IpAddr::V4(n), IpAddr::V4(a)) => {
                let m = if c.prefix_len() == 0 {
                    0
                } else {
                    u32::MAX << (32 - u32::from(c.prefix_len()))
                };
                u32::from(a) & m == u32::from(n) & m
            }
            (IpAddr::V6(n), IpAddr::V6(a)) => {
                let m = mask(c.prefix_len());
                u128::from(a) & m == u128::from(n) & m
            }
            _ => false,
        }
    }

    #[test]
    fn collapse_merges_siblings_and_drops_nested_ranges() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let out = collapse(&[
            (ip("10.0.0.0"), 25),
            (ip("10.0.0.128"), 25), // sibling → 10.0.0.0/24
            (ip("10.0.0.7"), 32),   // inside
            (ip("10.0.2.0"), 24),   // not adjacent to a sibling
            (ip("2600:1900::"), 32),
            (ip("2600:1900:1::"), 48), // inside
        ]);
        let text: Vec<String> = out
            .iter()
            .map(|c| format!("{}/{}", c.addr(), c.prefix_len()))
            .collect();
        assert_eq!(text, ["10.0.0.0/24", "10.0.2.0/24", "2600:1900::/32"]);
    }

    #[test]
    fn collapse_never_goes_wider_than_a_slash_8() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let out = collapse(&[(ip("10.0.0.0"), 8), (ip("11.0.0.0"), 8)]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn frankfurt_rules_cover_every_published_range_and_nothing_else() {
        let rules = build_rules(&[frankfurt()], GAME).unwrap();
        assert!(!rules.is_empty());
        let raw = ranges_in_area("server:Frankfurt|DE");
        assert!(raw.len() > 100, "Google and AWS Frankfurt together");
        let all: Vec<&Cidr> = rules.iter().flat_map(|r| &r.remote).collect();
        // Every published address is covered...
        for (addr, _) in &raw {
            assert!(all.iter().any(|c| contains(c, *addr)), "{addr} not blocked");
        }
        // ...and every rule's range starts inside a published one.
        for c in &all {
            assert!(
                raw.iter().any(|(a, l)| {
                    Cidr::new(*a, *l).is_ok_and(|p| contains(&p, c.addr())) || contains(c, *a)
                }),
                "{c:?} isn't from Frankfurt"
            );
        }
        for r in &rules {
            assert_eq!(r.name.as_str(), "efsp: region picker");
            assert_eq!(r.description, "efsp: blocks Frankfurt");
            assert!(r.remote.len() <= MAX_RANGES_PER_RULE);
        }
    }

    #[test]
    fn building_is_deterministic() {
        assert_eq!(
            build_rules(&[frankfurt()], GAME).unwrap(),
            build_rules(&[frankfurt()], GAME).unwrap()
        );
    }

    #[test]
    fn unknown_cities_and_wrong_programs_are_refused() {
        let nowhere = Area {
            key: "server:Atlantis|XX".into(),
            name: "Atlantis".into(),
        };
        assert_eq!(
            build_rules(&[nowhere], GAME),
            Err(PickerError::UnknownArea("Atlantis".into()))
        );
        assert!(matches!(
            build_rules(&[frankfurt()], r"C:\Windows\System32\svchost.exe"),
            Err(PickerError::Firewall(FirewallError::BadProgram(_)))
        ));
    }

    #[test]
    fn requests_carry_cities_and_the_helper_side_builds_the_rules() {
        let req = HelperRequest::Apply {
            program: GAME.into(),
            areas: vec![frankfurt()],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains('/'), "no address ranges in the request");
        let back: HelperRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.rules().unwrap(),
            build_rules(&[frankfurt()], GAME).unwrap()
        );

        // Tampering can't widen what gets blocked.
        let tampered = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut v = serde_json::to_value(&req).unwrap();
            f(&mut v);
            serde_json::from_value::<HelperRequest>(v)
                .map_err(|e| e.to_string())
                .and_then(|r| r.rules().map_err(|e| e.to_string()))
        };
        assert!(tampered(&|v| v["program"] = r"C:\Windows\explorer.exe".into()).is_err());
        assert!(tampered(&|v| v["areas"][0]["key"] = "server:Nowhere|XX".into()).is_err());
        // A hostile name is cleaned, never passed to netsh as is.
        let named = tampered(&|v| v["areas"][0]["name"] = "Frank\"furt\\".into()).unwrap();
        assert_eq!(named[0].description, "efsp: blocks Frankfurt");

        let remove: HelperRequest = serde_json::from_str(r#"{"op":"removeAll"}"#).unwrap();
        assert_eq!(remove.rules().unwrap(), []);
    }
}
