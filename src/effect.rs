//! Whether a change helped: the line in the day before a tweak was applied,
//! set against the time since.
//!
//! The figures are shown side by side and nothing more is claimed. A day is
//! long enough to hold an evening's congestion on one side and not the
//! other, and another change may have landed in the same window, so the app
//! says what was measured and leaves "because of the tweak" unsaid.

use crate::diagnose::{ANCHOR_ALT_KEY, ANCHOR_KEY};
use crate::store::{Store, TweakLogRow};
use std::collections::HashMap;

/// How far each side reaches: a day before, and up to a day after.
pub const WINDOW_S: f64 = 86_400.0;

/// Fewer readings than this on a side, and the side is "not enough data":
/// ten minutes at the default one-second cadence.
const MIN_SAMPLES: usize = 600;

/// The line over one stretch, read the way the headline reads it: each
/// figure from whichever public anchor shows the least of it, so one
/// filtered address does not stand in for the line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Side {
    pub median_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: f64,
    /// How long the stretch was, in seconds. The side after a change made an
    /// hour ago covers an hour, not a day.
    pub span_s: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    /// When the change that is being judged was applied.
    pub applied: f64,
    /// `None` when that side has too few readings to say anything.
    pub before: Option<Side>,
    pub after: Option<Side>,
}

/// The effect of the latest successful apply of `tweak_id`, or `None` when
/// the log has no such apply. `log` is the tweak log in any order.
///
/// The side after ends at a revert of the same tweak, if one came later: past
/// that point the machine is back as it was and the figures say nothing
/// about the change.
#[cfg(test)]
pub fn of(store: &Store, log: &[TweakLogRow], tweak_id: &str, now: f64) -> Option<Effect> {
    of_with(log, tweak_id, now, |from, to| side(store, from, to))
}

/// Sides already read, by the stretch they cover.
///
/// Reading a side is a day of samples on two anchors, and the Optimise tab
/// asked for every applied tweak at start-up, on every visit and after every
/// change: 44 queries and 1.9 million rows on a week-old history. A stretch
/// that has ended never reads differently, so each is read once.
#[derive(Default)]
pub struct Cache {
    sides: HashMap<(u64, u64), Option<Side>>,
}

/// The side after a change is still growing for a day. It is read up to the
/// last whole step of this length, so asking again within the step is the
/// same stretch and costs nothing. `span_s` says how far it reaches.
const AFTER_STEP_S: f64 = 300.0;

/// Beyond this many stretches the cache starts over.
// ponytail: a growing side leaves one stale entry per step until its day is
// up; clearing wholesale bounds that without tracking which are still asked
// for. An LRU would keep the hot ones across the reset if it ever shows.
const CACHE_MAX: usize = 512;

impl Cache {
    fn side(&mut self, store: &Store, from: f64, to: f64) -> Option<Side> {
        if self.sides.len() >= CACHE_MAX {
            self.sides.clear();
        }
        *self.sides.entry((from.to_bits(), to.to_bits())).or_insert_with(|| side(store, from, to))
    }
}

/// [`of`] through `cache`, with the side after read up to the last
/// [`AFTER_STEP_S`] boundary rather than up to `now`.
pub fn of_cached(
    store: &Store,
    log: &[TweakLogRow],
    tweak_id: &str,
    now: f64,
    cache: &mut Cache,
) -> Option<Effect> {
    let now = (now / AFTER_STEP_S).floor() * AFTER_STEP_S;
    of_with(log, tweak_id, now, |from, to| cache.side(store, from, to))
}

fn of_with(
    log: &[TweakLogRow],
    tweak_id: &str,
    now: f64,
    mut side: impl FnMut(f64, f64) -> Option<Side>,
) -> Option<Effect> {
    let applied = log
        .iter()
        .filter(|r| r.tweak_id == tweak_id && r.action == "apply")
        .map(|r| r.ts)
        .fold(None, |best: Option<f64>, ts| Some(best.map_or(ts, |b| b.max(ts))))?;
    let reverted = log
        .iter()
        .filter(|r| r.tweak_id == tweak_id && r.action == "revert" && r.ts > applied)
        .map(|r| r.ts)
        .fold(f64::INFINITY, f64::min);
    let end = (applied + WINDOW_S).min(reverted).min(now);
    Some(Effect { applied, before: side(applied - WINDOW_S, applied), after: side(applied, end) })
}

fn side(store: &Store, from: f64, to: f64) -> Option<Side> {
    let judged: Vec<_> = [ANCHOR_KEY, ANCHOR_ALT_KEY]
        .iter()
        .map(|key| store.stats_between(key, from, to))
        .filter(|s| s.count >= MIN_SAMPLES)
        .collect();
    if judged.is_empty() {
        return None;
    }
    let lowest = |v: Vec<f64>| v.into_iter().reduce(f64::min);
    Some(Side {
        median_ms: lowest(judged.iter().filter_map(|s| s.median).collect()),
        jitter_ms: lowest(judged.iter().filter_map(|s| s.jitter).collect()),
        loss_pct: lowest(judged.iter().map(|s| s.loss_pct).collect()).unwrap_or(0.0),
        span_s: to - from,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ts: f64, action: &str) -> TweakLogRow {
        TweakLogRow { ts, tweak_id: "nagle".into(), action: action.into(), result: String::new() }
    }

    /// A reading a second on both anchors from `from` for `n` seconds.
    fn fill(store: &Store, from: f64, n: usize, rtt: f64, lose_every: usize) {
        let mut rows = Vec::new();
        for i in 0..n {
            let ok = lose_every == 0 || i % lose_every != 0;
            let ts = from + i as f64;
            for key in [ANCHOR_KEY, ANCHOR_ALT_KEY] {
                rows.push((ts, key.to_string(), ok.then_some(rtt), ok));
            }
        }
        store.add_samples(&rows).unwrap();
    }

    #[test]
    fn the_day_before_and_the_time_since_are_read_apart() {
        let store = Store::open_in_memory().unwrap();
        let applied = 100_000.0;
        fill(&store, applied - 1_000.0, 1_000, 40.0, 10);
        fill(&store, applied, 900, 20.0, 0);
        let e = of(&store, &[row(applied, "apply")], "nagle", applied + 900.0).unwrap();
        let (before, after) = (e.before.unwrap(), e.after.unwrap());
        assert_eq!(before.median_ms, Some(40.0));
        assert!((before.loss_pct - 10.0).abs() < 0.01, "{}", before.loss_pct);
        assert_eq!(after.median_ms, Some(20.0));
        assert_eq!(after.loss_pct, 0.0);
        assert_eq!(after.span_s, 900.0, "the side after covers only the time since");
    }

    #[test]
    fn a_stretch_is_read_once_and_the_side_after_stops_at_a_step() {
        let store = Store::open_in_memory().unwrap();
        let applied = 100_020.0;
        fill(&store, applied - 1_000.0, 1_000, 40.0, 0);
        fill(&store, applied, 1_000, 20.0, 0);
        let log = [row(applied, "apply")];
        let mut cache = Cache::default();
        let now = applied + 950.0;
        let first = of_cached(&store, &log, "nagle", now, &mut cache).unwrap();
        // Up to the last step boundary, and saying so.
        let step_end = (now / AFTER_STEP_S).floor() * AFTER_STEP_S;
        assert_eq!(first.after.unwrap().span_s, step_end - applied);
        assert_eq!(first.before, of(&store, &log, "nagle", now).unwrap().before);

        // Rows that land in a stretch already read do not change it: the
        // answer came from the cache, not from another query.
        fill(&store, applied - 500.5, 400, 90.0, 0);
        let again = of_cached(&store, &log, "nagle", now + 10.0, &mut cache).unwrap();
        assert_eq!(again, first);
    }

    #[test]
    fn too_few_readings_is_said_rather_than_judged() {
        let store = Store::open_in_memory().unwrap();
        let applied = 100_000.0;
        fill(&store, applied - 100.0, 100, 40.0, 0);
        let e = of(&store, &[row(applied, "apply")], "nagle", applied + 50.0).unwrap();
        assert_eq!((e.before, e.after), (None, None));
    }

    #[test]
    fn only_a_change_that_took_effect_is_judged_and_a_revert_ends_it() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(of(&store, &[row(5.0, "apply_failed")], "nagle", 10.0), None);
        assert_eq!(of(&store, &[row(5.0, "apply")], "mtu", 10.0), None, "another tweak");

        let applied = 100_000.0;
        fill(&store, applied, 700, 20.0, 0);
        // Back as it was after ten minutes, then a bad hour that is not the
        // change's doing.
        fill(&store, applied + 700.0, 3_000, 90.0, 0);
        let log = [row(applied, "apply"), row(applied + 700.0, "revert")];
        let after = of(&store, &log, "nagle", applied + 3_700.0).unwrap().after.unwrap();
        assert_eq!(after.median_ms, Some(20.0));
        assert_eq!(after.span_s, 700.0);
    }
}
