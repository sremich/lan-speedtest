//! Stored results.
//!
//! Every completed run is POSTed back and kept, so this week can be compared
//! with last week and one client with another — the thing that vanished when a
//! LibreSpeed tab was closed.
//!
//! Two design choices are worth stating. The full engine summary is stored
//! verbatim as JSON alongside the extracted columns, so a metric we did not
//! think to give a column to is not lost. And writes go through a single
//! mutex-guarded connection: this is a LAN tool recording one row per test run,
//! where a connection pool would be more moving parts than the workload
//! deserves.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Ignore absurd payloads outright rather than storing them.
pub const MAX_SUMMARY_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum HistoryError {
    Db(rusqlite::Error),
    Json(serde_json::Error),
    TooLarge { bytes: usize, limit: usize },
    NothingMeasured,
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db(e) => write!(f, "database error: {e}"),
            Self::Json(e) => write!(f, "malformed result payload: {e}"),
            Self::TooLarge { bytes, limit } => {
                write!(f, "result payload is {bytes} bytes, limit is {limit}")
            }
            Self::NothingMeasured => write!(
                f,
                "the result contains no measurements at all — refusing to store an empty run"
            ),
        }
    }
}

impl std::error::Error for HistoryError {}

impl From<rusqlite::Error> for HistoryError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e)
    }
}
impl From<serde_json::Error> for HistoryError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// What the front end POSTs when a run finishes.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultSubmission {
    /// The engine's `getSummary()`, passed through unchanged.
    pub summary: SubmittedSummary,
    /// `getScores()`, flattened to name -> classification.
    #[serde(default)]
    pub scores: std::collections::BTreeMap<String, String>,
    /// Which measurement profile produced this.
    #[serde(default)]
    pub profile: String,
}

/// The subset of the engine summary we give columns to.
///
/// Everything the engine reports is also stored verbatim, so nothing here is
/// load-bearing for data retention — these exist to make queries and charts
/// straightforward.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmittedSummary {
    pub download: Option<f64>,
    pub upload: Option<f64>,
    pub latency: Option<f64>,
    pub jitter: Option<f64>,
    pub down_loaded_latency: Option<f64>,
    pub up_loaded_latency: Option<f64>,
    pub packet_loss: Option<f64>,
    pub total_duration_ms: Option<f64>,
}

impl SubmittedSummary {
    /// A run with nothing in it is a bug or a probe, not a measurement.
    fn is_empty(&self) -> bool {
        self.download.is_none()
            && self.upload.is_none()
            && self.latency.is_none()
            && self.jitter.is_none()
    }
}

/// One stored run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredRun {
    pub id: i64,
    /// RFC 3339, UTC.
    pub recorded_at: String,
    pub client_ip: String,
    pub client_name: Option<String>,
    pub user_agent: String,
    pub profile: String,
    pub download: Option<f64>,
    pub upload: Option<f64>,
    pub latency: Option<f64>,
    pub jitter: Option<f64>,
    pub down_loaded_latency: Option<f64>,
    pub up_loaded_latency: Option<f64>,
    pub packet_loss: Option<f64>,
    pub total_duration_ms: Option<f64>,
    pub scores: std::collections::BTreeMap<String, String>,
}

pub struct History {
    conn: Mutex<Connection>,
}

impl History {
    pub fn open(path: &Path) -> Result<Self, HistoryError> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(dir);
            }
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    pub fn in_memory() -> Result<Self, HistoryError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, HistoryError> {
        // WAL keeps a reader on /history from blocking the write that lands at
        // the end of a run.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS runs (
                 id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                 recorded_at         TEXT    NOT NULL,
                 client_ip           TEXT    NOT NULL,
                 user_agent          TEXT    NOT NULL DEFAULT '',
                 profile             TEXT    NOT NULL DEFAULT '',
                 download            REAL,
                 upload              REAL,
                 latency             REAL,
                 jitter              REAL,
                 down_loaded_latency REAL,
                 up_loaded_latency   REAL,
                 packet_loss         REAL,
                 total_duration_ms   REAL,
                 scores_json         TEXT    NOT NULL DEFAULT '{}',
                 -- The engine summary verbatim, so a column we did not think
                 -- to add does not mean the data is gone.
                 summary_json        TEXT    NOT NULL DEFAULT '{}'
             );
             CREATE INDEX IF NOT EXISTS runs_recorded_at ON runs (recorded_at DESC);
             CREATE INDEX IF NOT EXISTS runs_client_ip   ON runs (client_ip);

             -- Friendly names for clients, keyed by address. Populated by hand;
             -- absent entries simply fall back to the address.
             CREATE TABLE IF NOT EXISTS client_names (
                 client_ip TEXT PRIMARY KEY,
                 name      TEXT NOT NULL
             );",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Stores a completed run and returns its id.
    pub fn record(
        &self,
        submission: &ResultSubmission,
        client_ip: &str,
        user_agent: &str,
        recorded_at: &str,
    ) -> Result<i64, HistoryError> {
        if submission.summary.is_empty() {
            return Err(HistoryError::NothingMeasured);
        }

        let summary_json = serde_json::to_string(&submission.summary)?;
        let scores_json = serde_json::to_string(&submission.scores)?;

        let conn = self.conn.lock().expect("history mutex");
        conn.execute(
            "INSERT INTO runs (
                 recorded_at, client_ip, user_agent, profile,
                 download, upload, latency, jitter,
                 down_loaded_latency, up_loaded_latency, packet_loss,
                 total_duration_ms, scores_json, summary_json
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                recorded_at,
                client_ip,
                user_agent,
                submission.profile,
                submission.summary.download,
                submission.summary.upload,
                submission.summary.latency,
                submission.summary.jitter,
                submission.summary.down_loaded_latency,
                submission.summary.up_loaded_latency,
                submission.summary.packet_loss,
                submission.summary.total_duration_ms,
                scores_json,
                summary_json,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Most recent runs first.
    pub fn recent(
        &self,
        limit: u32,
        client_ip: Option<&str>,
    ) -> Result<Vec<StoredRun>, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        let limit = limit.clamp(1, 1000);

        let sql = "SELECT r.id, r.recorded_at, r.client_ip, c.name, r.user_agent, r.profile,
                          r.download, r.upload, r.latency, r.jitter,
                          r.down_loaded_latency, r.up_loaded_latency, r.packet_loss,
                          r.total_duration_ms, r.scores_json
                   FROM runs r
                   LEFT JOIN client_names c ON c.client_ip = r.client_ip
                   WHERE (?1 IS NULL OR r.client_ip = ?1)
                   ORDER BY r.recorded_at DESC, r.id DESC
                   LIMIT ?2";

        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![client_ip, limit], |row| {
            let scores_json: String = row.get(14)?;
            Ok(StoredRun {
                id: row.get(0)?,
                recorded_at: row.get(1)?,
                client_ip: row.get(2)?,
                client_name: row.get(3)?,
                user_agent: row.get(4)?,
                profile: row.get(5)?,
                download: row.get(6)?,
                upload: row.get(7)?,
                latency: row.get(8)?,
                jitter: row.get(9)?,
                down_loaded_latency: row.get(10)?,
                up_loaded_latency: row.get(11)?,
                packet_loss: row.get(12)?,
                total_duration_ms: row.get(13)?,
                scores: serde_json::from_str(&scores_json).unwrap_or_default(),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Distinct clients seen, most recently active first.
    pub fn clients(&self) -> Result<Vec<ClientSummary>, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        let mut stmt = conn.prepare(
            "SELECT r.client_ip, c.name, COUNT(*), MAX(r.recorded_at)
             FROM runs r
             LEFT JOIN client_names c ON c.client_ip = r.client_ip
             GROUP BY r.client_ip
             ORDER BY MAX(r.recorded_at) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ClientSummary {
                client_ip: row.get(0)?,
                client_name: row.get(1)?,
                runs: row.get(2)?,
                last_seen: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn count(&self) -> Result<i64, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        Ok(conn.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))?)
    }

    /// Names a client. Used by the `[later]` friendly-naming feature; the table
    /// exists now so history does not need a migration to gain it.
    pub fn set_client_name(&self, client_ip: &str, name: &str) -> Result<(), HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        conn.execute(
            "INSERT INTO client_names (client_ip, name) VALUES (?1, ?2)
             ON CONFLICT(client_ip) DO UPDATE SET name = excluded.name",
            params![client_ip, name],
        )?;
        Ok(())
    }

    pub fn client_name(&self, client_ip: &str) -> Result<Option<String>, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        Ok(conn
            .query_row(
                "SELECT name FROM client_names WHERE client_ip = ?1",
                params![client_ip],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSummary {
    pub client_ip: String,
    pub client_name: Option<String>,
    pub runs: i64,
    pub last_seen: String,
}

/// Current time as RFC 3339 in UTC.
pub fn now_rfc3339() -> String {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submission(download: f64) -> ResultSubmission {
        ResultSubmission {
            summary: SubmittedSummary {
                download: Some(download),
                upload: Some(5.0e8),
                latency: Some(0.6),
                jitter: Some(0.1),
                down_loaded_latency: Some(1.1),
                up_loaded_latency: Some(0.9),
                packet_loss: Some(0.0),
                total_duration_ms: Some(6100.0),
            },
            scores: [
                ("streaming".to_string(), "great".to_string()),
                ("gaming".to_string(), "great".to_string()),
            ]
            .into_iter()
            .collect(),
            profile: "lan-1g".into(),
        }
    }

    fn history() -> History {
        History::in_memory().unwrap()
    }

    #[test]
    fn a_run_round_trips_with_its_client_and_scores() {
        let h = history();
        h.record(
            &submission(9.4e8),
            "10.0.0.11",
            "Mozilla/5.0 Chrome",
            "2026-08-24T10:00:00Z",
        )
        .unwrap();

        let runs = h.recent(10, None).unwrap();
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.client_ip, "10.0.0.11");
        assert_eq!(r.profile, "lan-1g");
        assert_eq!(r.download, Some(9.4e8));
        assert_eq!(r.packet_loss, Some(0.0));
        assert_eq!(r.scores.get("streaming").map(String::as_str), Some("great"));
    }

    #[test]
    fn three_runs_from_two_clients_are_attributed_correctly() {
        // The milestone's stated done-when, asserted directly.
        let h = history();
        h.record(
            &submission(1.0e9),
            "10.0.0.11",
            "Chrome",
            "2026-08-24T10:00:00Z",
        )
        .unwrap();
        h.record(
            &submission(2.0e9),
            "10.0.0.12",
            "Firefox",
            "2026-08-24T10:05:00Z",
        )
        .unwrap();
        h.record(
            &submission(3.0e9),
            "10.0.0.11",
            "Chrome",
            "2026-08-24T10:10:00Z",
        )
        .unwrap();

        let all = h.recent(50, None).unwrap();
        assert_eq!(all.len(), 3);
        // Newest first.
        assert_eq!(all[0].download, Some(3.0e9));
        assert_eq!(all[2].download, Some(1.0e9));

        let first_client = h.recent(50, Some("10.0.0.11")).unwrap();
        assert_eq!(first_client.len(), 2);
        assert!(first_client.iter().all(|r| r.client_ip == "10.0.0.11"));

        let second_client = h.recent(50, Some("10.0.0.12")).unwrap();
        assert_eq!(second_client.len(), 1);
        assert_eq!(second_client[0].user_agent, "Firefox");

        let clients = h.clients().unwrap();
        assert_eq!(clients.len(), 2);
        // Most recently active first.
        assert_eq!(clients[0].client_ip, "10.0.0.11");
        assert_eq!(clients[0].runs, 2);
        assert_eq!(clients[1].runs, 1);
    }

    #[test]
    fn an_empty_result_is_refused_rather_than_stored() {
        let h = history();
        let empty = ResultSubmission {
            summary: SubmittedSummary::default(),
            scores: Default::default(),
            profile: "quick".into(),
        };
        assert!(matches!(
            h.record(&empty, "10.0.0.1", "ua", "2026-08-24T10:00:00Z"),
            Err(HistoryError::NothingMeasured)
        ));
        assert_eq!(h.count().unwrap(), 0);
    }

    #[test]
    fn a_partial_result_is_still_worth_keeping() {
        // A run that measured download but nothing else is a real observation.
        let h = history();
        let partial = ResultSubmission {
            summary: SubmittedSummary {
                download: Some(1.0e8),
                ..Default::default()
            },
            scores: Default::default(),
            profile: "quick".into(),
        };
        h.record(&partial, "10.0.0.1", "ua", "2026-08-24T10:00:00Z")
            .unwrap();
        assert_eq!(h.count().unwrap(), 1);
        assert_eq!(h.recent(1, None).unwrap()[0].upload, None);
    }

    #[test]
    fn friendly_client_names_replace_the_address_when_set() {
        let h = history();
        h.record(
            &submission(1.0e9),
            "10.0.0.11",
            "Chrome",
            "2026-08-24T10:00:00Z",
        )
        .unwrap();
        assert_eq!(h.recent(1, None).unwrap()[0].client_name, None);

        h.set_client_name("10.0.0.11", "workshop-desktop").unwrap();
        assert_eq!(
            h.recent(1, None).unwrap()[0].client_name.as_deref(),
            Some("workshop-desktop")
        );

        // And renaming replaces rather than duplicating.
        h.set_client_name("10.0.0.11", "bench-pc").unwrap();
        assert_eq!(
            h.client_name("10.0.0.11").unwrap().as_deref(),
            Some("bench-pc")
        );
    }

    #[test]
    fn the_limit_is_clamped_to_something_sane() {
        let h = history();
        for i in 0..5 {
            h.record(
                &submission(1.0e9),
                "10.0.0.1",
                "ua",
                &format!("2026-08-24T10:0{i}:00Z"),
            )
            .unwrap();
        }
        assert_eq!(h.recent(2, None).unwrap().len(), 2);
        // Zero would otherwise return nothing at all, which reads as "no data".
        assert_eq!(h.recent(0, None).unwrap().len(), 1);
        assert_eq!(h.recent(u32::MAX, None).unwrap().len(), 5);
    }

    #[test]
    fn opening_an_existing_database_keeps_its_rows() {
        let dir = std::env::temp_dir().join(format!("speedtest-hist-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.db");
        let _ = std::fs::remove_file(&path);

        {
            let h = History::open(&path).unwrap();
            h.record(&submission(1.0e9), "10.0.0.1", "ua", "2026-08-24T10:00:00Z")
                .unwrap();
        }
        {
            // Re-opening must not wipe or re-create the table.
            let h = History::open(&path).unwrap();
            assert_eq!(h.count().unwrap(), 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        let now = now_rfc3339();
        assert!(now.ends_with('Z'), "{now}");
        assert!(now.len() >= 20, "{now}");
        assert!(
            time::OffsetDateTime::parse(&now, &time::format_description::well_known::Rfc3339)
                .is_ok()
        );
    }
}
