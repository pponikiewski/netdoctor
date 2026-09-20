//! SQLite history: every sample, every outage, every applied tweak.
//!
//! Samples are buffered and flushed in batches — at one sweep per second
//! across four targets a transaction per sample would hammer the disk for no
//! reason.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::settings;

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

CREATE TABLE IF NOT EXISTS tweaks (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    ts       REAL NOT NULL,
    tweak_id TEXT NOT NULL,
    action   TEXT NOT NULL,
    before   TEXT,
    result   TEXT
);
"#;

#[derive(Debug, Clone, Default)]
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

#[derive(Debug, Clone)]
pub struct Event {
    pub id: i64,
    pub ts_start: f64,
    pub ts_end: Option<f64>,
    pub kind: String,
    pub scope: String,
    pub detail: String,
    /// Connection state as JSON at the moment the outage opened, including the
    /// `lead_up` series of sweeps that preceded it. This is the evidence the
    /// cause analysis reads; an empty string means the row predates it.
    pub context: String,
    /// Connection state as JSON at the moment the connection came back.
    pub context_end: Option<String>,
}

impl Event {
    pub fn duration_s(&self) -> Option<f64> {
        self.ts_end.map(|e| e - self.ts_start)
    }

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
    conn: Mutex<Connection>,
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

    pub fn open_default() -> Result<Self> {
        std::fs::create_dir_all(settings::data_dir())?;
        Self::open(settings::db_path())
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        // WAL keeps the writer from blocking the UI thread's reads.
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn);
        let store = Store { conn: Mutex::new(conn) };
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
        Ok(Store { conn: Mutex::new(conn) })
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
        let cutoff = now() - window_s;
        let conn = self.held();
        let mut stmt = match conn
            .prepare_cached("SELECT rtt_ms, ok FROM samples WHERE target=? AND ts>=? ORDER BY ts")
        {
            Ok(s) => s,
            Err(_) => return Stats::default(),
        };
        let rows = stmt.query_map(params![target, cutoff], |r| {
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

    /// Applies the retention setting to both sample and event history.
    ///
    /// Events used to be exempt, which sounded conservative and was not: each
    /// one carries its `context` and `context_end` JSON — the whole lead-up
    /// series — so on a machine that autostarts and runs all day they are the
    /// bulk of the file, and they grew without limit no matter what the user
    /// set. The tweak log is deliberately left alone: it is small, and it is
    /// the record of what this app changed on the machine.
    pub fn prune(&self, keep_days: i64) -> Result<()> {
        let cutoff = now() - (keep_days.max(1) as f64) * 86400.0;
        let conn = self.held();
        conn.execute("DELETE FROM samples WHERE ts < ?", params![cutoff])?;
        // An outage still open has no end yet and is never old enough to
        // drop. `close_orphans` runs at startup so this only ever spares one
        // that is genuinely still running, rather than every row a crash
        // left behind.
        conn.execute(
            "DELETE FROM events WHERE ts_start < ? AND ts_end IS NOT NULL",
            params![cutoff],
        )?;
        Ok(())
    }

    pub fn open_event(&self, kind: &str, scope: &str, detail: &str, context: &str) -> Result<i64> {
        let conn = self.held();
        conn.execute(
            "INSERT INTO events (ts_start, kind, scope, detail, context) VALUES (?,?,?,?,?)",
            params![now(), kind, scope, detail, context],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Closes an outage, recording the state it recovered *into*. The pair of
    /// contexts is what separates "the signal came back" from "the adapter was
    /// reset" — the recovery is as diagnostic as the failure.
    pub fn close_event(&self, id: i64, context_end: &str) -> Result<()> {
        let conn = self.held();
        conn.execute(
            "UPDATE events SET ts_end=?, context_end=? WHERE id=?",
            params![now(), context_end, id],
        )?;
        Ok(())
    }

    pub fn events_since(&self, window_s: f64) -> Vec<Event> {
        self.query_events("WHERE ts_start >= ? ORDER BY ts_start DESC", Some(now() - window_s))
    }

    /// Every sample for every target inside a time span, oldest first. This is
    /// what draws an outage's own timeline instead of a rolling live window.
    pub fn samples_between(&self, from: f64, to: f64) -> Vec<(f64, String, Option<f64>, bool)> {
        let conn = self.held();
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
        let conn = self.held();
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

    pub fn recent_events(&self, limit: usize) -> Vec<Event> {
        let conn = self.held();
        let sql =
            format!("SELECT {EVENT_COLUMNS} FROM events ORDER BY ts_start DESC LIMIT {limit}");
        let Ok(mut stmt) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map([], map_event);
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    fn query_events(&self, tail: &str, arg: Option<f64>) -> Vec<Event> {
        let conn = self.held();
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
        let conn = self.held();
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

/// The column order every event query uses.
const EVENT_COLUMNS: &str =
    "id, ts_start, ts_end, kind, scope, COALESCE(detail,''), COALESCE(context,''), context_end";

fn map_event(r: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        ts_start: r.get(1)?,
        ts_end: r.get(2)?,
        kind: r.get(3)?,
        scope: r.get(4)?,
        detail: r.get(5)?,
        context: r.get(6)?,
        context_end: r.get(7)?,
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
        let start = e.context_json().expect("opening context parses");
        assert_eq!(start["rssi_dbm"], -81);
        assert_eq!(start["ssid"], "home");
        assert_eq!(e.context_end_json().unwrap()["rssi_dbm"], -55);
        assert!(e.id > 0, "the row id is needed to select an outage in the UI");
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

        let store = Store { conn: Mutex::new(conn) };
        let events = store.recent_events(10);
        assert_eq!(events.len(), 1, "the old row must still be readable");
        assert_eq!(events[0].context, "", "an old row simply has no context");
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
        assert!(
            row.context_end.unwrap().contains("restart"),
            "the end was inferred, and the row has to say so"
        );
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
        let old = now() - 40.0 * 86400.0;

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
}
