//! Firewall abstraction for the experimental server picker.
//!
//! Real rules are applied by an elevated helper. Everywhere else, including
//! every test, the app talks to a [`Firewall`] implementation; [`FakeFirewall`]
//! keeps rules in memory so CI never touches a real firewall.
//!
//! Safety is encoded in the types where possible: a [`RuleName`] can only be
//! built with the `efsp:` prefix and a safe character set, a [`Cidr`] can't be
//! wider than a /8, and a [`Rule`] always has a program field. [`validate`]
//! checks the rest, and every [`Firewall`] implementation must call it before
//! applying anything.

use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;

/// Every rule this app creates starts with this, so it can find and remove
/// its own rules (and only its own rules) after a crash.
pub const RULE_PREFIX: &str = "efsp:";

const MAX_LABEL_LEN: usize = 64;

/// Widest range a rule may block. Anything wider (like `0.0.0.0/0`) would also
/// block lobby/login servers, which F5 forbids.
pub const MIN_PREFIX_V4: u8 = 8;
pub const MIN_PREFIX_V6: u8 = 16;

/// A firewall rule name that is guaranteed to look like `"efsp: <label>"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuleName(String);

impl RuleName {
    /// Builds `"efsp: <label>"`. Labels are 1–64 characters of
    /// `A-Z a-z 0-9 space _ . -`, so names are always safe to hand to the
    /// elevated helper and Windows firewall tools.
    pub fn new(label: &str) -> Result<Self, FirewallError> {
        let label = label.trim();
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | '-');
        if label.is_empty() || label.len() > MAX_LABEL_LEN || !label.chars().all(allowed) {
            return Err(FirewallError::InvalidRuleName(label.to_owned()));
        }
        Ok(Self(format!("{RULE_PREFIX} {label}")))
    }

    /// Accepts an existing full name only if it is exactly what
    /// [`RuleName::new`] would produce. Used when reading rules back from the
    /// system firewall, to decide which rules are ours.
    pub fn parse(full: &str) -> Result<Self, FirewallError> {
        let invalid = || FirewallError::InvalidRuleName(full.to_owned());
        let label = full
            .strip_prefix(RULE_PREFIX)
            .and_then(|rest| rest.strip_prefix(' '))
            .ok_or_else(invalid)?;
        let rebuilt = Self::new(label).map_err(|_| invalid())?;
        if rebuilt.0 == full {
            Ok(rebuilt)
        } else {
            Err(invalid())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RuleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An address range in CIDR form, e.g. `34.65.0.0/16`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cidr {
    addr: IpAddr,
    prefix_len: u8,
}

impl Cidr {
    pub fn new(addr: IpAddr, prefix_len: u8) -> Result<Self, FirewallError> {
        let (min, max) = match addr {
            IpAddr::V4(_) => (MIN_PREFIX_V4, 32),
            IpAddr::V6(_) => (MIN_PREFIX_V6, 128),
        };
        if !(min..=max).contains(&prefix_len) {
            return Err(FirewallError::InvalidCidr { addr, prefix_len });
        }
        Ok(Self { addr, prefix_len })
    }

    /// Parses `"34.65.0.0/16"`; the same limits as [`Cidr::new`].
    pub fn parse(s: &str) -> Result<Self, FirewallError> {
        let bad = || FirewallError::InvalidText(s.to_owned());
        let (addr, len) = s.split_once('/').ok_or_else(bad)?;
        let addr: IpAddr = addr.parse().map_err(|_| bad())?;
        let len: u8 = len.parse().map_err(|_| bad())?;
        Self::new(addr, len)
    }

    pub fn addr(&self) -> IpAddr {
        self.addr
    }

    pub fn prefix_len(&self) -> u8 {
        self.prefix_len
    }
}

/// The one name every region-picker rule carries. Windows allows
/// several rules with the same name, and `netsh … delete rule name=…`
/// removes them all at once, so "Remove all" is one command that never
/// depends on reading Windows' (translated) rule listings.
pub const PICKER_RULE_LABEL: &str = "region picker";

/// The executable every rule must be scoped to.
pub const GAME_EXE: &str = "eFootball.exe";

/// Most ranges in one rule, keeping each netsh command line short.
pub const MAX_RANGES_PER_RULE: usize = 200;

/// An outbound UDP block rule scoped to one program (the eFootball
/// executable). UDP only: matches run over UDP, while login, lobby and
/// store traffic is HTTPS over TCP and must never be blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: RuleName,
    /// Shown in Windows Firewall: which cities this rule blocks.
    pub description: String,
    /// Full path of the executable the rule applies to. Never optional: a
    /// rule without a program would block traffic for the whole PC.
    pub program: PathBuf,
    /// Remote ranges to block for that program.
    pub remote: Vec<Cidr>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FirewallError {
    #[error("rule name must look like \"{RULE_PREFIX} <label>\" with a safe label: {0:?}")]
    InvalidRuleName(String),
    #[error("CIDR prefix /{prefix_len} for {addr} is invalid or too wide")]
    InvalidCidr { addr: IpAddr, prefix_len: u8 },
    #[error("rule {0} has no program path")]
    MissingProgram(RuleName),
    #[error("rule {0} has no remote ranges")]
    EmptyRule(RuleName),
    #[error("not a CIDR range: {0:?}")]
    InvalidText(String),
    #[error("rule {0} must be scoped to the full path of {GAME_EXE}")]
    BadProgram(RuleName),
    #[error("rule {0} has more than {MAX_RANGES_PER_RULE} ranges")]
    TooManyRanges(RuleName),
    #[error("rule {0} has an unsafe description")]
    BadDescription(RuleName),
}

/// Applies and removes this app's firewall rules.
///
/// Implementations must only ever list or remove rules whose name passes
/// [`RuleName::parse`]; rules created by anything else are never touched.
pub trait Firewall {
    /// All active rules owned by this app.
    fn list(&self) -> Result<Vec<Rule>, FirewallError>;
    /// Adds rules. All-or-nothing: on error, nothing is applied.
    fn apply(&mut self, rules: &[Rule]) -> Result<(), FirewallError>;
    /// Removes every rule owned by this app and returns how many were removed.
    fn remove_all(&mut self) -> Result<usize, FirewallError>;
}

/// Checks the invariants every implementation must enforce before applying.
/// The elevated helper runs this again on everything it receives.
pub fn validate(rule: &Rule) -> Result<(), FirewallError> {
    let name = || rule.name.clone();
    if rule.program.as_os_str().is_empty() {
        return Err(FirewallError::MissingProgram(name()));
    }
    let program = rule.program.to_string_lossy();
    // Windows-style absolute path (C:\…), to the game's executable, with
    // nothing that could break out of netsh's quoting.
    let absolute = program.len() > 3
        && program.as_bytes()[0].is_ascii_alphabetic()
        && program[1..].starts_with(":\\");
    let is_game = program
        .rsplit('\\')
        .next()
        .is_some_and(|file| file.eq_ignore_ascii_case(GAME_EXE));
    if !absolute || !is_game || program.contains(['"', '\n', '\r']) {
        return Err(FirewallError::BadProgram(name()));
    }
    if rule.remote.is_empty() {
        return Err(FirewallError::EmptyRule(name()));
    }
    if rule.remote.len() > MAX_RANGES_PER_RULE {
        return Err(FirewallError::TooManyRanges(name()));
    }
    if rule.description.len() > 200 || !rule.description.chars().all(description_char) {
        return Err(FirewallError::BadDescription(name()));
    }
    Ok(())
}

/// Characters allowed in a rule description: letters (any language),
/// digits, and ` ,.-'():`. Nothing that could end or escape netsh's quotes
/// (no `"` or `\`) and no control characters.
pub fn description_char(c: char) -> bool {
    c.is_alphanumeric() || " ,.-'():".contains(c)
}

/// Exact `netsh` arguments, one string each, passed verbatim (no shell, no
/// re-quoting) by the elevated helper. Pure so it can be tested anywhere.
pub mod netsh {
    use super::{Cidr, PICKER_RULE_LABEL, RULE_PREFIX, Rule};

    /// `"efsp: region picker"`, as netsh sees it.
    pub fn picker_name() -> String {
        format!("{RULE_PREFIX} {PICKER_RULE_LABEL}")
    }

    fn cidr(c: &Cidr) -> String {
        format!("{}/{}", c.addr(), c.prefix_len())
    }

    /// `netsh advfirewall firewall add rule …` for one (validated) rule.
    /// Direction, action and protocol are fixed here, never taken from input.
    pub fn add(rule: &Rule) -> Vec<String> {
        let remote: Vec<String> = rule.remote.iter().map(cidr).collect();
        vec![
            "advfirewall".into(),
            "firewall".into(),
            "add".into(),
            "rule".into(),
            format!("name=\"{}\"", rule.name),
            "dir=out".into(),
            "action=block".into(),
            "protocol=udp".into(),
            format!("program=\"{}\"", rule.program.to_string_lossy()),
            format!("remoteip={}", remote.join(",")),
            "profile=any".into(),
            "enable=yes".into(),
            format!("description=\"{}\"", rule.description),
        ]
    }

    /// Deletes every rule named `"efsp: region picker"` (all of ours).
    pub fn delete_all() -> Vec<String> {
        vec![
            "advfirewall".into(),
            "firewall".into(),
            "delete".into(),
            "rule".into(),
            format!("name=\"{}\"", picker_name()),
        ]
    }

    /// Exit code 0 if any of our rules exist. Needs no admin rights.
    pub fn show() -> Vec<String> {
        vec![
            "advfirewall".into(),
            "firewall".into(),
            "show".into(),
            "rule".into(),
            format!("name=\"{}\"", picker_name()),
        ]
    }
}

/// In-memory firewall for tests and for development on macOS.
///
/// Like the real system firewall, it keeps rules from every program in one
/// list; the trait methods only see and remove the ones that are ours.
#[derive(Debug, Default)]
pub struct FakeFirewall {
    /// `(name, rule)`. Rules from other software have no parsed [`Rule`].
    entries: Vec<(String, Option<Rule>)>,
}

impl FakeFirewall {
    /// Starts with rules created by other software (e.g. `"Steam: allow"`).
    pub fn with_foreign_rules(names: &[&str]) -> Self {
        Self {
            entries: names.iter().map(|n| ((*n).to_owned(), None)).collect(),
        }
    }

    /// Every rule name on the "system", ours and foreign.
    pub fn all_names(&self) -> Vec<&str> {
        self.entries.iter().map(|(name, _)| name.as_str()).collect()
    }

    fn is_ours(name: &str) -> bool {
        RuleName::parse(name).is_ok()
    }
}

impl Firewall for FakeFirewall {
    fn list(&self) -> Result<Vec<Rule>, FirewallError> {
        Ok(self
            .entries
            .iter()
            .filter(|(name, _)| Self::is_ours(name))
            .filter_map(|(_, rule)| rule.clone())
            .collect())
    }

    fn apply(&mut self, rules: &[Rule]) -> Result<(), FirewallError> {
        // Validate everything first so a bad rule leaves no partial state.
        // Rules may share a name (see [`PICKER_RULE_LABEL`]).
        for rule in rules {
            validate(rule)?;
        }
        self.entries.extend(
            rules
                .iter()
                .map(|r| (r.name.as_str().to_owned(), Some(r.clone()))),
        );
        Ok(())
    }

    fn remove_all(&mut self) -> Result<usize, FirewallError> {
        let before = self.entries.len();
        self.entries.retain(|(name, _)| !Self::is_ours(name));
        Ok(before - self.entries.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn cidr(a: u8, b: u8, len: u8) -> Cidr {
        Cidr::new(IpAddr::V4(Ipv4Addr::new(a, b, 0, 0)), len).unwrap()
    }

    fn rule(label: &str) -> Rule {
        Rule {
            name: RuleName::new(label).unwrap(),
            description: "Frankfurt".into(),
            program: PathBuf::from(r"C:\Games\eFootball\eFootball.exe"),
            remote: vec![cidr(34, 65, 16)],
        }
    }

    #[test]
    fn rule_names_always_carry_the_prefix() {
        assert_eq!(
            RuleName::new("block london").unwrap().as_str(),
            "efsp: block london"
        );
        assert!(RuleName::new("   ").is_err());
    }

    #[test]
    fn rule_names_reject_unsafe_characters_and_long_labels() {
        for bad in ["a\nb", "quote\"", "semi;colon", "tab\there", "é"] {
            assert!(RuleName::new(bad).is_err(), "{bad:?}");
        }
        assert!(RuleName::new(&"x".repeat(64)).is_ok());
        assert!(RuleName::new(&"x".repeat(65)).is_err());
    }

    #[test]
    fn parse_only_accepts_names_we_would_create() {
        assert!(RuleName::parse("efsp: block london").is_ok());
        assert!(RuleName::parse("Steam: allow").is_err());
        assert!(RuleName::parse("efsp:").is_err());
        assert!(RuleName::parse("efsp:x").is_err());
        assert!(RuleName::parse("efsp: bad;name").is_err());
        assert!(RuleName::parse("efsp:  padded").is_err());
        assert!(
            RuleName::parse("EFSP: x").is_err(),
            "prefix match is case-sensitive"
        );
    }

    #[test]
    fn cidr_rejects_impossible_or_too_wide_prefixes() {
        let v4 = IpAddr::V4(Ipv4Addr::new(34, 0, 0, 0));
        assert!(Cidr::new(v4, 33).is_err());
        assert!(Cidr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0).is_err());
        assert!(Cidr::new(v4, MIN_PREFIX_V4 - 1).is_err());
        assert!(Cidr::new(v4, MIN_PREFIX_V4).is_ok());

        let v6: IpAddr = "2600:1900::".parse().unwrap();
        assert!(Cidr::new(v6, 0).is_err());
        assert!(Cidr::new(v6, MIN_PREFIX_V6 - 1).is_err());
        assert!(Cidr::new(v6, 128).is_ok());
        assert!(Cidr::new(v6, 129).is_err());
    }

    #[test]
    fn apply_list_remove_roundtrip() {
        let mut fw = FakeFirewall::default();
        fw.apply(&[rule("a"), rule("b")]).unwrap();
        assert_eq!(fw.list().unwrap().len(), 2);
        assert_eq!(fw.remove_all().unwrap(), 2);
        assert!(fw.list().unwrap().is_empty());
    }

    #[test]
    fn remove_all_leaves_other_software_alone() {
        let foreign = ["Steam: allow", "efsp-lookalike", "efsp:x"];
        let mut fw = FakeFirewall::with_foreign_rules(&foreign);
        fw.apply(&[rule("a")]).unwrap();
        assert_eq!(fw.list().unwrap().len(), 1, "foreign rules are not listed");
        assert_eq!(fw.remove_all().unwrap(), 1);
        assert_eq!(fw.all_names(), foreign);
    }

    #[test]
    fn apply_is_all_or_nothing() {
        let mut fw = FakeFirewall::default();
        let mut bad = rule("bad");
        bad.program = PathBuf::new();
        assert_eq!(
            fw.apply(&[rule("good"), bad.clone()]),
            Err(FirewallError::MissingProgram(bad.name))
        );
        assert!(fw.list().unwrap().is_empty());
    }

    #[test]
    fn apply_rejects_empty_rules_but_allows_shared_names() {
        let mut fw = FakeFirewall::default();
        let mut empty = rule("empty");
        empty.remote.clear();
        assert!(matches!(
            fw.apply(&[empty]),
            Err(FirewallError::EmptyRule(_))
        ));
        fw.apply(&[rule("x"), rule("x")]).unwrap();
        assert_eq!(fw.remove_all().unwrap(), 2, "one name removes them all");
    }

    #[test]
    fn rules_must_target_the_game_by_full_path() {
        for bad in [
            r"eFootball.exe",
            r"C:\Games\other.exe",
            r"C:\Games\eFootball.exe\..\cmd.exe",
            "C:\\Games\\eFoot\"ball.exe",
            r"\\server\share\eFootball.exe",
        ] {
            let mut r = rule("x");
            r.program = PathBuf::from(bad);
            assert!(
                matches!(validate(&r), Err(FirewallError::BadProgram(_))),
                "{bad}"
            );
        }
        let mut r = rule("x");
        r.program =
            PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\eFootball\EFOOTBALL.EXE");
        assert_eq!(validate(&r), Ok(()));
    }

    #[test]
    fn rules_cap_ranges_and_reject_unsafe_descriptions() {
        let mut r = rule("x");
        r.remote = vec![cidr(34, 65, 16); MAX_RANGES_PER_RULE + 1];
        assert!(matches!(validate(&r), Err(FirewallError::TooManyRanges(_))));
        let mut r = rule("x");
        for bad in ["São Paulo\" & more", "ends with \\", "tab\there"] {
            r.description = bad.into();
            assert!(
                matches!(validate(&r), Err(FirewallError::BadDescription(_))),
                "{bad:?}"
            );
        }
        r.description = "efsp: blocks São Paulo (AWS), Montréal".into();
        assert_eq!(validate(&r), Ok(()));
    }

    #[test]
    fn netsh_arguments_are_exact_and_fixed_to_outbound_udp_blocks() {
        let mut r = rule(PICKER_RULE_LABEL);
        r.program =
            PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\eFootball\eFootball.exe");
        r.remote = vec![
            cidr(34, 65, 16),
            Cidr::new("2600:1900::".parse().unwrap(), 28).unwrap(),
        ];
        r.description = "efsp: São Paulo, Frankfurt".into();
        assert_eq!(
            netsh::add(&r),
            [
                "advfirewall",
                "firewall",
                "add",
                "rule",
                "name=\"efsp: region picker\"",
                "dir=out",
                "action=block",
                "protocol=udp",
                r#"program="C:\Program Files (x86)\Steam\steamapps\common\eFootball\eFootball.exe""#,
                "remoteip=34.65.0.0/16,2600:1900::/28",
                "profile=any",
                "enable=yes",
                "description=\"efsp: São Paulo, Frankfurt\"",
            ]
        );
        assert_eq!(
            netsh::delete_all(),
            [
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                "name=\"efsp: region picker\""
            ]
        );
    }
}
