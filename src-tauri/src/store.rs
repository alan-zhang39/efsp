//! Match history in a local SQLite file. Nothing leaves the PC.
//!
//! The database holds numbers and where each match was (server city or the
//! peer's country), never an IP address or anything derived from one.
//! Schema changes go through `PRAGMA user_version`.

use std::path::Path;

use efsp_core::classify::{Endpoint, EndpointColumns};
use efsp_core::history::{
    Kind, MatchRecord, NewRecord, ServerRow, method_str, parse_method, parse_protocol, protocol_str,
};
use efsp_core::session::Point;
use rusqlite::{Connection, OptionalExtension, Row, params};

/// 4: the current layout.
/// 3: had a cloud `provider` column; upgraded by dropping it, keeping history.
/// 1, 2: test versions with fingerprints, notes and "seen before" counts;
/// replaced, not upgraded.
const SCHEMA_VERSION: i32 = 4;

const SCHEMA: &str = "CREATE TABLE matches (
    id            INTEGER PRIMARY KEY,
    started_at    INTEGER NOT NULL,
    duration_ms   INTEGER NOT NULL,
    kind          TEXT    NOT NULL,
    method        TEXT    NOT NULL,
    median_ms     REAL,
    p95_ms        REAL,
    jitter_ms     REAL,
    loss_pct      REAL,
    stalls        INTEGER NOT NULL,
    bars          INTEGER,
    rate_pps      REAL    NOT NULL,
    spark         TEXT    NOT NULL DEFAULT '[]',
    timeline      TEXT    NOT NULL,
    endpoint_kind TEXT,
    region        TEXT,
    country       TEXT,
    network       TEXT,
    protocol      TEXT    NOT NULL DEFAULT 'udp'
    );
    CREATE INDEX matches_started ON matches(started_at);";

pub struct Store {
    conn: Connection,
}

/// What the history list asks for.
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    /// Only records started at or after this Unix time.
    pub since: Option<i64>,
    pub include_offers: bool,
    pub limit: u32,
    pub offset: u32,
    /// Only `"server"` or `"peer"` records.
    pub endpoint_kind: Option<&'static str>,
    /// Only these regions, e.g. from a city search. An empty list matches
    /// nothing.
    pub regions: Option<Vec<String>>,
    pub sort: Sort,
    pub ascending: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    When,
    Ping,
    Length,
}

impl ListQuery {
    /// `WHERE …` and its parameters, shared by [`Store::list`] and
    /// [`Store::count`].
    fn filter(&self) -> (String, Vec<rusqlite::types::Value>) {
        use rusqlite::types::Value;
        let mut sql = String::from("started_at >= ? AND (? OR kind = 'match')");
        let mut args = vec![
            Value::Integer(self.since.unwrap_or(i64::MIN)),
            Value::Integer(i64::from(self.include_offers)),
        ];
        if let Some(kind) = self.endpoint_kind {
            sql.push_str(" AND endpoint_kind = ?");
            args.push(Value::Text(kind.to_owned()));
        }
        if let Some(regions) = &self.regions {
            if regions.is_empty() {
                sql.push_str(" AND 0");
            } else {
                let each = vec!["?"; regions.len()].join(", ");
                sql.push_str(&format!(
                    " AND endpoint_kind = 'server' AND region IN ({each})"
                ));
                for r in regions {
                    args.push(Value::Text(r.clone()));
                }
            }
        }
        (sql, args)
    }

    fn order(&self) -> String {
        // Fixed column names only; nothing from the caller reaches the SQL.
        let column = match self.sort {
            Sort::When => "started_at",
            Sort::Ping => "median_ms",
            Sort::Length => "duration_ms",
        };
        let dir = if self.ascending { "ASC" } else { "DESC" };
        // Records without a ping sort last either way.
        format!("{column} IS NULL, {column} {dir}, started_at DESC, id DESC")
    }
}

const COLUMNS: &str = "id, started_at, duration_ms, kind, method, median_ms, p95_ms, jitter_ms, \
                       loss_pct, stalls, bars, rate_pps, spark, \
                       endpoint_kind, region, country, network, protocol";
/// Index of the first column after [`COLUMNS`].
const AFTER_COLUMNS: usize = 18;

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// For tests: a throwaway database in memory.
    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        // The monitor thread and the history commands share one connection
        // behind a mutex; this only matters if another process holds the
        // file (e.g. a second copy of efsp).
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            // Written by a newer efsp: don't touch it, or we'd lose data.
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some(format!(
                    "history was saved by a newer version of efsp (schema {version})"
                )),
            ));
        }
        if version == 3 {
            // 3 → 4: same table minus the provider column; history is kept.
            conn.execute_batch(&format!(
                "BEGIN;
                 ALTER TABLE matches DROP COLUMN provider;
                 PRAGMA user_version = {SCHEMA_VERSION};
                 COMMIT;"
            ))?;
        } else if version < SCHEMA_VERSION {
            // A fresh file, or one from a pre-1.0 test build: start clean.
            conn.execute_batch(&format!(
                "BEGIN;
                 DROP TABLE IF EXISTS matches;
                 {SCHEMA}
                 PRAGMA user_version = {SCHEMA_VERSION};
                 COMMIT;"
            ))?;
        }
        Ok(Self { conn })
    }

    /// Saves a finished match; returns its id.
    pub fn insert(&self, r: &NewRecord) -> rusqlite::Result<i64> {
        let cols = r
            .endpoint
            .as_ref()
            .map(Endpoint::to_columns)
            .unwrap_or_default();
        self.conn.execute(
            "INSERT INTO matches (started_at, duration_ms, kind, method, median_ms, p95_ms,
                 jitter_ms, loss_pct, stalls, bars, rate_pps, timeline, spark, endpoint_kind,
                 region, country, network, protocol)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18)",
            params![
                r.started_at,
                r.duration_ms as i64,
                r.kind.as_str(),
                method_str(r.method),
                r.median_ms,
                r.p95_ms,
                r.jitter_ms,
                r.loss_pct,
                r.stalls,
                r.bars,
                r.rate_pps,
                r.timeline_json,
                r.spark_json,
                cols.kind,
                cols.region,
                cols.country,
                cols.network,
                protocol_str(r.protocol),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Newest first.
    pub fn list(&self, q: &ListQuery) -> rusqlite::Result<Vec<MatchRecord>> {
        let (filter, mut args) = q.filter();
        args.push(rusqlite::types::Value::Integer(i64::from(q.limit)));
        args.push(rusqlite::types::Value::Integer(i64::from(q.offset)));
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM matches WHERE {filter} ORDER BY {} LIMIT ? OFFSET ?",
            q.order()
        ))?;
        stmt.query_map(rusqlite::params_from_iter(args), row_to_record)?
            .collect()
    }

    /// How many records match the same filters as [`Store::list`].
    pub fn count(&self, q: &ListQuery) -> rusqlite::Result<u32> {
        let (filter, args) = q.filter();
        self.conn.query_row(
            &format!("SELECT COUNT(*) FROM matches WHERE {filter}"),
            rusqlite::params_from_iter(args),
            |r| r.get(0),
        )
    }

    /// Every saved record, matches and offers.
    pub fn count_all(&self) -> rusqlite::Result<u32> {
        self.conn
            .query_row("SELECT COUNT(*) FROM matches", [], |r| r.get(0))
    }

    /// How many records started before `before` (what a retention change
    /// would delete).
    pub fn count_before(&self, before: i64) -> rusqlite::Result<u32> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM matches WHERE started_at < ?1",
            [before],
            |r| r.get(0),
        )
    }

    /// One record with its whole timeline, for the detail view.
    pub fn detail(&self, id: i64) -> rusqlite::Result<Option<(MatchRecord, Vec<Point>)>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS}, timeline FROM matches WHERE id = ?1"),
                [id],
                |row| {
                    let record = row_to_record(row)?;
                    let json: String = row.get(AFTER_COLUMNS)?;
                    Ok((record, json))
                },
            )
            .optional()
            .map(|found| {
                found.map(|(record, json)| {
                    // A corrupt timeline shouldn't hide the match's numbers.
                    let points = serde_json::from_str(&json).unwrap_or_default();
                    (record, points)
                })
            })
    }

    /// Every saved record on a cloud server, for the region catalogue:
    /// (endpoint, kind, started_at, median_ms).
    pub fn server_rows(&self) -> rusqlite::Result<Vec<ServerRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT region, kind, started_at, median_ms FROM matches
             WHERE endpoint_kind = 'server'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<f64>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (region, kind, at, median) = row?;
            // Skip anything unreadable rather than failing the whole page.
            let endpoint = Endpoint::from_columns(EndpointColumns {
                kind: Some("server".into()),
                region,
                ..Default::default()
            });
            if let (Some(e), Some(k)) = (endpoint, Kind::parse(&kind)) {
                out.push((e, k, at, median));
            }
        }
        Ok(out)
    }

    pub fn delete_all(&self) -> rusqlite::Result<usize> {
        let n = self.conn.execute("DELETE FROM matches", [])?;
        // Actually remove the data from the file, not just mark it free.
        // VACUUM holds the shared lock for a moment, which can pause capture
        // briefly; acceptable for a rare, user-confirmed action.
        self.conn.execute_batch("VACUUM")?;
        Ok(n)
    }

    /// Deletes records that started before `before` (Unix seconds).
    pub fn prune(&self, before: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM matches WHERE started_at < ?1", [before])
    }
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<MatchRecord> {
    let kind: String = row.get(3)?;
    let method: String = row.get(4)?;
    let bad = |what: &str, value: String| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown {what}: {value}").into(),
        )
    };
    Ok(MatchRecord {
        id: row.get(0)?,
        started_at: row.get(1)?,
        duration_ms: row.get::<_, i64>(2)?.max(0) as u64,
        kind: Kind::parse(&kind).ok_or_else(|| bad("kind", kind.clone()))?,
        method: parse_method(&method).ok_or_else(|| bad("method", method.clone()))?,
        median_ms: row.get(5)?,
        p95_ms: row.get(6)?,
        jitter_ms: row.get(7)?,
        loss_pct: row.get(8)?,
        stalls: row.get(9)?,
        bars: row.get(10)?,
        rate_pps: row.get(11)?,
        // A corrupt sparkline just shows as empty.
        spark: serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_default(),
        endpoint: Endpoint::from_columns(EndpointColumns {
            kind: row.get(13)?,
            region: row.get(14)?,
            country: row.get(15)?,
            network: row.get(16)?,
        }),
        protocol: {
            let p: String = row.get(17)?;
            parse_protocol(&p).ok_or_else(|| bad("protocol", p.clone()))?
        },
    })
}

/// The history database, shared by the monitor thread (which saves
/// matches) and the history commands.
pub struct History {
    store: std::sync::Mutex<Store>,
}

impl History {
    /// Opens (or creates) `history.sqlite` in `data_dir`.
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        // Pre-1.0 builds kept a fingerprint salt here; it's no longer used.
        let _ = std::fs::remove_file(data_dir.join("salt.bin"));
        let store = Store::open(&data_dir.join("history.sqlite")).map_err(|e| e.to_string())?;
        Ok(Self {
            store: std::sync::Mutex::new(store),
        })
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self {
            store: std::sync::Mutex::new(Store::open_in_memory().unwrap()),
        }
    }

    /// Runs `f` with the store locked. Keep `f` short: the monitor thread
    /// shares this lock.
    pub fn with<T>(&self, f: impl FnOnce(&Store) -> T) -> T {
        let guard = self
            .store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efsp_core::cascade::Method;

    fn new_record(started_at: i64, duration_ms: u64, median: f64) -> NewRecord {
        NewRecord {
            started_at,
            duration_ms,
            kind: Kind::for_duration(duration_ms),
            method: Method::Icmp,
            median_ms: Some(median),
            p95_ms: Some(median + 5.0),
            jitter_ms: Some(3.0),
            loss_pct: Some(0.0),
            stalls: 0,
            bars: Some(4),
            rate_pps: 52.0,
            timeline_json:
                r#"[{"tMs":250,"rttMs":30.0,"jitterMs":2.0,"lost":0,"stalls":0,"ratePps":50.0}]"#
                    .into(),
            spark_json: "[30.0,null]".into(),
            endpoint: Some(Endpoint::Server {
                region: "europe-west3".into(),
                place: None,
            }),
            protocol: efsp_core::capture::Protocol::Udp,
        }
    }

    #[test]
    fn location_and_protocol_are_saved_and_the_city_looked_up_on_read() {
        let s = Store::open_in_memory().unwrap();
        let mut r = new_record(100, 600_000, 30.0);
        r.protocol = efsp_core::capture::Protocol::Tcp;
        let id = s.insert(&r).unwrap();
        let (rec, _) = s.detail(id).unwrap().unwrap();
        assert_eq!(rec.protocol, efsp_core::capture::Protocol::Tcp);
        let Some(Endpoint::Server { region, place, .. }) = rec.endpoint else {
            panic!("{:?}", rec.endpoint)
        };
        assert_eq!(region, "europe-west3");
        assert_eq!(place.unwrap().city, "Frankfurt");
    }

    #[test]
    fn server_rows_feed_the_region_catalogue() {
        let s = Store::open_in_memory().unwrap();
        s.insert(&new_record(100, 600_000, 30.0)).unwrap();
        let mut peer = new_record(200, 600_000, 50.0);
        peer.endpoint = Some(Endpoint::Peer {
            country: Some("DE".into()),
        });
        s.insert(&peer).unwrap();
        let rows = s.server_rows().unwrap();
        assert_eq!(rows.len(), 1);
        let stats = efsp_core::history::region_stats(&rows);
        assert_eq!(stats[0].matches, 1);
        assert_eq!(stats[0].median_ms, Some(30.0));
    }

    #[test]
    fn list_filters_by_kind_and_city_and_sorts() {
        let s = Store::open_in_memory().unwrap();
        // Frankfurt server matches at 40 and 20 ms, a peer at 30 ms.
        s.insert(&new_record(100, 600_000, 40.0)).unwrap();
        s.insert(&new_record(200, 900_000, 20.0)).unwrap();
        let mut peer = new_record(300, 700_000, 30.0);
        peer.endpoint = Some(Endpoint::Peer {
            country: Some("CA".into()),
        });
        s.insert(&peer).unwrap();
        let pings = |q: ListQuery| -> Vec<f64> {
            s.list(&ListQuery {
                include_offers: true,
                limit: 10,
                ..q
            })
            .unwrap()
            .iter()
            .filter_map(|r| r.median_ms)
            .collect()
        };
        assert_eq!(
            pings(ListQuery::default()),
            [30.0, 20.0, 40.0],
            "newest first"
        );
        assert_eq!(
            pings(ListQuery {
                sort: Sort::Ping,
                ascending: true,
                ..ListQuery::default()
            }),
            [20.0, 30.0, 40.0]
        );
        assert_eq!(
            pings(ListQuery {
                endpoint_kind: Some("peer"),
                ..ListQuery::default()
            }),
            [30.0]
        );
        let frankfurt = Some(vec!["europe-west3".to_owned()]);
        let q = ListQuery {
            include_offers: true,
            regions: frankfurt,
            ..ListQuery::default()
        };
        assert_eq!(s.count(&q).unwrap(), 2);
        let none = ListQuery {
            include_offers: true,
            regions: Some(Vec::new()),
            ..ListQuery::default()
        };
        assert_eq!(s.count(&none).unwrap(), 0, "a city search with no match");
    }

    #[test]
    fn reopening_an_upgraded_database_changes_nothing() {
        let dir = std::env::temp_dir().join(format!("efsp-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.sqlite");
        let _ = std::fs::remove_file(&path);
        Store::open(&path)
            .unwrap()
            .insert(&new_record(1, 600_000, 30.0))
            .unwrap();
        let again = Store::open(&path).unwrap();
        assert_eq!(again.count_all().unwrap(), 1);
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn insert_list_newest_first_with_offer_filter() {
        let s = Store::open_in_memory().unwrap();
        s.insert(&new_record(100, 600_000, 30.0)).unwrap();
        s.insert(&new_record(200, 9_000, 60.0)).unwrap(); // offer
        s.insert(&new_record(300, 700_000, 40.0)).unwrap();

        let q = |offers| ListQuery {
            since: None,
            include_offers: offers,
            limit: 50,
            offset: 0,
            ..ListQuery::default()
        };
        let all = s.list(&q(true)).unwrap();
        assert_eq!(
            all.iter().map(|r| r.started_at).collect::<Vec<_>>(),
            [300, 200, 100]
        );
        assert_eq!(all[1].kind, Kind::Offer);
        let matches = s.list(&q(false)).unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(
            (s.count(&q(true)).unwrap(), s.count(&q(false)).unwrap()),
            (3, 2)
        );

        let since = s
            .list(&ListQuery {
                since: Some(150),
                ..q(true)
            })
            .unwrap();
        assert_eq!(since.len(), 2);
        let page = s
            .list(&ListQuery {
                limit: 1,
                offset: 1,
                ..q(true)
            })
            .unwrap();
        assert_eq!(page[0].started_at, 200);
    }

    #[test]
    fn detail_prune_and_delete() {
        let s = Store::open_in_memory().unwrap();
        let id = s.insert(&new_record(100, 600_000, 30.0)).unwrap();
        s.insert(&new_record(200, 9_000, 99.0)).unwrap(); // an offer
        s.insert(&new_record(300, 600_000, 50.0)).unwrap();

        let (record, points) = s.detail(id).unwrap().unwrap();
        assert_eq!(record.median_ms, Some(30.0));
        assert_eq!(record.spark, [Some(30.0), None]);
        assert_eq!(points.len(), 1);
        assert!(s.detail(9999).unwrap().is_none());

        assert_eq!(s.prune(150).unwrap(), 1);
        assert_eq!(s.count_all().unwrap(), 2);
        assert_eq!(s.delete_all().unwrap(), 2);
        assert_eq!(s.count_all().unwrap(), 0);
    }

    #[test]
    fn reopening_keeps_data_and_schema_version() {
        let dir = std::env::temp_dir().join(format!("efsp-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.insert(&new_record(100, 600_000, 30.0)).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.count_all().unwrap(), 1);
        let v: i32 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        // A file from a newer efsp is refused, not overwritten.
        s.conn.execute_batch("PRAGMA user_version = 99").unwrap();
        drop(s);
        assert!(Store::open(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pre_1_0_database_is_replaced_with_the_clean_layout() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE matches (id INTEGER PRIMARY KEY, fingerprint TEXT, note TEXT);
             INSERT INTO matches (fingerprint, note) VALUES ('abc', 'old');
             PRAGMA user_version = 2;",
        )
        .unwrap();
        let s = Store::init(conn).unwrap();
        assert_eq!(s.count_all().unwrap(), 0, "old test data cleared");
        let cols: Vec<String> = s
            .conn
            .prepare("SELECT name FROM pragma_table_info('matches')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            !cols
                .iter()
                .any(|c| c == "fingerprint" || c == "note" || c == "seen_before")
        );
        s.insert(&new_record(1, 600_000, 30.0)).unwrap();
    }

    #[test]
    fn a_version_3_database_keeps_its_history_without_the_provider_column() {
        let conn = Connection::open_in_memory().unwrap();
        let v3 = SCHEMA.replace(
            "    endpoint_kind TEXT,\n",
            "    endpoint_kind TEXT,\n    provider      TEXT,\n",
        );
        conn.execute_batch(&format!("{v3}\nPRAGMA user_version = 3;"))
            .unwrap();
        conn.execute(
            "INSERT INTO matches (started_at, duration_ms, kind, method, stalls, rate_pps,
                 timeline, endpoint_kind, provider, region)
             VALUES (100, 600000, 'match', 'icmp', 0, 50.0, '[]', 'server', 'google',
                 'europe-west3')",
            [],
        )
        .unwrap();
        let s = Store::init(conn).unwrap();
        assert_eq!(s.count_all().unwrap(), 1, "history kept");
        let cols: Vec<String> = s
            .conn
            .prepare("SELECT name FROM pragma_table_info('matches')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!cols.iter().any(|c| c == "provider"));
        let rows = s.server_rows().unwrap();
        assert_eq!(rows[0].0.label(), "Frankfurt");
    }
}
