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
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_start REAL NOT NULL,
    ts_end   REAL,
    kind     TEXT NOT NULL,
    scope    TEXT NOT NULL,
    detail   TEXT,
    context  TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_start ON events(ts_start);

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
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub jitter: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Event {
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
        Ok(Store { conn: Mutex::new(conn) })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn add_samples(&self, rows: &[(f64, String, Option<f64>, bool)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap();
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

    pub fn stats(&self, target: &str, window_s: f64) -> Stats {
        let cutoff = now() - window_s;
        let conn = self.conn.lock().unwrap();
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

    pub fn prune(&self, keep_days: i64) -> Result<()> {
        let cutoff = now() - (keep_days.max(1) as f64) * 86400.0;
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM samples WHERE ts < ?", params![cutoff])?;
        Ok(())
    }

    pub fn open_event(&self, kind: &str, scope: &str, detail: &str, context: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (ts_start, kind, scope, detail, context) VALUES (?,?,?,?,?)",
            params![now(), kind, scope, detail, context],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn close_event(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE events SET ts_end=? WHERE id=?", params![now(), id])?;
        Ok(())
    }

    pub fn events_since(&self, window_s: f64) -> Vec<Event> {
        self.query_events("WHERE ts_start >= ? ORDER BY ts_start DESC", Some(now() - window_s))
    }

    pub fn recent_events(&self, limit: usize) -> Vec<Event> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT id, ts_start, ts_end, kind, scope, COALESCE(detail,'') FROM events \
             ORDER BY ts_start DESC LIMIT {limit}"
        );
        let Ok(mut stmt) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map([], map_event);
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    fn query_events(&self, tail: &str, arg: Option<f64>) -> Vec<Event> {
        let conn = self.conn.lock().unwrap();
        let sql =
            format!("SELECT id, ts_start, ts_end, kind, scope, COALESCE(detail,'') FROM events {tail}");
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
        let conn = self.conn.lock().unwrap();
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

fn map_event(r: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        ts_start: r.get(1)?,
        ts_end: r.get(2)?,
        kind: r.get(3)?,
        scope: r.get(4)?,
        detail: r.get(5)?,
    })
}

/// Loss, average, extremes and mean consecutive deviation (jitter).
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
    Stats { count: total, loss_pct, avg: Some(avg), min: Some(min), max: Some(max), jitter }
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

        let id = store.open_event("lan_down", "lan", "router stopped answering", "{}").unwrap();
        store.close_event(id).unwrap();
        let events = store.recent_events(10);
        assert_eq!(events.len(), 1);
        assert!(events[0].duration_s().is_some());
        assert_eq!(events[0].scope, "lan");
    }

    #[test]
    fn prune_drops_old_samples_only() {
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
}
