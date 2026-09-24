//! SQLite history: every sample, every outage, every applied tweak.
//!
//! Samples are buffered and flushed in batches — at one sweep per second
//! across four targets a transaction per sample would hammer the disk for no
//! reason.

use crate::probe::igd::RouterReading;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::settings;

/// A hole in the samples wider than this means nobody was watching: the app
/// was closed or paused, or the machine was asleep. The sweep runs about once
/// a second, so a minute is far past a slow sweep and far short of anything a
/// user would call "still running".
pub const OBSERVATION_GAP_S: f64 = 60.0;

/// How long an outage is kept, whatever the sample retention: a year, so a
/// report to the provider can reach back further than two weeks. Past the
/// sample retention it is kept without its lead-up, which is most of its
/// size.
pub const OUTAGE_KEEP_DAYS: i64 = 365;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS samples (
    ts     REAL NOT NULL,
    target TEXT NOT NULL,
    rtt_ms REAL,
    ok     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_samples_ts ON samples(ts);
CREATE INDEX IF NOT EXISTS idx_samples_target_ts ON samples(target, ts);

CREATE TABLE IF NOT EXISTS events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_start    REAL NOT NULL,
    ts_end      REAL,
    kind        TEXT NOT NULL,
    scope       TEXT NOT NULL,
    detail      TEXT,
    context     TEXT,
    context_end TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_start ON events(ts_start);
CREATE INDEX IF NOT EXISTS idx_events_scope ON events(scope, ts_start);

CREATE TABLE IF NOT EXISTS router (
    ts       REAL NOT NULL,
    status   TEXT NOT NULL,
    uptime_s INTEGER,
    ip_tag   TEXT
);
CREATE INDEX IF NOT EXISTS idx_router_ts ON router(ts);

CREATE TABLE IF NOT EXISTS tweaks (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    ts       REAL NOT NULL,
    tweak_id TEXT NOT NULL,
    action   TEXT NOT NULL,
    before   TEXT,
    result   TEXT
);
"#;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub count: usize,
    pub loss_pct: f64,
    pub avg: Option<f64>,
    /// The middle reading. Where "what is normal for this line" is the
    /// question, this is the answer and `avg` is not: one bad evening inside
    /// the window drags the mean up for as long as the window lasts, and a
    /// baseline that absorbs the fault stops reporting it.
    pub median: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub jitter: Option<f64>,
}

/// One outage, as the lists and tables read it.
///
/// Deliberately without the stored context: see [`EventContext`], which is
/// fetched a row at a time. A struct that carried both made "list the day's
/// outages" and "read this one's evidence" the same query, and the cheap one
/// paid for the expensive one on every frame.
#[derive(Debug, Clone)]
pub struct Event {
    pub id: i64,
    pub ts_start: f64,
    pub ts_end: Option<f64>,
    pub kind: String,
    pub scope: String,
    pub detail: String,
}

impl Event {
    pub fn duration_s(&self) -> Option<f64> {
        self.ts_end.map(|e| e - self.ts_start)
    }
}

/// The heavy half of an outage row, fetched only for the one row a person is
/// looking at.
///
/// It lives apart from [`Event`] because the two are read at completely
/// different rates: the list queries run every frame over hundreds of rows,
/// and this is 33 KB each. Keeping them in one struct meant every list query
/// paid for evidence nothing on screen was reading.
#[derive(Debug, Clone)]
pub struct EventContext {
    /// Connection state as JSON at the moment the outage opened, including the
    /// `lead_up` series of sweeps that preceded it. This is the evidence the
    /// cause analysis reads; an empty string means the row predates it.
    pub context: String,
    /// Connection state as JSON at the moment the connection came back.
    pub context_end: Option<String>,
}

impl EventContext {
    /// The stored context, parsed. Rows written by older builds — or by a
    /// build that failed to serialise — simply have nothing to say.
    pub fn context_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.context).ok()
    }

    pub fn context_end_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(self.context_end.as_deref()?).ok()
    }
}

#[derive(Debug, Clone)]
pub struct TweakLogRow {
    pub ts: f64,
    pub tweak_id: String,
    pub action: String,
    pub result: String,
}

pub struct Store {
    /// Everything that writes. The monitor owns this path in practice: a
    /// sweep's samples, an outage opening and closing, the nightly prune.
    conn: Mutex<Connection>,
    /// Everything that reads, on its own connection so the two never queue
    /// behind each other.
    ///
    /// The comment this replaces claimed "WAL keeps the writer from blocking
    /// the UI thread's reads". True of SQLite, and false of this code: with
    /// one `Mutex<Connection>` the two serialise in Rust before SQLite ever
    /// sees them, so a 7 ms chart query held the lock the monitor needed to
    /// record a sample. WAL only pays for itself across connections, which is
    /// what this is.
    ///
    /// `None` for an in-memory database, where a second connection would be a
    /// second, empty database rather than another view of the same one.
    read: Option<Mutex<Connection>>,
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

impl Store {
    /// Takes the connection lock, ignoring poisoning.
    ///
    /// A poisoned mutex only means another thread panicked while holding it.
    /// The SQLite connection behind it is untouched, and refusing to use it
    /// would turn one panic into a dead history for the rest of the session —
    /// in the component whose whole job is to still have the evidence
    /// afterwards.
    fn held(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The lock every read takes. Falls back to the writer's connection when
    /// there is no second one — an in-memory database — where the two cannot
    /// be separated and there is no disk to contend for anyway.
    fn held_read(&self) -> std::sync::MutexGuard<'_, Connection> {
        match &self.read {
            Some(r) => r.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            None => self.held(),
        }
    }

    pub fn open_default() -> Result<Self> {
        std::fs::create_dir_all(settings::data_dir())?;
        Self::open(settings::db_path())
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path)?;
        // WAL is what lets the reader below see a consistent database while
        // the writer is mid-transaction, instead of one of them waiting.
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn);

        // Opened after the schema exists, and marked read-only in SQLite
        // itself: "the UI never writes through this" is then a fact about the
        // connection rather than a convention about which method to call.
        let read = Connection::open(path).and_then(|r| {
            r.execute_batch("PRAGMA query_only=ON; PRAGMA busy_timeout=2000;")?;
            Ok(r)
        });

        let store = Store { conn: Mutex::new(conn), read: read.ok().map(Mutex::new) };
        store.close_orphans();
        Ok(store)
    }

    /// Closes outages that a previous run left open.
    ///
    /// `run_loop` closes the open event when it exits cleanly, so a row still
    /// open at startup means the process was killed, crashed, or the machine
    /// lost power mid-outage. Nothing used to reconcile those, and an outage
    /// with no end is not inert: retention keeps it forever, the history tab
    /// asks the event log for everything between its start and *now*, and the
    /// cause rules then read months of unrelated faults as evidence for it.
    ///
    /// The last sample we managed to record is the best estimate of when the
    /// app stopped watching, so that is the end time. `context_end` records
    /// that the end was inferred rather than observed, which is the honest
    /// thing to store and lets the UI say so.
    fn close_orphans(&self) {
        let conn = self.held();
        let last_sample: Option<f64> =
            conn.query_row("SELECT MAX(ts) FROM samples", [], |r| r.get(0)).ok().flatten();
        let _ = conn.execute(
            "UPDATE events \
                SET ts_end = MAX(ts_start, COALESCE(?1, ts_start)), \
                    context_end = ?2 \
              WHERE ts_end IS NULL",
            params![last_sample, r#"{"closed_by":"restart"}"#],
        );
    }

    /// Backdates or stretches every event, so a test can build a history
    /// that would otherwise take an hour of wall clock to produce.
    #[cfg(test)]
    pub(crate) fn reshape_events_for_test(&self, start_delta: f64, duration: f64) {
        let conn = self.held();
        conn.execute(
            "UPDATE events SET ts_start = ts_start + ?1, ts_end = ts_start + ?1 + ?2",
            params![start_delta, duration],
        )
        .unwrap();
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn);
        Ok(Store { conn: Mutex::new(conn), read: None })
    }

    pub fn add_samples(&self, rows: &[(f64, String, Option<f64>, bool)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut conn = self.held();
        let tx = conn.transaction()?;
        {
            let mut stmt =
                tx.prepare_cached("INSERT INTO samples (ts, target, rtt_ms, ok) VALUES (?,?,?,?)")?;
            for (ts, target, rtt, ok) in rows {
                stmt.execute(params![ts, target, rtt, *ok as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Loss, average, extremes and jitter over a window.
    ///
    /// ponytail: this pulls every row in the window and reduces in Rust,
    /// because jitter is the mean gap between *consecutive* samples and needs
    /// the order. Retention bounds the worst case (14 days × 1 sweep/s × a
    /// handful of targets), and the windows the UI asks for are minutes, not
    /// days. If a full-history view ever ships, move the reduction into SQL
    /// with `LAG(rtt_ms) OVER (ORDER BY ts)` rather than making this bigger.
    pub fn stats(&self, target: &str, window_s: f64) -> Stats {
        self.stats_between(target, now() - window_s, f64::INFINITY)
    }

    /// [`Store::stats`] over `from <= ts < to`, for a stretch that does not
    /// end now: the day before a change was applied, for one.
    pub fn stats_between(&self, target: &str, from: f64, to: f64) -> Stats {
        let conn = self.held_read();
        let mut stmt = match conn.prepare_cached(
            "SELECT rtt_ms, ok FROM samples WHERE target=? AND ts>=? AND ts<? ORDER BY ts",
        ) {
            Ok(s) => s,
            Err(_) => return Stats::default(),
        };
        let rows = stmt.query_map(params![target, from, to], |r| {
            Ok((r.get::<_, Option<f64>>(0)?, r.get::<_, i64>(1)? != 0))
        });
        let Ok(rows) = rows else {
            return Stats::default();
        };

        let mut rtts: Vec<f64> = Vec::new();
        let mut total = 0usize;
        for row in rows.flatten() {
            total += 1;
            if let (Some(v), true) = row {
                rtts.push(v);
            }
        }
        summarise(total, &rtts)
    }

    /// Applies the retention setting.
    ///
    /// Samples go after `keep_days`. Outages stay for [`OUTAGE_KEEP_DAYS`],
    /// because a report to the provider has to reach back further than two
    /// weeks, but after `keep_days` they lose their lead-up: that series is
    /// most of an outage's 33 KB, and it is what made them the bulk of the
    /// file when they were exempt from pruning altogether. What stays is the
    /// time, the kind, the note, the state it failed on and the path.
    ///
    /// The tweak log is deliberately left alone: it is small, and it is the
    /// record of what this app changed on the machine.
    pub fn prune(&self, keep_days: i64) -> Result<()> {
        let keep_days = keep_days.max(1);
        let cutoff = now() - (keep_days as f64) * 86400.0;
        let outage_cutoff = now() - (keep_days.max(OUTAGE_KEEP_DAYS) as f64) * 86400.0;
        let conn = self.held();
        conn.execute("DELETE FROM samples WHERE ts < ?", params![cutoff])?;
        conn.execute("DELETE FROM router WHERE ts < ?", params![cutoff])?;
        // An outage still open has no end yet and is never old enough to
        // touch. `close_orphans` runs at startup so this only ever spares one
        // that is genuinely still running, rather than every row a crash
        // left behind.
        conn.execute(
            "UPDATE events SET context = json_remove(context, '$.lead_up') \
              WHERE ts_start < ?1 AND ts_end IS NOT NULL \
                AND json_valid(context) AND json_type(context, '$.lead_up') IS NOT NULL",
            params![cutoff],
        )?;
        conn.execute(
            "DELETE FROM events WHERE ts_start < ? AND ts_end IS NOT NULL",
            params![outage_cutoff],
        )?;
        Ok(())
    }

    /// Deletes every finished outage, at the user's request. Returns how many
    /// went.
    ///
    /// An outage still open is kept: the monitor holds its id and will close
    /// it, and a row it cannot find would lose the end of an outage that is
    /// happening now. The samples stay too; they are the live chart and the
    /// record of how long the line was watched, not the outage history.
    pub fn clear_events(&self) -> Result<usize> {
        let conn = self.held();
        Ok(conn.execute("DELETE FROM events WHERE ts_end IS NOT NULL", [])?)
    }

    pub fn open_event(&self, kind: &str, scope: &str, detail: &str, context: &str) -> Result<i64> {
        let conn = self.held();
        conn.execute(
            "INSERT INTO events (ts_start, kind, scope, detail, context) VALUES (?,?,?,?,?)",
            params![now(), kind, scope, detail, context],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Renames an open outage to a graver state it reached after it opened.
    /// The context stays: it is the evidence from the moment it started.
    pub fn set_event_kind(&self, id: i64, kind: &str, scope: &str, detail: &str) -> Result<()> {
        let conn = self.held();
        conn.execute(
            "UPDATE events SET kind = ?1, scope = ?2, detail = ?3 WHERE id = ?4",
            params![kind, scope, detail, id],
        )?;
        Ok(())
    }

    /// Closes an outage, recording the state it recovered *into*. The pair of
    /// contexts is what separates "the signal came back" from "the adapter was
    /// reset" — the recovery is as diagnostic as the failure.
    ///
    /// Tests only: the monitor always closes at a moment it names, with
    /// [`Store::close_event_at`], because "now" is wrong after any pause.
    #[cfg(test)]
    pub fn close_event(&self, id: i64, context_end: &str) -> Result<()> {
        let conn = self.held();
        conn.execute(
            "UPDATE events SET ts_end=?, context_end=? WHERE id=?",
            params![now(), context_end, id],
        )?;
        Ok(())
    }

    /// Closes an outage at a given moment rather than now: the last sweep
    /// that actually watched it. Used when watching stopped — a pause, a
    /// sleeping machine — so the time nobody measured is not billed to the
    /// outage. Never earlier than its start.
    pub fn close_event_at(&self, id: i64, ts_end: f64, context_end: &str) -> Result<()> {
        let conn = self.held();
        conn.execute(
            "UPDATE events SET ts_end = MAX(ts_start, ?1), context_end = ?2 WHERE id = ?3",
            params![ts_end, context_end, id],
        )?;
        Ok(())
    }

    pub fn events_since(&self, window_s: f64) -> Vec<Event> {
        self.query_events("WHERE ts_start >= ? ORDER BY ts_start DESC", Some(now() - window_s))
    }

    /// When the current unbroken stretch of watching began, or `None` on an
    /// empty history.
    ///
    /// An empty history is not a clean history, and neither is a history with
    /// holes in it: without this a fresh install claims "24 h+ uninterrupted"
    /// a minute after it starts, and a machine that was asleep for a week
    /// claims the week. A gap wider than `gap_s` means nobody was watching,
    /// so the stretch starts again on the far side of it.
    ///
    /// ponytail: scans one day of distinct sweep timestamps (~86k rows), which
    /// is why [`crate::monitor`] calls it once at startup and tracks the gaps
    /// itself afterwards. Anything older than a day cannot change the answer,
    /// because the card it feeds tops out at 24 h.
    pub fn observing_since(&self, gap_s: f64) -> Option<f64> {
        let conn = self.held_read();
        conn.query_row(
            "SELECT MAX(ts) FROM (
                 SELECT ts, ts - LAG(ts) OVER (ORDER BY ts) AS gap
                 FROM (SELECT DISTINCT ts FROM samples WHERE ts >= ?1)
             ) WHERE gap IS NULL OR gap > ?2",
            params![now() - 24.0 * 3600.0, gap_s],
            |r| r.get(0),
        )
        .ok()
        .flatten()
    }

    /// How many seconds of `[from, to]` were actually watched: the time
    /// between consecutive sweeps, where they are no more than `gap_s` apart.
    ///
    /// "No outages in the last day" means nothing unless somebody watched the
    /// day. This is what says how much of it they did.
    ///
    /// ponytail: the same one-day scan of distinct sweep timestamps as
    /// [`Store::observing_since`], so it belongs in a scan or a report, not
    /// in anything that runs every frame.
    pub fn observed_seconds(&self, from: f64, to: f64, gap_s: f64) -> f64 {
        let conn = self.held_read();
        conn.query_row(
            "SELECT COALESCE(SUM(gap), 0) FROM (
                 SELECT ts - LAG(ts) OVER (ORDER BY ts) AS gap
                 FROM (SELECT DISTINCT ts FROM samples WHERE ts >= ?1 AND ts <= ?2)
             ) WHERE gap <= ?3",
            params![from, to, gap_s],
            |r| r.get(0),
        )
        .unwrap_or(0.0)
    }

    /// The newest sample on record. Tells a restart whether it is resuming a
    /// stretch of observation or beginning one.
    pub fn last_sample_ts(&self) -> Option<f64> {
        let conn = self.held_read();
        conn.query_row("SELECT MAX(ts) FROM samples", [], |r| r.get(0)).ok().flatten()
    }

    /// Every sample for every target inside a time span, oldest first. This is
    /// what draws an outage's own timeline instead of a rolling live window.
    pub fn samples_between(&self, from: f64, to: f64) -> Vec<(f64, String, Option<f64>, bool)> {
        let conn = self.held_read();
        let Ok(mut stmt) = conn.prepare_cached(
            "SELECT ts, target, rtt_ms, ok FROM samples WHERE ts>=? AND ts<=? ORDER BY ts",
        ) else {
            return Vec::new();
        };
        let rows = stmt.query_map(params![from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0))
        });
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    /// Tweaks applied inside a time span. A change made minutes before an
    /// outage is the first thing worth suspecting, and until now nothing in
    /// the app ever put the two tables side by side.
    pub fn tweaks_between(&self, from: f64, to: f64) -> Vec<TweakLogRow> {
        let conn = self.held_read();
        let Ok(mut stmt) = conn.prepare_cached(
            "SELECT ts, tweak_id, action, COALESCE(result,'') FROM tweaks \
             WHERE ts>=? AND ts<=? ORDER BY ts DESC",
        ) else {
            return Vec::new();
        };
        let rows = stmt.query_map(params![from, to], |r| {
            Ok(TweakLogRow {
                ts: r.get(0)?,
                tweak_id: r.get(1)?,
                action: r.get(2)?,
                result: r.get(3)?,
            })
        });
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    /// One answer from the router's UPnP service. See [`crate::probe::igd`].
    pub fn add_router_reading(&self, r: &RouterReading) -> Result<()> {
        self.held().execute(
            "INSERT INTO router (ts, status, uptime_s, ip_tag) VALUES (?,?,?,?)",
            params![r.ts, r.status, r.uptime_s.map(|u| u as i64), r.ip_tag],
        )?;
        Ok(())
    }

    /// The router's answers inside a time span, oldest first.
    pub fn router_between(&self, from: f64, to: f64) -> Vec<RouterReading> {
        let conn = self.held_read();
        let Ok(mut stmt) = conn.prepare_cached(
            "SELECT ts, status, uptime_s, ip_tag FROM router WHERE ts>=? AND ts<=? ORDER BY ts",
        ) else {
            return Vec::new();
        };
        let rows = stmt.query_map(params![from, to], |r| {
            Ok(RouterReading {
                ts: r.get(0)?,
                status: r.get(1)?,
                uptime_s: r.get::<_, Option<i64>>(2)?.map(|u| u.max(0) as u64),
                ip_tag: r.get(3)?,
            })
        });
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    /// The stored evidence for one outage. `None` when the row is gone.
    pub fn event_context(&self, id: i64) -> Option<EventContext> {
        let conn = self.held_read();
        conn.query_row(
            "SELECT COALESCE(context,''), context_end FROM events WHERE id=?",
            params![id],
            |r| Ok(EventContext { context: r.get(0)?, context_end: r.get(1)? }),
        )
        .ok()
    }

    pub fn recent_events(&self, limit: usize) -> Vec<Event> {
        let conn = self.held_read();
        let sql =
            format!("SELECT {EVENT_COLUMNS} FROM events ORDER BY ts_start DESC LIMIT {limit}");
        let Ok(mut stmt) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map([], map_event);
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    fn query_events(&self, tail: &str, arg: Option<f64>) -> Vec<Event> {
        let conn = self.held_read();
        let sql = format!("SELECT {EVENT_COLUMNS} FROM events {tail}");
        let Ok(mut stmt) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let rows = match arg {
            Some(v) => stmt.query_map(params![v], map_event),
            None => stmt.query_map([], map_event),
        };
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    pub fn log_tweak(&self, tweak_id: &str, action: &str, before: &str, result: &str) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO tweaks (ts, tweak_id, action, before, result) VALUES (?,?,?,?,?)",
                params![now(), tweak_id, action, before, result],
            );
        }
    }

    pub fn tweak_log(&self, limit: usize) -> Vec<TweakLogRow> {
        let conn = self.held_read();
        let sql = format!(
            "SELECT ts, tweak_id, action, COALESCE(result,'') FROM tweaks ORDER BY ts DESC LIMIT {limit}"
        );
        let Ok(mut stmt) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map([], |r| {
            Ok(TweakLogRow {
                ts: r.get(0)?,
                tweak_id: r.get(1)?,
                action: r.get(2)?,
                result: r.get(3)?,
            })
        });
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }
}

/// The column order every event *list* query uses.
///
/// `context` and `context_end` are not here on purpose. They are the two
/// heavy columns — 33 KB per outage — and the queries that name this run over
/// hundreds of rows every frame. [`Store::event_context`] fetches them for the
/// single row that is actually on screen.
const EVENT_COLUMNS: &str = "id, ts_start, ts_end, kind, scope, COALESCE(detail,'')";

fn map_event(r: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        ts_start: r.get(1)?,
        ts_end: r.get(2)?,
        kind: r.get(3)?,
        scope: r.get(4)?,
        detail: r.get(5)?,
    })
}

/// Adds columns that later versions introduced. `CREATE TABLE IF NOT EXISTS`
/// leaves an existing table exactly as it was, so a database written by an
/// earlier build keeps its old shape and every query naming a new column fails
/// at prepare time — silently, because the callers here return empty vectors
/// on error. Adding the column is cheap and idempotent enough to run always.
fn migrate(conn: &Connection) {
    let existing: Vec<String> = conn
        .prepare("PRAGMA table_info(events)")
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(1)).map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    for (name, decl) in [("context", "TEXT"), ("context_end", "TEXT")] {
        if !existing.iter().any(|c| c == name) {
            let _ = conn.execute_batch(&format!("ALTER TABLE events ADD COLUMN {name} {decl}"));
        }
    }
}

/// Loss, average, median, extremes and mean consecutive deviation (jitter).
///
/// `rtts` must be in the order they were sampled — jitter is the mean gap
/// between *consecutive* readings, so sorting them here would silently turn
/// it into something else. The median takes its own sorted copy.
pub fn summarise(total: usize, rtts: &[f64]) -> Stats {
    if total == 0 {
        return Stats::default();
    }
    let lost = total - rtts.len();
    let loss_pct = lost as f64 / total as f64 * 100.0;
    if rtts.is_empty() {
        return Stats { count: total, loss_pct, ..Default::default() };
    }
    let avg = rtts.iter().sum::<f64>() / rtts.len() as f64;
    let min = rtts.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = rtts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let jitter = if rtts.len() > 1 {
        let sum: f64 = rtts.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        Some(sum / (rtts.len() - 1) as f64)
    } else {
        None
    };
    Stats {
        count: total,
        loss_pct,
        avg: Some(avg),
        median: Some(median(rtts)),
        min: Some(min),
        max: Some(max),
        jitter,
    }
}

/// The middle reading of a non-empty slice, averaging the two middles on an
/// even count. Takes a copy: the caller's order carries meaning.
fn median(rtts: &[f64]) -> f64 {
    let mut sorted = rtts.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_readings_come_back_in_order_and_are_pruned_with_samples() {
        let store = Store::open_in_memory().unwrap();
        let t = now();
        for (dt, status) in [(-20.0, "Connected"), (-10.0, "Disconnected")] {
            let r = RouterReading {
                ts: t + dt,
                status: status.into(),
                uptime_s: Some(100),
                ip_tag: Some("ab".into()),
            };
            store.add_router_reading(&r).unwrap();
        }
        let old = RouterReading {
            ts: t - 30.0 * 86_400.0,
            status: "Connected".into(),
            uptime_s: None,
            ip_tag: None,
        };
        store.add_router_reading(&old).unwrap();
        let got = store.router_between(t - 60.0, t);
        let statuses: Vec<&str> = got.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(statuses, ["Connected", "Disconnected"]);
        assert_eq!(got[0].uptime_s, Some(100));
        store.prune(14).unwrap();
        assert!(store.router_between(0.0, t).iter().all(|r| r.ts > t - 86_400.0));
    }

    #[test]
    fn outages_outlive_the_samples_but_not_their_lead_up() {
        // An outage from three weeks ago was deleted with the samples, so a
        // month's report to the provider could never show it.
        let store = Store::open_in_memory().unwrap();
        let ctx = r#"{"medium":"Wi-Fi","lead_up":[{"ts":1.0}],"path":{"hops":[]}}"#;
        let id = store.open_event("isp_down", "isp", "old", ctx).unwrap();
        store.close_event(id, "{}").unwrap();
        store.reshape_events_for_test(-21.0 * 86_400.0, 60.0);
        let old = now() - 21.0 * 86_400.0;
        store.add_samples(&[(old, "gateway".to_string(), Some(1.0), true)]).unwrap();

        store.prune(14).unwrap();

        assert!(store.samples_between(old - 1.0, old + 1.0).is_empty(), "samples go");
        let kept = store.events_since(30.0 * 86_400.0);
        assert_eq!(kept.len(), 1, "the outage stays");
        let context = store.event_context(id).unwrap().context;
        let v: serde_json::Value = serde_json::from_str(&context).unwrap();
        assert!(v.get("lead_up").is_none(), "without the bulky lead-up: {context}");
        assert_eq!(v["medium"], "Wi-Fi", "and with everything else");
        assert!(v.get("path").is_some());

        // A year on, it goes too.
        store.reshape_events_for_test(-(OUTAGE_KEEP_DAYS as f64) * 86_400.0, 60.0);
        store.prune(14).unwrap();
        assert!(store.events_since(1000.0 * 86_400.0).is_empty());
    }

    #[test]
    fn only_the_time_between_close_sweeps_counts_as_watched() {
        let store = Store::open_in_memory().unwrap();
        let t = now() - 1000.0;
        // Ten minutes at one sweep a second, a two-hour sleep, then ten more
        // seconds. Two targets per sweep must not double the count.
        let mut rows = Vec::new();
        for i in 0..=600 {
            for target in ["gateway", "cloudflare"] {
                rows.push((t - 7800.0 + i as f64, target.to_string(), Some(1.0), true));
            }
        }
        for i in 0..=10 {
            rows.push((t + i as f64, "gateway".to_string(), Some(1.0), true));
        }
        store.add_samples(&rows).unwrap();
        let watched = store.observed_seconds(t - 86_400.0, now(), OBSERVATION_GAP_S);
        assert!((watched - 610.0).abs() < 1e-6, "{watched}");
        assert_eq!(store.observed_seconds(t + 100.0, now(), OBSERVATION_GAP_S), 0.0);
    }

    #[test]
    fn renaming_an_outage_keeps_the_evidence_it_opened_with() {
        let store = Store::open_in_memory().unwrap();
        let id = store.open_event("degraded", "internet", "slow", r#"{"lead_up":[1]}"#).unwrap();
        let other = store.open_event("lan_down", "lan", "x", "{}").unwrap();
        store.set_event_kind(id, "isp_down", "isp", "router answers").unwrap();

        let events = store.events_since(3600.0);
        let renamed = events.iter().find(|e| e.id == id).unwrap();
        assert_eq!((renamed.kind.as_str(), renamed.scope.as_str()), ("isp_down", "isp"));
        assert_eq!(renamed.detail, "router answers");
        assert_eq!(store.event_context(id).unwrap().context, r#"{"lead_up":[1]}"#);
        let untouched = events.iter().find(|e| e.id == other).unwrap();
        assert_eq!(untouched.kind, "lan_down", "only the row asked for");
    }

    #[test]
    fn jitter_is_mean_consecutive_deviation() {
        let s = summarise(4, &[10.0, 20.0, 15.0, 15.0]);
        // |20-10| + |15-20| + |15-15| = 15, over 3 gaps
        assert!((s.jitter.unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(s.loss_pct, 0.0);
        assert_eq!(s.min, Some(10.0));
        assert_eq!(s.max, Some(20.0));
    }

    #[test]
    fn loss_counts_missing_samples_not_missing_rows() {
        let s = summarise(10, &[12.0; 8]);
        assert!((s.loss_pct - 20.0).abs() < 1e-9);
    }

    #[test]
    fn total_loss_reports_no_average_rather_than_zero() {
        let s = summarise(5, &[]);
        assert_eq!(s.loss_pct, 100.0);
        assert!(s.avg.is_none(), "an unreachable host has no average, not an average of zero");
    }

    /// The monitor's writes must not queue behind the UI's reads.
    ///
    /// The bug this guards is not in SQLite: WAL has always allowed a writer
    /// and a reader to work at once. It was one `Mutex<Connection>` in front
    /// of it, which made them take turns in Rust before SQLite was consulted,
    /// so a chart query over an hour of samples held the lock a sweep needed
    /// to record its four rows.
    ///
    /// Written as a deadline rather than a comparison: what matters is not
    /// that writes got faster but that the slowest one stays far inside the
    /// sweep interval it belongs to.
    #[test]
    fn a_reader_drawing_a_chart_does_not_hold_up_the_monitors_writes() {
        let path = std::env::temp_dir().join(format!("netdoctor-lock-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = std::sync::Arc::new(Store::open(&path).unwrap());

        // An hour of four targets: enough that a read over the whole window
        // is milliseconds of work rather than microseconds.
        let t0 = now() - 3600.0;
        let rows: Vec<(f64, String, Option<f64>, bool)> = (0..3600)
            .flat_map(|i| {
                ["gateway", "cloudflare", "google", "quad9"].into_iter().map(move |target| {
                    (t0 + i as f64, target.to_string(), Some(12.0 + (i % 7) as f64), true)
                })
            })
            .collect();
        store.add_samples(&rows).unwrap();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let drawing = {
            let (store, stop) = (std::sync::Arc::clone(&store), std::sync::Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut reads = 0u32;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = store.samples_between(t0, now());
                    reads += 1;
                }
                reads
            })
        };

        let mut took: Vec<std::time::Duration> = Vec::new();
        for i in 0..40 {
            let ts = now() + i as f64;
            let sweep = vec![(ts, "gateway".to_string(), Some(9.0), true)];
            let at = std::time::Instant::now();
            store.add_samples(&sweep).unwrap();
            took.push(at.elapsed());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        took.sort();
        let median = took[took.len() / 2];
        let worst = *took.last().expect("forty writes");
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let reads = drawing.join().unwrap();

        println!("median write {median:?}, worst {worst:?}, {reads} reads alongside");
        assert!(reads > 0, "the drawing thread has to have been reading throughout");
        // On the median rather than the maximum. Measured here: 0.4–0.9 ms
        // worst case with the connections split, 24–39 ms with the reads
        // forced back onto the writer's. A threshold on the maximum sat close
        // enough to the noise that a loaded machine could trip it — it did,
        // once, while the rest of the suite was running — and a test that
        // fails for being unlucky teaches people to rerun rather than to
        // look. Under a shared connection every write waits, so the median
        // moves just as decisively as the worst case does.
        assert!(
            median < std::time::Duration::from_millis(5),
            "the typical sweep waited {median:?} on the reader (worst {worst:?}); \
             the two share a connection again ({reads} reads ran alongside)"
        );

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    /// What a second instance does to the numbers.
    ///
    /// Two processes sweeping the same targets into one file interleave their
    /// rows, and `stats` reads the distance between *consecutive* rows as
    /// jitter. The second instance therefore does not merely duplicate the
    /// history — it reports a calmer line than the one being measured. This
    /// is the reason [`crate::single`] exists; the plan called it out without
    /// reproducing it, because reproducing it with two real processes means
    /// writing into the real history.
    #[test]
    fn a_second_writer_flatters_the_jitter_it_is_measuring() {
        let path = std::env::temp_dir().join(format!("netdoctor-two-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let one = Store::open(&path).unwrap();
        let t = now();
        // One sweep a second, alternating 5 ms and 45 ms: 40 ms of jitter.
        let alone: Vec<_> = (0..20)
            .map(|i| {
                (
                    t + i as f64,
                    "gateway".to_string(),
                    Some(if i % 2 == 0 { 5.0 } else { 45.0 }),
                    true,
                )
            })
            .collect();
        one.add_samples(&alone).unwrap();
        let honest = one.stats("gateway", 3600.0).jitter.unwrap();

        // A second instance, half a second out of step, recording the same
        // link and therefore the same readings.
        let _ = std::fs::remove_file(&path);
        let one = Store::open(&path).unwrap();
        let two = Store::open(&path).unwrap();
        one.add_samples(&alone).unwrap();
        let shadow: Vec<_> =
            alone.iter().map(|(ts, k, rtt, ok)| (ts + 0.5, k.clone(), *rtt, *ok)).collect();
        two.add_samples(&shadow).unwrap();
        let doubled = one.stats("gateway", 3600.0).jitter.unwrap();

        assert!(
            doubled < honest * 0.75,
            "a second writer has to visibly distort the reading: {honest} alone, {doubled} doubled"
        );

        drop((one, two));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn samples_and_events_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let t = now();
        store
            .add_samples(&[
                (t, "cloudflare".into(), Some(12.0), true),
                (t + 1.0, "cloudflare".into(), None, false),
                (t + 2.0, "cloudflare".into(), Some(14.0), true),
            ])
            .unwrap();
        let s = store.stats("cloudflare", 3600.0);
        assert_eq!(s.count, 3);
        assert!((s.loss_pct - 33.333).abs() < 0.01);

        let id = store
            .open_event("lan_down", "lan", "router stopped answering", r#"{"rssi_dbm":-78}"#)
            .unwrap();
        store.close_event(id, r#"{"rssi_dbm":-52}"#).unwrap();
        let events = store.recent_events(10);
        assert_eq!(events.len(), 1);
        assert!(events[0].duration_s().is_some());
        assert_eq!(events[0].scope, "lan");
    }

    #[test]
    fn stored_context_survives_the_round_trip() {
        // The bug this guards: the context column was written on every outage
        // and read back by nothing, so the richest evidence the app collected
        // was invisible to it.
        let store = Store::open_in_memory().unwrap();
        let id = store
            .open_event("lan_down", "lan", "note", r#"{"rssi_dbm":-81,"ssid":"home"}"#)
            .unwrap();
        store.close_event(id, r#"{"rssi_dbm":-55}"#).unwrap();

        let e = &store.recent_events(1)[0];
        assert!(e.id > 0, "the row id is needed to select an outage in the UI");
        let ctx = store.event_context(e.id).expect("the row is there");
        let start = ctx.context_json().expect("opening context parses");
        assert_eq!(start["rssi_dbm"], -81);
        assert_eq!(start["ssid"], "home");
        assert_eq!(ctx.context_end_json().unwrap()["rssi_dbm"], -55);
    }

    #[test]
    fn a_database_from_an_older_build_gains_the_new_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, ts_start REAL NOT NULL,
             ts_end REAL, kind TEXT NOT NULL, scope TEXT NOT NULL, detail TEXT);
             INSERT INTO events (ts_start, kind, scope, detail) VALUES (1.0,'lan_down','lan','x');",
        )
        .unwrap();
        migrate(&conn);

        let store = Store { conn: Mutex::new(conn), read: None };
        let events = store.recent_events(10);
        assert_eq!(events.len(), 1, "the old row must still be readable");
        let ctx = store.event_context(events[0].id).expect("the old row is still there");
        assert_eq!(ctx.context, "", "an old row simply has no context");
    }

    #[test]
    fn samples_and_tweaks_are_fetched_by_span() {
        let store = Store::open_in_memory().unwrap();
        let t = now();
        store
            .add_samples(&[
                (t - 100.0, "gateway".into(), Some(3.0), true),
                (t - 10.0, "gateway".into(), None, false),
                (t + 500.0, "gateway".into(), Some(4.0), true),
            ])
            .unwrap();
        store.log_tweak("adapter_power", "apply", "on", "ok");

        let span = store.samples_between(t - 60.0, t + 60.0);
        assert_eq!(span.len(), 1, "only the sample inside the window");
        assert!(!span[0].3, "and it is the failed one");
        assert_eq!(store.tweaks_between(t - 60.0, t + 60.0).len(), 1);
        assert!(store.tweaks_between(t + 3600.0, t + 7200.0).is_empty());
    }

    #[test]
    fn the_median_ignores_the_spike_the_mean_absorbs() {
        // Nineteen ordinary readings and one bad one. "What is normal for
        // this line" is 10 ms; the mean says 59 and would raise the bar high
        // enough to hide the next fault for a week.
        let mut rtts = vec![10.0; 19];
        rtts.push(1_000.0);
        let s = summarise(rtts.len(), &rtts);
        assert_eq!(s.median, Some(10.0));
        assert!(s.avg.unwrap() > 55.0, "the mean is exactly the problem");
    }

    #[test]
    fn the_median_averages_the_middle_pair_on_an_even_count() {
        let s = summarise(4, &[4.0, 1.0, 3.0, 2.0]);
        assert_eq!(s.median, Some(2.5), "and the caller's order is not relied on");
    }

    #[test]
    fn a_fresh_database_has_no_observation_to_report() {
        let store = Store::open_in_memory().unwrap();
        assert!(
            store.observing_since(OBSERVATION_GAP_S).is_none(),
            "an empty history is not a clean history"
        );
        assert!(store.last_sample_ts().is_none());
    }

    #[test]
    fn observation_restarts_on_the_far_side_of_a_gap() {
        // Watched for an hour, closed for two, watched for five minutes. The
        // card may only claim the five minutes: nothing was being measured in
        // between, so the hour proves nothing about the line now.
        let store = Store::open_in_memory().unwrap();
        let t = now();
        let mut rows = Vec::new();
        for i in 0..60 {
            rows.push((t - 10_800.0 + f64::from(i) * 60.0, "x".into(), Some(1.0), true));
        }
        let resumed = t - 300.0;
        for i in 0..6 {
            rows.push((resumed + f64::from(i) * 60.0, "x".into(), Some(1.0), true));
        }
        store.add_samples(&rows).unwrap();

        let since = store.observing_since(OBSERVATION_GAP_S).expect("there is a stretch to report");
        assert!(
            (since - resumed).abs() < 1.0,
            "observation starts at {since}, expected the far side of the gap at {resumed}"
        );
    }

    #[test]
    fn an_unbroken_history_counts_from_its_start() {
        // Sweeps every 30 s, well inside the gap threshold: one stretch.
        let store = Store::open_in_memory().unwrap();
        let t = now();
        let start = t - 3600.0;
        let rows: Vec<_> = (0..120)
            .map(|i| (start + f64::from(i) * 30.0, "x".to_string(), Some(1.0), true))
            .collect();
        store.add_samples(&rows).unwrap();

        let since = store.observing_since(OBSERVATION_GAP_S).expect("there is a stretch to report");
        assert!((since - start).abs() < 1.0, "observation starts at {since}, expected {start}");
    }

    #[test]
    fn an_outage_left_open_by_a_crash_is_closed_at_startup() {
        // `run_loop` closes its event on a clean exit, so a row still open
        // when the app starts means the last run was killed mid-outage.
        // Left alone it is never pruned, and the history tab asks the event
        // log for everything between its start and now.
        let store = Store::open_in_memory().unwrap();
        let t = now();
        store.add_samples(&[(t - 600.0, "x".into(), Some(1.0), true)]).unwrap();
        let orphan = store.open_event("outage", "lan", "killed mid-outage", "{}").unwrap();

        store.close_orphans();

        let row = store.recent_events(10).into_iter().find(|e| e.id == orphan).unwrap();
        let end = row.ts_end.expect("the orphan must have been given an end");
        assert!(end >= row.ts_start, "an outage cannot end before it started");
        let ctx = store.event_context(row.id).expect("the orphan row is still there");
        assert!(
            ctx.context_end.unwrap().contains("restart"),
            "the end was inferred, and the row has to say so"
        );
    }

    #[test]
    fn a_dropped_store_leaves_files_that_can_be_deleted() {
        // What the live outage test relies on to clean up after itself:
        // Windows will not delete an open file, and WAL adds two beside it.
        let db = std::env::temp_dir().join(format!("netdoctor-drop-{}.db", std::process::id()));
        let store = Store::open(&db).unwrap();
        store.add_samples(&[(now(), "x".into(), Some(1.0), true)]).unwrap();
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let mut path = db.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(&path);
            assert!(!std::path::Path::new(&path).exists(), "{path:?} is still there");
        }
    }

    #[test]
    fn an_outage_cut_short_by_sleep_ends_at_the_last_sweep_not_at_wake() {
        let store = Store::open_in_memory().unwrap();
        let id = store.open_event("isp_down", "isp", "", "{}").unwrap();
        let start = store.recent_events(1)[0].ts_start;

        // The machine slept thirty seconds in; it woke hours later.
        store.close_event_at(id, start + 30.0, r#"{"closed_by":"gap"}"#).unwrap();
        let row = store.recent_events(1).remove(0);
        assert_eq!(row.duration_s(), Some(30.0));

        // A last sweep from before the outage opened cannot make it negative.
        let id = store.open_event("isp_down", "isp", "", "{}").unwrap();
        store.close_event_at(id, 0.0, "{}").unwrap();
        let row = store.recent_events(1).remove(0);
        assert_eq!(row.duration_s(), Some(0.0));
    }

    #[test]
    fn closing_orphans_leaves_a_genuinely_running_outage_alone() {
        let store = Store::open_in_memory().unwrap();
        let closed = store.open_event("outage", "lan", "done", "{}").unwrap();
        store.close_event(closed, "{}").unwrap();
        store.close_orphans();

        // A fresh outage opened after the reconciliation stays open.
        let running = store.open_event("outage", "lan", "now", "{}").unwrap();
        let row = store.recent_events(10).into_iter().find(|e| e.id == running).unwrap();
        assert!(row.ts_end.is_none());
    }

    #[test]
    fn prune_drops_old_samples() {
        let store = Store::open_in_memory().unwrap();
        let t = now();
        store
            .add_samples(&[
                (t - 40.0 * 86400.0, "x".into(), Some(1.0), true),
                (t, "x".into(), Some(2.0), true),
            ])
            .unwrap();
        store.prune(14).unwrap();
        assert_eq!(store.stats("x", 100.0 * 86400.0).count, 1);
    }

    #[test]
    fn prune_drops_closed_events_but_keeps_the_open_one() {
        let store = Store::open_in_memory().unwrap();
        // Past the outage retention, not merely the sample retention: see
        // `outages_outlive_the_samples_but_not_their_lead_up`.
        let old = now() - (OUTAGE_KEEP_DAYS + 35) as f64 * 86400.0;

        let closed = store.open_event("outage", "lan", "old", "{}").unwrap();
        store.close_event(closed, "{}").unwrap();
        let still_open = store.open_event("outage", "lan", "running", "{}").unwrap();
        // `open_event` stamps ts_start with `now()`, so age it by hand.
        {
            let conn = store.conn.lock().unwrap();
            conn.execute("UPDATE events SET ts_start = ?", params![old]).unwrap();
        }

        store.prune(14).unwrap();

        let left = store.recent_events(10);
        assert_eq!(left.len(), 1, "the closed one goes, the open one stays");
        assert_eq!(left[0].id, still_open);
    }

    #[test]
    fn clearing_the_history_spares_the_outage_still_running_and_the_samples() {
        let store = Store::open_in_memory().unwrap();
        for _ in 0..3 {
            let id = store.open_event("outage", "isp", "", "{}").unwrap();
            store.close_event(id, "{}").unwrap();
        }
        let running = store.open_event("outage", "lan", "", "{}").unwrap();
        store.add_samples(&[(now(), "gateway".into(), Some(3.0), true)]).unwrap();

        assert_eq!(store.clear_events().unwrap(), 3);

        let left = store.recent_events(10);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, running);
        // The monitor can still close the one it holds.
        store.close_event(running, "{}").unwrap();
        assert!(store.recent_events(10)[0].ts_end.is_some());
        assert_eq!(store.samples_between(now() - 60.0, now() + 1.0).len(), 1);
    }

    /// A context the size the monitor really writes: 180 lead-up sweeps plus
    /// the recovery state, measured at 33 398 bytes on a live database.
    fn fat_context() -> String {
        // Built from the real struct rather than hand-written JSON, so the
        // fixture keeps its size if a field is ever added to a sweep.
        let lead: Vec<crate::monitor::LeadSample> = (0..180)
            .map(|i| crate::monitor::LeadSample {
                ts: 1_800_000_000.0 + i as f64,
                status: "degraded".into(),
                up: true,
                bssid: format!("aa:bb:cc:dd:ee:{:02x}", i % 256),
                signal_pct: Some(62),
                rssi_dbm: Some(-(50 + (i % 30))),
                channel: Some(36),
                rx_mbps: Some(390),
                gateway_ms: Some(12.5),
                internet_ms: Some(24.75),
                internet_ok: true,
            })
            .collect();
        serde_json::json!({
            "medium": "wifi",
            "ssid": "home",
            "rssi_dbm": -72,
            "channel": 36,
            "up": true,
            "lead_up": lead,
        })
        .to_string()
    }

    /// The list queries must not drag the context along.
    ///
    /// Measured before the split: `events_since(24h)` over 200 outages took
    /// 21.56 ms and moved 13.4 MB, because `EVENT_COLUMNS` named `context` and
    /// `context_end` and the History tab runs the query every frame. Without
    /// those two columns the same query took 199 µs.
    ///
    /// The threshold is 5 ms rather than 1 ms so a loaded CI box cannot fail
    /// it by being slow; the regression it guards is 20x over budget, so the
    /// margin costs nothing.
    #[test]
    fn listing_events_does_not_carry_their_context() {
        let store = Store::open_in_memory().unwrap();
        let ctx = fat_context();
        assert!(ctx.len() > 30_000, "the fixture has to be the size of a real context");
        for _ in 0..200 {
            let id = store.open_event("lan_down", "lan", "router stopped answering", &ctx).unwrap();
            store.close_event(id, r#"{"rssi_dbm":-52}"#).unwrap();
        }

        let started = std::time::Instant::now();
        let events = store.events_since(24.0 * 3600.0);
        let elapsed = started.elapsed();
        assert_eq!(events.len(), 200);
        assert!(
            elapsed < std::time::Duration::from_millis(5),
            "listing 200 events took {elapsed:?}; the context columns are back in the list query"
        );

        // The context is still reachable, one row at a time.
        let ctx =
            store.event_context(events[0].id).expect("the selected row still has its context");
        assert!(ctx.context_json().is_some());
        assert!(ctx.context_end_json().is_some());
    }
}
