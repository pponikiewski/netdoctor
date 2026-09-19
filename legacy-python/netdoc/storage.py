"""SQLite history: every probe sweep, every outage, every applied tweak.

The GUI reads live data from memory; this store exists so that a dropout at
3 a.m. is still explainable the next morning.
"""

import json
import sqlite3
import threading
import time

from . import config

_SCHEMA = """
CREATE TABLE IF NOT EXISTS samples (
    ts        REAL NOT NULL,
    target    TEXT NOT NULL,
    rtt_ms    REAL,
    ok        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_samples_ts ON samples(ts);

CREATE TABLE IF NOT EXISTS events (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_start  REAL NOT NULL,
    ts_end    REAL,
    kind      TEXT NOT NULL,      -- outage | degraded | dns_fail | wifi_drop
    scope     TEXT NOT NULL,      -- lan | isp | internet | dns | adapter
    detail    TEXT,
    context   TEXT                -- JSON snapshot of the connection state
);

CREATE TABLE IF NOT EXISTS tweaks (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    ts        REAL NOT NULL,
    tweak_id  TEXT NOT NULL,
    action    TEXT NOT NULL,      -- apply | revert
    before    TEXT,
    result    TEXT
);
"""


class Store:
    def __init__(self, path=config.DB_PATH):
        path.parent.mkdir(parents=True, exist_ok=True)
        self.path = path
        self._lock = threading.Lock()
        self._db = sqlite3.connect(str(path), check_same_thread=False)
        self._db.executescript(_SCHEMA)
        self._db.commit()

    def close(self):
        with self._lock:
            self._db.close()

    # -- samples ----------------------------------------------------------
    def add_samples(self, rows):
        """rows: iterable of (ts, target, rtt_ms, ok)."""
        with self._lock:
            self._db.executemany(
                "INSERT INTO samples (ts, target, rtt_ms, ok) VALUES (?,?,?,?)", rows
            )
            self._db.commit()

    def stats(self, target, since_s):
        """(count, loss_pct, avg, min, max, jitter) over the last since_s seconds."""
        cutoff = time.time() - since_s
        with self._lock:
            cur = self._db.execute(
                "SELECT rtt_ms, ok FROM samples WHERE target=? AND ts>=? ORDER BY ts",
                (target, cutoff),
            )
            rows = cur.fetchall()
        if not rows:
            return (0, 0.0, None, None, None, None)
        total = len(rows)
        rtts = [r[0] for r in rows if r[1] and r[0] is not None]
        lost = total - len(rtts)
        loss = lost / total * 100
        if not rtts:
            return (total, loss, None, None, None, None)
        avg = sum(rtts) / len(rtts)
        jitter = None
        if len(rtts) > 1:
            diffs = [abs(rtts[i] - rtts[i - 1]) for i in range(1, len(rtts))]
            jitter = sum(diffs) / len(diffs)
        return (total, loss, avg, min(rtts), max(rtts), jitter)

    def prune(self, keep_days=14):
        cutoff = time.time() - keep_days * 86400
        with self._lock:
            self._db.execute("DELETE FROM samples WHERE ts < ?", (cutoff,))
            self._db.commit()

    # -- events -----------------------------------------------------------
    def open_event(self, kind, scope, detail, context=None):
        with self._lock:
            cur = self._db.execute(
                "INSERT INTO events (ts_start, kind, scope, detail, context)"
                " VALUES (?,?,?,?,?)",
                (time.time(), kind, scope, detail, json.dumps(context or {}, default=str)),
            )
            self._db.commit()
            return cur.lastrowid

    def close_event(self, event_id, detail=None):
        with self._lock:
            if detail is None:
                self._db.execute(
                    "UPDATE events SET ts_end=? WHERE id=?", (time.time(), event_id)
                )
            else:
                self._db.execute(
                    "UPDATE events SET ts_end=?, detail=? WHERE id=?",
                    (time.time(), detail, event_id),
                )
            self._db.commit()

    def recent_events(self, limit=100):
        with self._lock:
            cur = self._db.execute(
                "SELECT id, ts_start, ts_end, kind, scope, detail, context"
                " FROM events ORDER BY ts_start DESC LIMIT ?",
                (limit,),
            )
            return cur.fetchall()

    def events_since(self, since_s):
        cutoff = time.time() - since_s
        with self._lock:
            cur = self._db.execute(
                "SELECT id, ts_start, ts_end, kind, scope, detail FROM events"
                " WHERE ts_start >= ? ORDER BY ts_start",
                (cutoff,),
            )
            return cur.fetchall()

    # -- tweaks -----------------------------------------------------------
    def log_tweak(self, tweak_id, action, before, result):
        with self._lock:
            self._db.execute(
                "INSERT INTO tweaks (ts, tweak_id, action, before, result) VALUES (?,?,?,?,?)",
                (time.time(), tweak_id, action, json.dumps(before, default=str), result),
            )
            self._db.commit()

    def tweak_log(self, limit=50):
        with self._lock:
            cur = self._db.execute(
                "SELECT ts, tweak_id, action, result FROM tweaks ORDER BY ts DESC LIMIT ?",
                (limit,),
            )
            return cur.fetchall()
