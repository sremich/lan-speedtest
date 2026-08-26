//! Stored results.
//!
//! Every completed run is POSTed back and kept, so this week can be compared
//! with last week and one client with another — the thing that vanished when a
//! LibreSpeed tab was closed.
//!
//! Three design choices are worth stating. The full engine summary is stored
//! verbatim as JSON alongside the extracted columns, so a metric we did not
//! think to give a column to is not lost. Writes go through a single
//! mutex-guarded connection: this is a LAN tool recording one row per test run,
//! where a connection pool would be more moving parts than the workload
//! deserves. And the database looks after its own disk: deleting rows in SQLite
//! returns their pages to a free list rather than to the filesystem, so a
//! deployment that prunes daily would otherwise grow for ever while reporting
//! fewer and fewer runs.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Ignore absurd payloads outright rather than storing them.
///
/// Generous since 1.3.0, because a submission now carries every sample rather
/// than just the summary — a `lan-10g` run is a few tens of kilobytes.
pub const MAX_SUMMARY_BYTES: usize = 512 * 1024;

/// Samples larger than this are dropped and the run kept without them. The
/// measurement succeeded; losing it over the size of its detail would not be
/// an improvement.
const MAX_POINTS_BYTES: usize = 256 * 1024;

/// The name a snapshot lands under, inside the configured directory.
///
/// Fixed rather than timestamped, because this is a last-known-good copy and
/// not an archive. A dated filename would leave a directory that nothing ever
/// prunes, which is the problem this module already has to solve once.
pub const SNAPSHOT_FILE: &str = "history-backup.db";

/// Where a snapshot is written before it is moved into place. A crash halfway
/// through leaves rubbish here and the previous good snapshot untouched.
const SNAPSHOT_TEMP_FILE: &str = "history-backup.db.tmp";

/// `PRAGMA auto_vacuum` reports its mode as a number; 2 is `INCREMENTAL`.
const AUTO_VACUUM_INCREMENTAL: i64 = 2;

#[derive(Debug)]
pub enum HistoryError {
    Db(rusqlite::Error),
    Json(serde_json::Error),
    TooLarge {
        bytes: usize,
        limit: usize,
    },
    NothingMeasured,
    /// A snapshot could not be written where it was asked to go.
    Snapshot {
        path: PathBuf,
        detail: String,
    },
    /// The configured snapshot destination *is* the live database.
    SnapshotOverwritesDatabase {
        path: PathBuf,
    },
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
            Self::Snapshot { path, detail } => {
                write!(
                    f,
                    "could not write a history snapshot to {path:?}: {detail}"
                )
            }
            Self::SnapshotOverwritesDatabase { path } => write!(
                f,
                "a history snapshot would be written over the live database at {path:?} — \
                 point server.history_backup_dir at a different directory"
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
    /// Every sample the engine collected, passed through untyped.
    ///
    /// Stored verbatim for the same reason the summary is: giving it a schema
    /// here would mean tracking the engine's, and a permalink that can only
    /// show the fields we thought of is a worse permalink. Only the front end
    /// interprets this.
    #[serde(default)]
    pub points: serde_json::Value,
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

/// What a prune actually removed. Reported rather than assumed, so the log
/// says what happened instead of what was requested.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pruned {
    pub runs_deleted: usize,
    pub samples_cleared: usize,
    /// How much smaller the database file got, measured before and after
    /// rather than inferred from the row count. Deleting a row and returning
    /// its disk are separate events in SQLite, and only the second one is
    /// visible to `df`.
    pub bytes_reclaimed: u64,
}

/// One stored run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredRun {
    pub id: i64,
    /// RFC 3339, UTC.
    pub recorded_at: String,
    pub client_ip: String,
    /// A name someone chose. Beats anything resolved.
    pub client_name: Option<String>,
    /// A name from reverse DNS, if one was found.
    pub hostname: Option<String>,
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
    /// A note written by hand after the fact. `None` when never set.
    pub note: Option<String>,
    /// The build that measured this run. `None` for runs stored before the
    /// version was recorded, whose latency may predate the `TCP_NODELAY` fix.
    pub app_version: Option<String>,
}

/// A single run with everything kept about it, for a permalink.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredRunDetail {
    #[serde(flatten)]
    pub run: StoredRun,
    /// The engine summary as submitted.
    pub summary: serde_json::Value,
    /// Every sample, as submitted. `{}` for runs stored before 1.3.0.
    pub points: serde_json::Value,
}

pub struct History {
    conn: Mutex<Connection>,
    /// Where this database lives, when it lives anywhere at all. `None` for the
    /// in-memory databases the tests and the history-disabled deployment use.
    /// Kept so a snapshot can be checked against the file it is a copy of.
    path: Option<PathBuf>,
}

impl History {
    pub fn open(path: &Path) -> Result<Self, HistoryError> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(dir);
            }
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn, Some(path.to_path_buf()))
    }

    pub fn in_memory() -> Result<Self, HistoryError> {
        Self::from_connection(Connection::open_in_memory()?, None)
    }

    fn from_connection(conn: Connection, path: Option<PathBuf>) -> Result<Self, HistoryError> {
        // WAL keeps a reader on /history from blocking the write that lands at
        // the end of a run.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        // Before the schema below, because on a new database that is the only
        // moment this setting can be taken.
        Self::enable_incremental_vacuum(&conn)?;

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
             -- Both columns, in the order `recent` sorts by. With only
             -- `recorded_at` indexed, SQLite can satisfy the first key from the
             -- index and then has to sort the ties by id in memory.
             CREATE INDEX IF NOT EXISTS runs_recorded_at_id
                 ON runs (recorded_at DESC, id DESC);
             CREATE INDEX IF NOT EXISTS runs_client_ip ON runs (client_ip);

             -- Friendly names for clients, keyed by address. Populated by hand;
             -- absent entries simply fall back to the address.
             CREATE TABLE IF NOT EXISTS client_names (
                 client_ip TEXT PRIMARY KEY,
                 name      TEXT NOT NULL
             );

             -- Names found by reverse lookup, kept apart from the ones people
             -- chose so a PTR record can never overwrite a deliberate name.
             -- An empty name is a remembered miss, which stops a client with
             -- no PTR record being looked up on every single run.
             CREATE TABLE IF NOT EXISTS resolved_names (
                 client_ip   TEXT PRIMARY KEY,
                 name        TEXT NOT NULL,
                 resolved_at TEXT NOT NULL
             );",
        )?;

        Self::add_missing_columns(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    /// Puts the database into `auto_vacuum = INCREMENTAL`, converting one that
    /// is not already there.
    ///
    /// SQLite reads this setting once, when the first table is created, which
    /// is why it is applied above the schema rather than beside the other
    /// pragmas. On a database that already has tables the assignment is
    /// **silently ignored** — no error, no warning — and a later
    /// `PRAGMA incremental_vacuum` then frees nothing while looking like it
    /// worked. The only way to change the mode afterwards is a full `VACUUM`.
    ///
    /// So the pragma is read back rather than assumed, and the `VACUUM` happens
    /// only when the mode is genuinely wrong. It rewrites the whole file, which
    /// for a homelab's history is a few megabytes and a fraction of a second,
    /// and it happens at most once per database: the mode is stored in the file
    /// header, so every start after the first reads `INCREMENTAL` and does
    /// nothing.
    fn enable_incremental_vacuum(conn: &Connection) -> Result<(), HistoryError> {
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        if Self::auto_vacuum_mode(conn)? == AUTO_VACUUM_INCREMENTAL {
            return Ok(());
        }

        // An existing database, then. The assignment above is now pending and
        // a rewrite is what applies it.
        conn.execute_batch("VACUUM")?;

        let mode = Self::auto_vacuum_mode(conn)?;
        if mode != AUTO_VACUUM_INCREMENTAL {
            // Not fatal — the history still works, it just will not hand disk
            // back after a prune. Worth saying out loud, because the symptom
            // otherwise is a file that only ever grows.
            tracing::warn!(
                "auto_vacuum is still mode {mode} after a full VACUUM; pruning will free \
                 rows but not disk"
            );
        }
        Ok(())
    }

    fn auto_vacuum_mode(conn: &Connection) -> Result<i64, HistoryError> {
        Ok(conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?)
    }

    /// Brings an existing database up to the current shape.
    ///
    /// `CREATE TABLE IF NOT EXISTS` does nothing for a table that already
    /// exists, so a column added after a deployment has to be added
    /// explicitly. Checked rather than attempted-and-ignored: swallowing the
    /// error would also swallow a real one.
    fn add_missing_columns(conn: &Connection) -> Result<(), HistoryError> {
        let mut stmt = conn.prepare("PRAGMA table_info(runs)")?;
        let existing: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<_, _>>()?;

        if !existing.iter().any(|c| c == "points_json") {
            conn.execute(
                "ALTER TABLE runs ADD COLUMN points_json TEXT NOT NULL DEFAULT '{}'",
                [],
            )?;
        }
        // A note written afterwards: where in the house, on what device, what
        // was being tested. Belongs to the run rather than the client, since
        // that is the whole point of writing it down.
        if !existing.iter().any(|c| c == "note") {
            conn.execute(
                "ALTER TABLE runs ADD COLUMN note TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        // Which build measured this run.
        //
        // Added because a stored figure is only interpretable if you know what
        // produced it: every run recorded before 1.3.1 has its latency
        // inflated by up to 40 ms by the `TCP_NODELAY` bug, and until this
        // column existed there was no way to tell those rows apart from
        // correct ones. Empty means "recorded before the version was tracked",
        // which is a weaker claim than a version and is presented as one.
        if !existing.iter().any(|c| c == "app_version") {
            conn.execute(
                "ALTER TABLE runs ADD COLUMN app_version TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        Ok(())
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
        let points_json = match serde_json::to_string(&submission.points) {
            Ok(j) if j.len() <= MAX_POINTS_BYTES => j,
            _ => "{}".to_string(),
        };

        let conn = self.conn.lock().expect("history mutex");
        conn.execute(
            "INSERT INTO runs (
                 recorded_at, client_ip, user_agent, profile,
                 download, upload, latency, jitter,
                 down_loaded_latency, up_loaded_latency, packet_loss,
                 total_duration_ms, scores_json, summary_json, points_json,
                 app_version
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
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
                points_json,
                crate::VERSION,
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
                          r.total_duration_ms, r.scores_json, n.name, r.note,
                          r.app_version
                   FROM runs r
                   LEFT JOIN client_names c ON c.client_ip = r.client_ip
                   LEFT JOIN resolved_names n ON n.client_ip = r.client_ip
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
                hostname: non_empty(row.get::<_, Option<String>>(15)?),
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
                note: non_empty(row.get::<_, Option<String>>(16)?),
                app_version: non_empty(row.get::<_, Option<String>>(17)?),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Distinct clients seen, most recently active first.
    pub fn clients(&self) -> Result<Vec<ClientSummary>, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        let mut stmt = conn.prepare(
            "SELECT r.client_ip, c.name, COUNT(*), MAX(r.recorded_at), n.name
             FROM runs r
             LEFT JOIN client_names c ON c.client_ip = r.client_ip
             LEFT JOIN resolved_names n ON n.client_ip = r.client_ip
             GROUP BY r.client_ip
             ORDER BY MAX(r.recorded_at) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ClientSummary {
                client_ip: row.get(0)?,
                client_name: row.get(1)?,
                hostname: non_empty(row.get::<_, Option<String>>(4)?),
                runs: row.get(2)?,
                last_seen: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// The most recent run from each client.
    ///
    /// What a dashboard wants: one current reading per machine, rather than
    /// the whole history re-exported on every scrape. The window function
    /// picks the newest row per client in one pass — the alternative, a
    /// correlated subquery per client, gets slower with every run stored.
    pub fn latest_per_client(&self) -> Result<Vec<StoredRun>, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        let mut stmt = conn.prepare(
            "SELECT id, recorded_at, client_ip, client_name, hostname, user_agent, profile,
                    download, upload, latency, jitter,
                    down_loaded_latency, up_loaded_latency, packet_loss,
                    total_duration_ms, scores_json, note, app_version
             FROM (
               SELECT r.id, r.recorded_at, r.client_ip, c.name AS client_name,
                      n.name AS hostname, r.user_agent, r.profile,
                      r.download, r.upload, r.latency, r.jitter,
                      r.down_loaded_latency, r.up_loaded_latency, r.packet_loss,
                      r.total_duration_ms, r.scores_json, r.note, r.app_version,
                      ROW_NUMBER() OVER (
                        PARTITION BY r.client_ip
                        ORDER BY r.recorded_at DESC, r.id DESC
                      ) AS rank
               FROM runs r
               LEFT JOIN client_names c ON c.client_ip = r.client_ip
               LEFT JOIN resolved_names n ON n.client_ip = r.client_ip
             )
             WHERE rank = 1
             ORDER BY recorded_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let scores_json: String = row.get(15)?;
            Ok(StoredRun {
                id: row.get(0)?,
                recorded_at: row.get(1)?,
                client_ip: row.get(2)?,
                client_name: row.get(3)?,
                hostname: non_empty(row.get::<_, Option<String>>(4)?),
                user_agent: row.get(5)?,
                profile: row.get(6)?,
                download: row.get(7)?,
                upload: row.get(8)?,
                latency: row.get(9)?,
                jitter: row.get(10)?,
                down_loaded_latency: row.get(11)?,
                up_loaded_latency: row.get(12)?,
                packet_loss: row.get(13)?,
                total_duration_ms: row.get(14)?,
                scores: serde_json::from_str(&scores_json).unwrap_or_default(),
                note: non_empty(row.get::<_, Option<String>>(16)?),
                app_version: non_empty(row.get::<_, Option<String>>(17)?),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Drops old data, and reports what it dropped.
    ///
    /// Two windows because the two costs are different by orders of magnitude.
    /// A summary row is a couple of hundred bytes and is the thing you keep for
    /// a trend; the sample blob behind it can be a quarter of a megabyte and is
    /// only interesting while the run is recent. So the blobs can be released
    /// long before the rows they belong to, and a run that loses its samples
    /// still draws its headline — `points_json` of `{}` is exactly what every
    /// run stored before 1.3.0 already looks like.
    ///
    /// Both windows are off by default. Silently deleting a homelab's history
    /// because a default said so is not a behaviour to opt out of.
    ///
    /// Reclaiming the disk is part of this call rather than a second one the
    /// caller has to remember: a prune that frees rows but not space is the
    /// failure mode this whole path exists to avoid, and making it impossible
    /// to forget is cheaper than documenting it.
    pub fn prune(
        &self,
        runs_older_than: Option<&str>,
        samples_older_than: Option<&str>,
    ) -> Result<Pruned, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        let mut pruned = Pruned::default();

        if let Some(cutoff) = runs_older_than {
            pruned.runs_deleted =
                conn.execute("DELETE FROM runs WHERE recorded_at < ?1", params![cutoff])?;
        }
        if let Some(cutoff) = samples_older_than {
            pruned.samples_cleared = conn.execute(
                "UPDATE runs SET points_json = '{}'
                 WHERE recorded_at < ?1 AND points_json <> '{}'",
                params![cutoff],
            )?;
        }

        pruned.bytes_reclaimed = Self::reclaim(&conn)?;
        Ok(pruned)
    }

    /// Hands freed pages back to the filesystem and truncates the WAL.
    ///
    /// Two separate leaks, both invisible from the row count. A `DELETE` moves
    /// pages onto SQLite's free list, where they are reused by later inserts
    /// but never returned — `PRAGMA incremental_vacuum` is what returns them,
    /// and it does nothing at all unless the database is in
    /// `auto_vacuum = INCREMENTAL`, which is why that is set at open time.
    ///
    /// The WAL is the second. Every page touched by a prune is written into it,
    /// and a plain checkpoint leaves the file sitting at its high-water mark —
    /// after a large delete, that is the size of everything removed, kept on
    /// disk indefinitely. `TRUNCATE` is the mode that actually shortens it.
    fn reclaim(conn: &Connection) -> Result<u64, HistoryError> {
        let before = Self::file_bytes(conn)?;
        // Both of these are stepped to completion rather than executed, and
        // that is not a stylistic choice. `PRAGMA incremental_vacuum` emits one
        // row per page it frees and stops there, so a statement stepped once
        // frees exactly one page — 4 KiB of a multi-megabyte prune — while
        // reporting success. It looked like the pragma was being ignored.
        drain(conn, "PRAGMA incremental_vacuum")?;
        drain(conn, "PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(before.saturating_sub(Self::file_bytes(conn)?))
    }

    /// The size of the database as SQLite itself accounts for it.
    ///
    /// Asked of the database rather than of the filesystem so it means the same
    /// thing for the in-memory databases the tests use.
    fn file_bytes(conn: &Connection) -> Result<u64, HistoryError> {
        let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        Ok(pages.max(0) as u64 * page_size.max(0) as u64)
    }

    /// Checks a snapshot directory is usable and returns where the snapshot
    /// will land, creating the directory if it is not there.
    ///
    /// Called at startup as well as before every snapshot, so an unwritable
    /// directory — or one that would put the copy on top of the original — is a
    /// refusal to boot rather than a warning nobody reads on the first
    /// maintenance pass a day later.
    pub fn prepare_snapshot_dir(&self, dir: &Path) -> Result<PathBuf, HistoryError> {
        std::fs::create_dir_all(dir).map_err(|e| HistoryError::Snapshot {
            path: dir.to_path_buf(),
            detail: e.to_string(),
        })?;

        let destination = dir.join(SNAPSHOT_FILE);
        if let Some(live) = &self.path {
            if same_file(&destination, live) {
                return Err(HistoryError::SnapshotOverwritesDatabase { path: destination });
            }
        }
        Ok(destination)
    }

    /// Writes a consistent copy of the database into `dir`, and returns its
    /// size in bytes.
    ///
    /// `VACUUM INTO` rather than a file copy: the database is being served
    /// while this runs, and copying the file underneath a live writer produces
    /// something that may or may not open, discovered whenever it is next
    /// needed. It also compacts on the way out, so the snapshot is the smallest
    /// honest representation of the data.
    ///
    /// The copy is written to a temporary name and renamed into place, because
    /// the failure this guards against is not a failed snapshot — it is a
    /// half-written one sitting where the good one used to be. A rename within
    /// one directory is atomic, so the destination is either the previous
    /// snapshot or a complete new one, never something in between.
    pub fn snapshot(&self, dir: &Path) -> Result<u64, HistoryError> {
        let destination = self.prepare_snapshot_dir(dir)?;
        let temp = dir.join(SNAPSHOT_TEMP_FILE);

        // `VACUUM INTO` refuses to write to a file that already exists, so a
        // leftover from a run that died mid-snapshot would otherwise block
        // every snapshot from here on.
        if let Err(e) = std::fs::remove_file(&temp) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(HistoryError::Snapshot {
                    path: temp,
                    detail: e.to_string(),
                });
            }
        }

        {
            let conn = self.conn.lock().expect("history mutex");
            // The path is bound, not interpolated: a directory name is allowed
            // to contain a quote, and building this string by hand is how that
            // becomes a syntax error at 3am.
            conn.execute("VACUUM INTO ?1", params![temp.to_string_lossy()])?;
        }

        std::fs::rename(&temp, &destination).map_err(|e| HistoryError::Snapshot {
            path: destination.clone(),
            detail: e.to_string(),
        })?;

        Ok(std::fs::metadata(&destination)
            .map(|m| m.len())
            .unwrap_or(0))
    }

    pub fn count(&self) -> Result<i64, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        Ok(conn.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))?)
    }

    /// Names a client by hand. An empty name clears it, falling back to
    /// whatever reverse DNS found, and then to the address.
    pub fn set_client_name(&self, client_ip: &str, name: &str) -> Result<(), HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        let name = name.trim();
        if name.is_empty() {
            conn.execute(
                "DELETE FROM client_names WHERE client_ip = ?1",
                params![client_ip],
            )?;
            return Ok(());
        }
        conn.execute(
            "INSERT INTO client_names (client_ip, name) VALUES (?1, ?2)
             ON CONFLICT(client_ip) DO UPDATE SET name = excluded.name",
            params![client_ip, name],
        )?;
        Ok(())
    }

    /// Annotates one run, or clears the note when given an empty string.
    ///
    /// Returns whether a run with that id existed, so the caller can answer
    /// 404 rather than silently accepting a note about nothing.
    pub fn set_note(&self, id: i64, note: &str) -> Result<bool, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        let changed = conn.execute(
            "UPDATE runs SET note = ?2 WHERE id = ?1",
            params![id, note.trim()],
        )?;
        Ok(changed > 0)
    }

    /// A cached reverse-lookup result and when it was taken, if any.
    ///
    /// An entry with an empty name is a remembered miss — a client with no PTR
    /// record should not be looked up again on every run.
    pub fn resolved_name(&self, client_ip: &str) -> Result<Option<(String, String)>, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        Ok(conn
            .query_row(
                "SELECT name, resolved_at FROM resolved_names WHERE client_ip = ?1",
                params![client_ip],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn record_resolved_name(
        &self,
        client_ip: &str,
        name: &str,
        resolved_at: &str,
    ) -> Result<(), HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        conn.execute(
            "INSERT INTO resolved_names (client_ip, name, resolved_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(client_ip) DO UPDATE SET
                 name = excluded.name, resolved_at = excluded.resolved_at",
            params![client_ip, name, resolved_at],
        )?;
        Ok(())
    }

    /// One run in full, for a permalink.
    pub fn by_id(&self, id: i64) -> Result<Option<StoredRunDetail>, HistoryError> {
        let conn = self.conn.lock().expect("history mutex");
        let row = conn
            .query_row(
                "SELECT r.id, r.recorded_at, r.client_ip, c.name, r.user_agent, r.profile,
                        r.download, r.upload, r.latency, r.jitter,
                        r.down_loaded_latency, r.up_loaded_latency, r.packet_loss,
                        r.total_duration_ms, r.scores_json, n.name,
                        r.summary_json, r.points_json, r.note, r.app_version
                 FROM runs r
                 LEFT JOIN client_names c ON c.client_ip = r.client_ip
                 LEFT JOIN resolved_names n ON n.client_ip = r.client_ip
                 WHERE r.id = ?1",
                params![id],
                |row| {
                    let scores_json: String = row.get(14)?;
                    let summary_json: String = row.get(16)?;
                    let points_json: String = row.get(17)?;
                    Ok(StoredRunDetail {
                        run: StoredRun {
                            id: row.get(0)?,
                            recorded_at: row.get(1)?,
                            client_ip: row.get(2)?,
                            client_name: row.get(3)?,
                            hostname: non_empty(row.get::<_, Option<String>>(15)?),
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
                            note: non_empty(row.get::<_, Option<String>>(18)?),
                            app_version: non_empty(row.get::<_, Option<String>>(19)?),
                        },
                        summary: serde_json::from_str(&summary_json).unwrap_or_default(),
                        points: serde_json::from_str(&points_json).unwrap_or_default(),
                    })
                },
            )
            .optional()?;
        Ok(row)
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
    pub hostname: Option<String>,
    pub runs: i64,
    pub last_seen: String,
}

/// `Some("")` is how a remembered miss is stored; it is not a name.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

/// Runs a statement to completion, discarding whatever it emits.
///
/// The pragmas this module leans on do their work *while* producing rows, so
/// stepping once — which is all `execute` does — stops them part-way through.
fn drain(conn: &Connection, sql: &str) -> Result<(), HistoryError> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    while rows.next()?.is_some() {}
    Ok(())
}

/// Whether two paths name the same file, decided by the filesystem rather than
/// by how they are spelled.
///
/// Only the directories are resolved, since the files themselves need not exist
/// yet — which is enough to settle the question that matters here:
/// `data/history.db` and `./data/../data/history.db` differ as text and agree
/// as paths. Comparing the strings would answer the wrong question.
fn same_file(a: &Path, b: &Path) -> bool {
    fn resolved(p: &Path) -> Option<PathBuf> {
        let dir = match p.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
            _ => PathBuf::from("."),
        };
        Some(std::fs::canonicalize(dir).ok()?.join(p.file_name()?))
    }
    match (resolved(a), resolved(b)) {
        (Some(a), Some(b)) => a == b,
        // An unresolvable path is not evidence of a collision.
        _ => false,
    }
}

/// How long ago an RFC 3339 timestamp was, or `None` if it will not parse.
///
/// A clock that has moved backwards yields zero rather than a negative age, so
/// a cache entry stamped in the future expires immediately instead of never.
pub fn age_since(timestamp: &str) -> Option<std::time::Duration> {
    use time::format_description::well_known::Rfc3339;
    let then = time::OffsetDateTime::parse(timestamp, &Rfc3339).ok()?;
    let delta = time::OffsetDateTime::now_utc() - then;
    Some(delta.try_into().unwrap_or(std::time::Duration::ZERO))
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
            points: serde_json::json!({ "download": [{ "bps": download }] }),
        }
    }

    fn history() -> History {
        History::in_memory().unwrap()
    }

    /// A submission whose sample blob is large enough to be worth reclaiming,
    /// and small enough to survive `MAX_POINTS_BYTES`.
    fn fat_submission(download: f64) -> ResultSubmission {
        let mut s = submission(download);
        s.points = serde_json::json!({ "download": vec![download; 18_000] });
        assert!(
            serde_json::to_string(&s.points).unwrap().len() < MAX_POINTS_BYTES,
            "the fixture must not be dropped on the way in"
        );
        s
    }

    /// A directory of this test's own.
    ///
    /// Named after the caller as well as the process, because these tests share
    /// a process and run in parallel — one shared directory would have them
    /// deleting each other's databases.
    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("speedtest-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Everything SQLite has on disk for this database, WAL included.
    ///
    /// The WAL is the point: after a large delete it holds every page that was
    /// touched, so measuring only `history.db` would report a saving that the
    /// filesystem has not actually seen.
    fn on_disk(db: &Path) -> u64 {
        ["", "-wal", "-shm"]
            .iter()
            .map(|suffix| {
                let mut name = db.as_os_str().to_os_string();
                name.push(suffix);
                std::fs::metadata(PathBuf::from(name))
                    .map(|m| m.len())
                    .unwrap_or(0)
            })
            .sum()
    }

    fn mode(h: &History) -> i64 {
        let conn = h.conn.lock().unwrap();
        History::auto_vacuum_mode(&conn).unwrap()
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
            points: serde_json::Value::Null,
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
            points: serde_json::Value::Null,
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
    fn a_database_predating_the_version_column_gains_it_and_keeps_its_rows() {
        // The rows that matter most here are the ones already on disk: they were
        // measured by a build we can no longer identify, and the migration must
        // say so rather than inventing a version for them. Anything recorded
        // afterwards carries the real one.
        let dir = std::env::temp_dir().join(format!("speedtest-migrate-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.db");
        let _ = std::fs::remove_file(&path);

        {
            // A database in the pre-`app_version` shape, written by hand so the
            // test exercises the migration rather than today's schema.
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                     id          INTEGER PRIMARY KEY AUTOINCREMENT,
                     recorded_at TEXT NOT NULL,
                     client_ip   TEXT NOT NULL,
                     user_agent  TEXT NOT NULL DEFAULT '',
                     profile     TEXT NOT NULL DEFAULT '',
                     download    REAL, upload REAL, latency REAL, jitter REAL,
                     down_loaded_latency REAL, up_loaded_latency REAL,
                     packet_loss REAL, total_duration_ms REAL,
                     scores_json  TEXT NOT NULL DEFAULT '{}',
                     summary_json TEXT NOT NULL DEFAULT '{}'
                 );
                 INSERT INTO runs (recorded_at, client_ip, latency)
                 VALUES ('2026-08-24T10:00:00Z', '10.0.0.1', 41.9);",
            )
            .unwrap();
        }

        let h = History::open(&path).unwrap();
        assert_eq!(h.count().unwrap(), 1, "the existing row must survive");

        let old = &h.recent(10, None).unwrap()[0];
        assert_eq!(
            old.app_version, None,
            "a run from before the column existed has no version to claim"
        );

        h.record(&submission(1.0e9), "10.0.0.2", "ua", "2026-08-26T10:00:00Z")
            .unwrap();
        let new = &h.recent(10, None).unwrap()[0];
        assert_eq!(new.app_version.as_deref(), Some(crate::VERSION));

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

    #[test]
    fn a_new_database_is_created_in_incremental_auto_vacuum() {
        // Asserted against the running database rather than against the line
        // that sets it. SQLite reads this pragma once, when the first table is
        // created, and ignores it in silence afterwards — a correct-looking
        // assignment made one statement too late reads back as mode 0 and
        // nothing anywhere says so.
        let dir = temp_dir("autovacuum-new");
        let h = History::open(&dir.join("history.db")).unwrap();
        assert_eq!(mode(&h), AUTO_VACUUM_INCREMENTAL);

        // And the in-memory databases the tests run on, which go through the
        // same path and would otherwise let every other test here pass while
        // the deployed database reclaimed nothing.
        assert_eq!(mode(&history()), AUTO_VACUUM_INCREMENTAL);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_database_created_before_this_is_converted_and_keeps_its_rows() {
        // Every deployment that already exists has a database in mode 0, where
        // the pragma is accepted and ignored. Converting it costs one full
        // VACUUM, once, and the alternative is a file that grows for ever on
        // exactly the installations with the most history to prune.
        let dir = temp_dir("autovacuum-existing");
        let path = dir.join("history.db");

        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                     id          INTEGER PRIMARY KEY AUTOINCREMENT,
                     recorded_at TEXT NOT NULL,
                     client_ip   TEXT NOT NULL,
                     user_agent  TEXT NOT NULL DEFAULT '',
                     profile     TEXT NOT NULL DEFAULT '',
                     download    REAL, upload REAL, latency REAL, jitter REAL,
                     down_loaded_latency REAL, up_loaded_latency REAL,
                     packet_loss REAL, total_duration_ms REAL,
                     scores_json  TEXT NOT NULL DEFAULT '{}',
                     summary_json TEXT NOT NULL DEFAULT '{}'
                 );
                 INSERT INTO runs (recorded_at, client_ip, download)
                 VALUES ('2026-08-24T10:00:00Z', '10.0.0.1', 9.4e8);",
            )
            .unwrap();
            let existing: i64 = conn
                .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
                .unwrap();
            assert_eq!(existing, 0, "the fixture must start in the old mode");
        }

        let h = History::open(&path).unwrap();
        assert_eq!(mode(&h), AUTO_VACUUM_INCREMENTAL);
        assert_eq!(h.count().unwrap(), 1, "the VACUUM must not lose the row");
        assert_eq!(h.recent(1, None).unwrap()[0].download, Some(9.4e8));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pruning_hands_the_disk_back_instead_of_only_the_rows() {
        // The property, not the pragma: a prune that deletes every row while
        // the file stays the same size is the failure this guards against, and
        // it is indistinguishable from success in the row count alone. The WAL
        // is measured too, because a checkpoint without TRUNCATE leaves it at
        // the high-water mark — which after this prune is everything deleted.
        let dir = temp_dir("prune-reclaim");
        let path = dir.join("history.db");
        let h = History::open(&path).unwrap();

        for i in 0..24 {
            h.record(
                &fat_submission(9.4e8),
                "10.0.0.1",
                "ua",
                &format!("2026-08-{:02}T10:00:00Z", i + 1),
            )
            .unwrap();
        }
        let before = on_disk(&path);
        assert!(before > 2 * 1024 * 1024, "fixture too small to be a test");

        let pruned = h.prune(Some("2026-09-01T00:00:00Z"), None).unwrap();
        assert_eq!(pruned.runs_deleted, 24);
        assert_eq!(h.count().unwrap(), 0);
        assert!(
            pruned.bytes_reclaimed > 0,
            "SQLite freed the pages but kept the file"
        );

        let after = on_disk(&path);
        assert!(
            after < before / 4,
            "still {after} bytes on disk against {before} before the prune"
        );

        let mut wal = path.as_os_str().to_os_string();
        wal.push("-wal");
        let wal = std::fs::metadata(PathBuf::from(wal))
            .map(|m| m.len())
            .unwrap_or(0);
        assert!(
            wal < 256 * 1024,
            "the WAL is {wal} bytes — it was checkpointed but not truncated"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pruning_nothing_is_still_safe_to_run() {
        // Both windows off is the default, and a deployment that only wants the
        // nightly snapshot takes this path every day.
        let h = history();
        h.record(&submission(1.0e9), "10.0.0.1", "ua", "2026-08-24T10:00:00Z")
            .unwrap();

        let pruned = h.prune(None, None).unwrap();
        assert_eq!(pruned, Pruned::default());
        assert_eq!(h.count().unwrap(), 1, "nothing was asked for, nothing went");
    }

    #[test]
    fn a_snapshot_is_a_readable_database_holding_the_same_runs() {
        // A backup that cannot be opened is worse than none, because it is
        // believed. So the copy is opened and questioned rather than weighed.
        let dir = temp_dir("snapshot-valid");
        let backups = dir.join("backups");
        let h = History::open(&dir.join("history.db")).unwrap();

        for i in 0..3 {
            h.record(
                &submission(1.0e9),
                "10.0.0.1",
                "ua",
                &format!("2026-08-2{i}T10:00:00Z"),
            )
            .unwrap();
        }

        let bytes = h.snapshot(&backups).unwrap();
        assert!(bytes > 0);

        let copy = backups.join(SNAPSHOT_FILE);
        assert!(copy.is_file(), "{copy:?} should exist");
        assert!(
            !backups.join(SNAPSHOT_TEMP_FILE).exists(),
            "the temporary file should have been renamed away, not left behind"
        );

        let conn = rusqlite::Connection::open(&copy).unwrap();
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(runs, 3);
        let download: f64 = conn
            .query_row("SELECT download FROM runs LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(download, 1.0e9);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_half_written_snapshot_left_by_a_crash_neither_blocks_nor_replaces_the_good_one() {
        // `VACUUM INTO` refuses an existing file, so rubbish at the temporary
        // name would otherwise stop every snapshot from that day onwards —
        // silently, once a day, until someone looked. The previous good copy
        // must also still be the previous good copy: the rename is what makes
        // that true, and this is the closest a test in one process can get to
        // watching it.
        let dir = temp_dir("snapshot-crash");
        let backups = dir.join("backups");
        let h = History::open(&dir.join("history.db")).unwrap();

        h.record(&submission(1.0e9), "10.0.0.1", "ua", "2026-08-24T10:00:00Z")
            .unwrap();
        h.snapshot(&backups).unwrap();

        // What a process killed mid-`VACUUM INTO` leaves behind.
        std::fs::write(
            backups.join(SNAPSHOT_TEMP_FILE),
            b"SQLite format 3\0truncated",
        )
        .unwrap();

        // The good snapshot is untouched by that, and still opens.
        let copy = backups.join(SNAPSHOT_FILE);
        let conn = rusqlite::Connection::open(&copy).unwrap();
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(runs, 1);
        drop(conn);

        h.record(&submission(2.0e9), "10.0.0.1", "ua", "2026-08-25T10:00:00Z")
            .unwrap();
        h.snapshot(&backups).unwrap();

        let conn = rusqlite::Connection::open(&copy).unwrap();
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            runs, 2,
            "the second snapshot should have replaced the first"
        );
        assert!(!backups.join(SNAPSHOT_TEMP_FILE).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_snapshot_refuses_to_be_written_over_the_live_database() {
        // Naming the database `history-backup.db` and its own directory as the
        // backup directory would rename a copy of the database on top of the
        // database. The check is made against the filesystem rather than
        // against the spelling of the two paths, so a directory reached by a
        // different-looking route is still the same directory.
        let dir = temp_dir("snapshot-collision");
        let h = History::open(&dir.join(SNAPSHOT_FILE)).unwrap();

        assert!(matches!(
            h.snapshot(&dir),
            Err(HistoryError::SnapshotOverwritesDatabase { .. })
        ));
        assert!(matches!(
            h.snapshot(&dir.join("sub").join("..")),
            Err(HistoryError::SnapshotOverwritesDatabase { .. })
        ));

        // The database is still there, and still a database.
        assert_eq!(h.count().unwrap(), 0);

        // A different directory is fine, even though the name is the same.
        h.snapshot(&dir.join("elsewhere")).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_snapshot_directory_whose_name_contains_a_quote_still_round_trips() {
        // `VACUUM INTO` takes an expression, and the obvious way to write it is
        // to paste the path into the SQL. Round-tripped through SQLite rather
        // than inspected as a string: this passes only if the path is bound.
        let dir = temp_dir("snapshot-quoting");
        let awkward = dir.join("Stephen's backups; DROP TABLE runs--");
        let h = History::open(&dir.join("history.db")).unwrap();
        h.record(&submission(1.0e9), "10.0.0.1", "ua", "2026-08-24T10:00:00Z")
            .unwrap();

        h.snapshot(&awkward).unwrap();

        let conn = rusqlite::Connection::open(awkward.join(SNAPSHOT_FILE)).unwrap();
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(runs, 1);
        assert_eq!(
            h.count().unwrap(),
            1,
            "and the original still has its table"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unusable_snapshot_directory_is_reported_before_it_is_needed() {
        // `prepare_snapshot_dir` is what startup calls, so a directory that
        // cannot be made is a refusal to boot rather than a warning at 3am.
        let dir = temp_dir("snapshot-unusable");
        let file = dir.join("not-a-directory");
        std::fs::write(&file, b"x").unwrap();

        let h = History::open(&dir.join("history.db")).unwrap();
        assert!(matches!(
            h.prepare_snapshot_dir(&file),
            Err(HistoryError::Snapshot { .. })
        ));

        // And the usable case returns where the snapshot will land, which is
        // what the startup log says out loud.
        assert_eq!(
            h.prepare_snapshot_dir(&dir.join("backups")).unwrap(),
            dir.join("backups").join(SNAPSHOT_FILE)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
