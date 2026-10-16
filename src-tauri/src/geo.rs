//! Offline country and network-owner lookups backed by the DB-IP Lite
//! databases (CC BY 4.0, "IP Geolocation by DB-IP"), bundled with the app
//! under `ipdb/`. Nothing is looked up online.
//!
//! If a file is missing or unreadable, that half of the lookup returns
//! nothing: cloud ranges still classify servers, and the rest shows as
//! unclassified rather than guessed.

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use efsp_core::classify::{GeoLookup, NoGeo};
use maxminddb::{Reader, geoip2};

/// Shared, thread-safe country and network lookups.
pub type Geo = Arc<dyn GeoLookup + Send + Sync>;

pub struct DbIp {
    country: Option<Reader<Vec<u8>>>,
    asn: Option<Reader<Vec<u8>>>,
}

impl DbIp {
    fn open_one(path: &Path) -> Option<Reader<Vec<u8>>> {
        match Reader::open_readfile(path) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("efsp: can't open {}: {e}", path.display());
                None
            }
        }
    }
}

/// Opens the databases in `dir`; falls back to no lookups at all.
pub fn open(dir: &Path) -> Geo {
    let db = DbIp {
        country: DbIp::open_one(&dir.join("dbip-country-lite.mmdb")),
        asn: DbIp::open_one(&dir.join("dbip-asn-lite.mmdb")),
    };
    if db.country.is_none() && db.asn.is_none() {
        Arc::new(NoGeo)
    } else {
        Arc::new(db)
    }
}

impl GeoLookup for DbIp {
    fn country(&self, ip: IpAddr) -> Option<String> {
        let found = self.country.as_ref()?.lookup(ip).ok()?;
        let rec: geoip2::Country = found.decode().ok()??;
        rec.country.iso_code.map(str::to_owned)
    }

    fn network(&self, ip: IpAddr) -> Option<String> {
        let found = self.asn.as_ref()?.lookup(ip).ok()?;
        let rec: geoip2::Asn = found.decode().ok()??;
        rec.autonomous_system_organization.map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efsp_core::classify::{Endpoint, classify};

    fn dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/ipdb")
    }

    #[test]
    fn missing_databases_fall_back_to_no_lookups() {
        let geo = open(Path::new("/definitely/not/here"));
        assert_eq!(geo.country("8.8.8.8".parse().unwrap()), None);
    }

    /// Runs only when the databases have been downloaded
    /// (`node scripts/update-ip-data.mjs --geo`), as in CI. Uses live
    /// monthly data, so a reassigned block could make it drift.
    #[test]
    fn real_databases_answer_for_well_known_networks() {
        if !dir().join("dbip-asn-lite.mmdb").exists() {
            eprintln!("skipped: DB-IP databases not downloaded");
            return;
        }
        let geo = open(&dir());
        // Cloudflare's resolver: a hosting network, so never a peer.
        let one = "1.1.1.1".parse().unwrap();
        assert!(geo.network(one).is_some());
        assert!(matches!(
            classify(one, geo.as_ref()),
            Endpoint::Unknown {
                network: Some(_),
                ..
            }
        ));
        // Deutsche Telekom address space: residential, German.
        let dt = "80.128.0.1".parse().unwrap();
        assert_eq!(geo.country(dt).as_deref(), Some("DE"));
        assert_eq!(
            classify(dt, geo.as_ref()),
            Endpoint::Peer {
                country: Some("DE".into())
            }
        );
    }
}
