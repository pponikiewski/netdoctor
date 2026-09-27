//! The recorded history added up over a period: how long the connection was
//! down, how often it broke and on which link, how often it was only slow,
//! what the pings looked like, and what the router said about itself. Read by
//! the statistics tab and by the report.
//!
//! It adds up only what was recorded. Every share is a share of the time
//! somebody watched, never of the calendar, and a period longer than the
//! samples are kept is measured over the part that is left, and says so.

use crate::cause::ROUTER_CLOCK_SLACK_S;
use crate::monitor::Status;
use crate::probe::igd::RouterReading;
use crate::store::{self, Event, PingTotals, Store};
use std::sync::Arc;

const DAY_S: f64 = 86_400.0;

/// Less watching than this and a share of it is noise: one ten-second drop in
/// a minute of watching reads as 83 %.
pub const MIN_WATCHED_S: f64 = 600.0;

/// An hour of the day watched for less than this, over the whole period, gets
/// no bar: one bad evening would be its whole story.
pub const MIN_HOUR_WATCHED_S: f64 = 1800.0;

/// The events of a period, added up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tally {
    pub breaks: usize,
    pub down_s: f64,
    pub longest_s: f64,
    pub slow: usize,
    pub slow_s: f64,
    /// Per stored kind: how many, and for how long in total. Longest first.
    pub by_kind: Vec<(String, usize, f64)>,
    /// A break is still open at the end of the period.
    pub ongoing: bool,
}

/// One bar of the "when" chart: a local hour or a local day.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bucket {
    pub from: f64,
    pub to: f64,
    pub down_s: f64,
    pub slow_s: f64,
    /// "14:00–15:00" or "27.09": what hovering the bar says.
    pub label: String,
    /// "14:00" or "27.09": what the axis says under it.
    pub tick: String,
}

/// One hour of the day across the whole period: how long it was watched, and
/// how much of that was down or slow.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HourShare {
    pub watched_s: f64,
    pub down_s: f64,
    pub slow_s: f64,
}

impl HourShare {
    /// Percent of the watched time down and slow, or `None` when the hour was
    /// watched too little to say.
    pub fn shares(&self) -> Option<(f64, f64)> {
        (self.watched_s >= MIN_HOUR_WATCHED_S).then(|| {
            let pct = |s: f64| (s / self.watched_s * 100.0).clamp(0.0, 100.0);
            (pct(self.down_s), pct(self.slow_s))
        })
    }
}

/// Pings to one target over the measured part of the period.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ping {
    pub totals: PingTotals,
    pub p95: Option<f64>,
    /// Sent and lost outside the drops. A drop loses every ping by
    /// definition, so counted in, the loss figure only repeats the drop count
    /// and says nothing about how the line behaves when it is up.
    pub outside_sent: u64,
    pub outside_lost: u64,
}

impl Ping {
    pub fn outside_loss_pct(&self) -> Option<f64> {
        (self.outside_sent > 0).then(|| self.outside_lost as f64 * 100.0 / self.outside_sent as f64)
    }

    fn read(store: &Store, key: &str, from: f64, to: f64, breaks: &[(f64, f64)]) -> Ping {
        let totals = store.ping_totals(key, from, to);
        let (mut sent, mut lost) = (totals.sent, totals.lost);
        for &(a, b) in breaks {
            let during = store.ping_totals(key, a, b);
            sent = sent.saturating_sub(during.sent);
            lost = lost.saturating_sub(during.lost);
        }
        Ping {
            totals,
            p95: store.rtt_percentile(key, from, to, 0.95),
            outside_sent: sent,
            outside_lost: lost,
        }
    }
}

/// What the router reported about itself over UPnP. Each figure is `None`
/// when the router never gave that field: not asked is not "none happened".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RouterTally {
    /// Its uptime counter started again: the router or its internet
    /// connection restarted. At least this many; two inside one stretch
    /// nobody watched count as one.
    pub restarts: Option<usize>,
    /// The public address changed.
    pub new_ips: Option<usize>,
}

/// One stretch of time, added up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Period {
    pub from: f64,
    pub to: f64,
    /// Every event in `[from, to]`: outages are kept for a year.
    pub tally: Tally,
    /// Where the samples start inside the period: later than `from` when the
    /// period reaches past how long they are kept.
    pub measured_from: f64,
    pub watched_s: f64,
    /// Downtime and drops inside `[measured_from, to]`, the part the watched
    /// time is about.
    pub measured_down_s: f64,
    pub measured_breaks: usize,
    /// The longest watched stretch without a drop, or `None` when too little
    /// was watched to say.
    pub longest_clear_s: Option<f64>,
}

impl Period {
    /// Availability in percent, or `None` when too little was watched to say.
    pub fn availability(&self) -> Option<f64> {
        (self.watched_s >= MIN_WATCHED_S).then(|| {
            let up = (self.watched_s - self.measured_down_s).max(0.0);
            (up / self.watched_s * 100.0).clamp(0.0, 100.0)
        })
    }

    /// Watched time per drop: "one every 3 h". `None` without drops or
    /// without enough watching.
    pub fn mean_between_breaks(&self) -> Option<f64> {
        (self.measured_breaks > 0 && self.watched_s >= MIN_WATCHED_S)
            .then(|| self.watched_s / self.measured_breaks as f64)
    }
}

/// Everything the statistics tab and the report's summary show.
#[derive(Debug, Clone)]
pub struct Totals {
    /// The period's length in days, `None` for everything on record.
    pub days: Option<u32>,
    pub built_at: f64,
    pub keep_days: i64,
    pub now: Period,
    /// The same length just before, to compare with. `None` for everything.
    pub prev: Option<Period>,
    pub internet: Ping,
    pub router_ping: Ping,
    /// `None` when the router never answered UPnP in the period.
    pub router: Option<RouterTally>,
    /// Shared, so a chart's formatters can hold the names without copying
    /// them on every frame.
    pub buckets: Arc<Vec<Bucket>>,
    pub hourly: bool,
    /// Per local hour of the day; `None` for a single day, where the "when"
    /// chart already is that.
    pub by_hour: Option<[HourShare; 24]>,
}

impl Totals {
    /// How long drops are kept, as [`Store::prune`] keeps them.
    pub fn outage_keep_days(&self) -> i64 {
        self.keep_days.max(1).max(store::OUTAGE_KEEP_DAYS)
    }

    /// The period before, when it can be compared with: every part of it
    /// inside the sample retention, and watched enough to say anything.
    /// Otherwise its "0 drops" may only mean nobody was looking.
    pub fn comparable_prev(&self) -> Option<&Period> {
        self.prev
            .as_ref()
            .filter(|p| p.measured_from <= p.from + 1.0 && p.watched_s >= MIN_WATCHED_S)
    }
}

/// The part of `e` inside `[from, to]`, in seconds. An open event runs to `to`.
fn clipped(e: &Event, from: f64, to: f64) -> f64 {
    let end = e.ts_end.unwrap_or(to).min(to);
    (end - e.ts_start.max(from)).max(0.0)
}

pub fn is_slow(e: &Event) -> bool {
    e.kind == Status::Degraded.key()
}

/// Adds up the events over `[from, to]`. Only the part of each inside the
/// period counts, so an outage that began yesterday is not all today's.
pub fn tally(events: &[Event], from: f64, to: f64) -> Tally {
    let mut t = Tally::default();
    for e in events {
        if e.ts_start >= to || e.ts_end.is_some_and(|end| end <= from) {
            continue;
        }
        let len = clipped(e, from, to);
        if is_slow(e) {
            t.slow += 1;
            t.slow_s += len;
        } else {
            t.breaks += 1;
            t.down_s += len;
            t.longest_s = t.longest_s.max(len);
            t.ongoing |= e.ts_end.is_none();
        }
        match t.by_kind.iter_mut().find(|(k, _, _)| *k == e.kind) {
            Some(row) => {
                row.1 += 1;
                row.2 += len;
            }
            None => t.by_kind.push((e.kind.clone(), 1, len)),
        }
    }
    // Ties broken by kind, so rows do not swap places between refreshes.
    t.by_kind.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    t
}

/// The drops' spans clipped to `[from, to]`, oldest first.
fn break_spans(events: &[Event], from: f64, to: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = events
        .iter()
        .filter(|e| !is_slow(e))
        .map(|e| (e.ts_start.max(from), e.ts_end.unwrap_or(to).min(to)))
        .filter(|(a, b)| b > a)
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// Spreads each event's time over the buckets it covers. Each event visits
/// only the buckets it overlaps, found by bisecting `starts`: a year of days
/// against every event in it was hundreds of millions of comparisons.
pub fn fill_buckets(events: &[Event], starts: &[f64], to: f64) -> Vec<Bucket> {
    let ends = starts.iter().skip(1).copied().chain(std::iter::once(to));
    let mut out: Vec<Bucket> = starts
        .iter()
        .zip(ends)
        .map(|(&from, end)| Bucket { from, to: end, ..Default::default() })
        .collect();
    for e in events {
        let end = e.ts_end.unwrap_or(to).min(to);
        let first = starts.partition_point(|&s| s <= e.ts_start).saturating_sub(1);
        let slow = is_slow(e);
        for b in out[first..].iter_mut().take_while(|b| b.from < end) {
            let len = clipped(e, b.from, b.to);
            if slow {
                b.slow_s += len;
            } else {
                b.down_s += len;
            }
        }
    }
    out
}

/// Names each bucket once, here, rather than on every frame it is drawn:
/// `label` for hovering it and for the history it opens, `tick` for the axis.
fn name_buckets(buckets: &mut [Bucket], hourly: bool) {
    use crate::clock::{format_clock, format_day};
    for b in buckets {
        if hourly {
            b.tick = format_clock(b.from);
            b.label = format!("{}–{}", b.tick, format_clock(b.to));
        } else {
            b.tick = format_day(b.from);
            b.label = b.tick.clone();
        }
    }
}

/// Bucket starts over `[from, to]`: whole hours for a day, local days for
/// anything longer.
fn bucket_starts(from: f64, to: f64, hourly: bool) -> Vec<f64> {
    let mut out = Vec::new();
    let mut at =
        if hourly { (from / 3600.0).floor() * 3600.0 } else { crate::clock::local_midnight(from) };
    while at < to {
        out.push(at.max(from));
        at = if hourly { at + 3600.0 } else { crate::clock::next_local_midnight(at) };
    }
    out
}

/// The stretches somebody was watching: runs of sweeps no more than `gap_s`
/// apart. The same reading of the samples as [`Store::observed_seconds`].
pub fn watched(times: &[f64], gap_s: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for &t in times {
        match out.last_mut() {
            Some(seg) if t - seg.1 <= gap_s => seg.1 = t,
            _ => out.push((t, t)),
        }
    }
    out.retain(|(a, b)| b > a);
    out
}

fn clip(segs: &[(f64, f64)], from: f64, to: f64) -> Vec<(f64, f64)> {
    segs.iter().map(|&(a, b)| (a.max(from), b.min(to))).filter(|(a, b)| b > a).collect()
}

fn total(segs: &[(f64, f64)]) -> f64 {
    segs.iter().map(|(a, b)| b - a).sum()
}

/// The longest piece of watched time with no drop in it.
pub fn longest_clear(segs: &[(f64, f64)], breaks: &[(f64, f64)]) -> f64 {
    let mut best = 0.0_f64;
    for &(a, b) in segs {
        let mut cursor = a;
        for &(s, e) in breaks.iter().filter(|(s, e)| *e > a && *s < b) {
            best = best.max(s.max(a) - cursor);
            cursor = cursor.max(e.min(b));
        }
        best = best.max(b - cursor);
    }
    best
}

/// Calls `add` with each piece of `[a, b]` that falls in one clock hour, and
/// that hour of the day.
///
/// ponytail: cut on whole UTC hours, so in a zone offset by a half hour each
/// piece is filed under the hour it starts in. Cut on local hours if a user
/// there ever reads the chart to the minute.
fn spread(a: f64, b: f64, hour_of: &dyn Fn(f64) -> usize, add: &mut dyn FnMut(usize, f64)) {
    let mut t = a;
    while t < b {
        let end = (((t / 3600.0).floor() + 1.0) * 3600.0).min(b);
        add(hour_of(t) % 24, end - t);
        t = end;
    }
}

/// Watched, down and slow time per hour of the day. `segs` must already be
/// clipped to `[from, to]`.
pub fn by_hour(
    segs: &[(f64, f64)],
    events: &[Event],
    from: f64,
    to: f64,
    hour_of: &dyn Fn(f64) -> usize,
) -> [HourShare; 24] {
    let mut out = [HourShare::default(); 24];
    for &(a, b) in segs {
        spread(a, b, hour_of, &mut |h, s| out[h].watched_s += s);
    }
    for e in events {
        let (a, b) = (e.ts_start.max(from), e.ts_end.unwrap_or(to).min(to));
        let slow = is_slow(e);
        spread(a, b, hour_of, &mut |h, s| {
            if slow {
                out[h].slow_s += s;
            } else {
                out[h].down_s += s;
            }
        });
    }
    out
}

/// Restarts and address changes, read off consecutive router answers the way
/// [`crate::cause`] reads them for one outage.
pub fn router_tally(readings: &[RouterReading]) -> Option<RouterTally> {
    if readings.is_empty() {
        return None;
    }
    let (mut restarts, mut new_ips) = (None::<usize>, None::<usize>);
    let (mut last_start, mut last_ip): (Option<f64>, Option<&str>) = (None, None);
    for r in readings {
        if let Some(c) = r.counter_start() {
            let n = restarts.get_or_insert(0);
            if last_start.is_some_and(|p| c > p + ROUTER_CLOCK_SLACK_S) {
                *n += 1;
            }
            last_start = Some(c);
        }
        if let Some(ip) = r.ip_tag.as_deref() {
            let n = new_ips.get_or_insert(0);
            if last_ip.is_some_and(|p| p != ip) {
                *n += 1;
            }
            last_ip = Some(ip);
        }
    }
    Some(RouterTally { restarts, new_ips })
}

/// Adds up `[from, to]`. `segs` are the watched stretches; `keep_from` is
/// where the samples start.
pub fn period(events: &[Event], segs: &[(f64, f64)], from: f64, to: f64, keep_from: f64) -> Period {
    let measured_from = from.max(keep_from).min(to);
    let measured = tally(events, measured_from, to);
    let segs = clip(segs, measured_from, to);
    let watched_s = total(&segs);
    Period {
        from,
        to,
        tally: tally(events, from, to),
        measured_from,
        watched_s,
        measured_down_s: measured.down_s,
        measured_breaks: measured.breaks,
        longest_clear_s: (watched_s >= MIN_WATCHED_S)
            .then(|| longest_clear(&segs, &break_spans(events, measured_from, to))),
    }
}

/// Counts everything for the last `days` (all of it for `None`). Slow: most
/// of a second on two weeks of samples, so run it off the UI thread, and on a
/// [`Store::side_reader`] so the monitor is not kept waiting.
pub fn count(store: &Store, days: Option<u32>, keep_days: i64, now: f64) -> Totals {
    let keep_from = now - keep_days.max(1) as f64 * DAY_S;
    let from = match days {
        Some(d) => now - f64::from(d) * DAY_S,
        None => store.first_event_ts().unwrap_or(keep_from).min(keep_from),
    };
    let prev_from = days.map(|_| from - (now - from));
    let earliest = prev_from.unwrap_or(from);

    let events = store.events_overlapping(earliest, now);
    let segs = watched(&store.sweep_times(earliest.max(keep_from), now), store::OBSERVATION_GAP_S);
    let current = period(&events, &segs, from, now, keep_from);
    let prev = prev_from.map(|pf| period(&events, &segs, pf, from, keep_from));

    let measured_from = current.measured_from;
    let breaks = break_spans(&events, measured_from, now);
    let hourly = days == Some(1);
    let mut buckets = fill_buckets(&events, &bucket_starts(from, now, hourly), now);
    name_buckets(&mut buckets, hourly);
    let by_hour = (!hourly).then(|| {
        by_hour(&clip(&segs, measured_from, now), &events, measured_from, now, &|t| {
            crate::clock::local_hour(t) as usize
        })
    });
    Totals {
        days,
        built_at: now,
        keep_days,
        internet: Ping::read(store, "cloudflare", measured_from, now, &breaks),
        router_ping: Ping::read(store, "gateway", measured_from, now, &breaks),
        router: router_tally(&store.router_between(measured_from, now)),
        buckets: Arc::new(buckets),
        hourly,
        by_hour,
        now: current,
        prev,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, start: f64, end: Option<f64>) -> Event {
        Event {
            id: 0,
            ts_start: start,
            ts_end: end,
            kind: kind.into(),
            scope: String::new(),
            detail: String::new(),
        }
    }

    #[test]
    fn breaks_and_slowdowns_are_counted_apart() {
        let events = [
            ev("isp_down", 100.0, Some(160.0)),
            ev("degraded", 200.0, Some(500.0)),
            ev("lan_down", 600.0, Some(610.0)),
            ev("isp_down", 700.0, Some(720.0)),
        ];
        let t = tally(&events, 0.0, 1000.0);
        assert_eq!((t.breaks, t.down_s, t.longest_s), (3, 90.0, 60.0));
        assert_eq!((t.slow, t.slow_s), (1, 300.0));
        assert!(!t.ongoing);
        // Longest total first, and slowness is a kind of its own.
        assert_eq!(
            t.by_kind,
            vec![
                ("degraded".to_string(), 1, 300.0),
                ("isp_down".to_string(), 2, 80.0),
                ("lan_down".to_string(), 1, 10.0),
            ]
        );
    }

    #[test]
    fn only_the_part_inside_the_period_counts() {
        // Began before the period, and one still running at its end.
        let events = [ev("isp_down", -50.0, Some(30.0)), ev("lan_down", 900.0, None)];
        let t = tally(&events, 0.0, 1000.0);
        assert_eq!((t.breaks, t.down_s, t.longest_s), (2, 130.0, 100.0));
        assert!(t.ongoing, "an open outage is said to be going on");

        // Entirely outside, or only touching the edge: not counted at all.
        let t = tally(&[ev("isp_down", 2000.0, Some(2100.0))], 0.0, 1000.0);
        assert_eq!(t, Tally::default());
        let t = tally(&[ev("isp_down", -10.0, Some(0.0))], 0.0, 1000.0);
        assert_eq!(t, Tally::default());
    }

    #[test]
    fn an_outage_across_buckets_is_split_between_them() {
        let events = [ev("isp_down", 50.0, Some(150.0)), ev("degraded", 120.0, Some(130.0))];
        let b = fill_buckets(&events, &[0.0, 100.0, 200.0], 300.0);
        let down: Vec<f64> = b.iter().map(|b| b.down_s).collect();
        let slow: Vec<f64> = b.iter().map(|b| b.slow_s).collect();
        assert_eq!(down, vec![50.0, 50.0, 0.0]);
        assert_eq!(slow, vec![0.0, 10.0, 0.0]);
        assert_eq!((b[2].from, b[2].to), (200.0, 300.0));

        // Begun before the first bucket, and one still open: each lands in
        // every bucket it covers and in no other.
        let events = [ev("lan_down", -50.0, Some(20.0)), ev("isp_down", 180.0, None)];
        let b = fill_buckets(&events, &[0.0, 100.0, 200.0], 300.0);
        let down: Vec<f64> = b.iter().map(|b| b.down_s).collect();
        assert_eq!(down, vec![20.0, 20.0, 100.0]);
    }

    fn watched_period(watched_s: f64, measured_down_s: f64, breaks: usize) -> Period {
        Period {
            to: DAY_S,
            watched_s,
            measured_down_s,
            measured_breaks: breaks,
            ..Default::default()
        }
    }

    #[test]
    fn availability_is_a_share_of_the_watched_time_not_the_calendar() {
        // Watched an hour of the day, down for 36 s of it.
        let a = watched_period(3600.0, 36.0, 1).availability().unwrap();
        assert!((a - 99.0).abs() < 1e-9, "{a}");
        // A minute of watching says nothing either way.
        assert_eq!(watched_period(60.0, 0.0, 0).availability(), None);
        // More down than watched cannot go below zero.
        assert_eq!(watched_period(1000.0, 5000.0, 1).availability(), Some(0.0));
    }

    #[test]
    fn time_between_drops_is_watched_time_per_drop() {
        assert_eq!(watched_period(7200.0, 10.0, 4).mean_between_breaks(), Some(1800.0));
        assert_eq!(watched_period(7200.0, 0.0, 0).mean_between_breaks(), None);
        assert_eq!(watched_period(60.0, 1.0, 1).mean_between_breaks(), None);
    }

    #[test]
    fn a_day_is_counted_in_hours_and_anything_longer_in_days() {
        let hours = bucket_starts(3600.0 * 10.5, 3600.0 * 13.0, true);
        assert_eq!(hours, vec![3600.0 * 10.5, 3600.0 * 11.0, 3600.0 * 12.0]);
        let now = store::now();
        let days = bucket_starts(now - 7.0 * DAY_S, now, false);
        assert!((7..=8).contains(&days.len()), "{}", days.len());
        assert!(days.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn a_hole_in_the_sweeps_splits_the_watched_time() {
        let times = [0.0, 1.0, 2.0, 100.0, 101.0, 500.0];
        // The lone sweep at 500 s is a moment, not a stretch.
        assert_eq!(watched(&times, 60.0), vec![(0.0, 2.0), (100.0, 101.0)]);
        assert_eq!(watched(&[], 60.0), vec![]);
    }

    #[test]
    fn the_longest_clear_stretch_is_watched_and_has_no_drop_in_it() {
        let segs = [(0.0, 1000.0), (5000.0, 5600.0)];
        // A drop at 300..310 leaves 690 s after it, and the hole between the
        // stretches is not clear time: nobody watched it.
        assert_eq!(longest_clear(&segs, &[(300.0, 310.0)]), 690.0);
        // Drops overlapping a stretch's edges.
        assert_eq!(longest_clear(&[(0.0, 100.0)], &[(-5.0, 10.0), (90.0, 200.0)]), 80.0);
        assert_eq!(longest_clear(&segs, &[]), 1000.0);
    }

    #[test]
    fn a_period_says_nothing_about_clear_time_it_barely_watched() {
        let p = period(&[], &[(0.0, 60.0)], 0.0, 1000.0, 0.0);
        assert_eq!((p.watched_s, p.longest_clear_s, p.availability()), (60.0, None, None));
        // Samples kept for less than the period: measured over what is left.
        let events = [ev("isp_down", 100.0, Some(110.0)), ev("isp_down", 900.0, Some(905.0))];
        let p = period(&events, &[(0.0, 2000.0)], 0.0, 2000.0, 500.0);
        assert_eq!((p.tally.breaks, p.measured_breaks, p.measured_from), (2, 1, 500.0));
        assert_eq!((p.watched_s, p.measured_down_s), (1500.0, 5.0));
        assert_eq!(p.longest_clear_s, Some(1095.0));
    }

    #[test]
    fn hours_of_the_day_are_shares_of_their_own_watched_time() {
        // A clock with no time zone: hour of the day is UTC's.
        let utc = |t: f64| ((t / 3600.0).floor() as i64).rem_euclid(24) as usize;
        let h = 3600.0;
        // Watched 20:00-22:00 on two days, and 09:00-09:10 once.
        let segs = [
            (20.0 * h, 22.0 * h),
            (DAY_S + 20.0 * h, DAY_S + 22.0 * h),
            (9.0 * h, 9.0 * h + 600.0),
        ];
        let events = [
            ev("isp_down", 21.0 * h, Some(21.0 * h + 360.0)),
            ev("degraded", DAY_S + 20.5 * h, Some(DAY_S + 21.5 * h)),
            ev("isp_down", 9.0 * h, Some(9.0 * h + 60.0)),
        ];
        let out = by_hour(&segs, &events, 0.0, 2.0 * DAY_S, &utc);
        assert_eq!(out[20].watched_s, 2.0 * h);
        let (down, slow) = out[21].shares().unwrap();
        assert!((down - 5.0).abs() < 1e-9 && (slow - 25.0).abs() < 1e-9, "{down} {slow}");
        assert_eq!(out[20].shares(), Some((0.0, 25.0)));
        // Ten minutes of watching at 09:00 is too little to call it bad.
        assert_eq!(out[9].shares(), None);
        assert_eq!(out[3].shares(), None);
    }

    fn reading(ts: f64, uptime: Option<u64>, ip: Option<&str>) -> RouterReading {
        RouterReading {
            ts,
            status: "Connected".into(),
            uptime_s: uptime,
            ip_tag: ip.map(str::to_string),
        }
    }

    #[test]
    fn the_router_restarting_and_changing_address_is_counted() {
        let r = [
            reading(1000.0, Some(900), Some("a")),
            reading(1030.0, Some(931), Some("a")), // clock slack: same run
            reading(2000.0, Some(20), Some("b")),  // restarted, new address
            reading(2030.0, Some(50), None),       // address not given
            reading(3000.0, Some(1020), Some("b")),
            reading(4000.0, Some(10), Some("a")),
        ];
        let t = router_tally(&r).unwrap();
        assert_eq!((t.restarts, t.new_ips), (Some(2), Some(2)));

        // A router that never gives a field says nothing about it.
        let t = router_tally(&[reading(0.0, None, None), reading(30.0, None, None)]).unwrap();
        assert_eq!((t.restarts, t.new_ips), (None, None));
        assert_eq!(router_tally(&[]), None, "no answers at all is not zero restarts");
    }

    #[test]
    fn counting_an_empty_history_claims_nothing() {
        let store = Store::open_in_memory().unwrap();
        let t = count(&store, Some(7), 14, store::now());
        assert_eq!(t.now.tally, Tally::default());
        assert_eq!(t.now.availability(), None, "nothing watched is not 100 %");
        assert_eq!(t.now.longest_clear_s, None);
        assert_eq!(t.internet.totals.loss_pct(), None);
        assert_eq!(t.router, None);
        assert!(t.prev.is_some() && count(&store, None, 14, store::now()).prev.is_none());
        assert!(t.comparable_prev().is_none(), "an unwatched week compares with nothing");
    }

    #[test]
    fn the_period_before_is_compared_only_where_it_was_measured() {
        let store = Store::open_in_memory().unwrap();
        let now = store::now();
        // Watched for an hour eight days ago, in the week before this one.
        let rows: Vec<(f64, String, Option<f64>, bool)> = (0..3600)
            .map(|i| (now - 8.0 * DAY_S + i as f64, "gateway".to_string(), Some(2.0), true))
            .collect();
        store.add_samples(&rows).unwrap();
        let week = count(&store, Some(7), 14, now);
        let prev = week.comparable_prev().expect("the week before was watched");
        assert!((prev.watched_s - 3599.0).abs() < 1e-6);
        // The month before reaches past the two weeks samples are kept.
        assert!(count(&store, Some(30), 14, now).comparable_prev().is_none());
    }

    #[test]
    fn loss_outside_the_drops_leaves_the_drops_out() {
        let store = Store::open_in_memory().unwrap();
        let now = store::now();
        // 100 sweeps a second apart; the middle 10 lost in a recorded drop,
        // and 2 more lost while the line was up.
        let rows: Vec<(f64, String, Option<f64>, bool)> = (0..100)
            .map(|i| {
                let ts = now - 100.0 + i as f64;
                let lost = (40..50).contains(&i) || i == 10 || i == 80;
                (ts, "cloudflare".to_string(), (!lost).then_some(20.0), !lost)
            })
            .collect();
        store.add_samples(&rows).unwrap();
        let id = store.open_event("isp_down", "isp", "", "{}").unwrap();
        store.close_event_at(id, now - 51.0, "").unwrap();
        // Starts a hair before sweep 40 and ends after sweep 49.
        store.reshape_events_for_test((now - 60.0) - store::now(), 9.5);

        let t = count(&store, Some(1), 14, now);
        assert_eq!((t.internet.totals.sent, t.internet.totals.lost), (100, 12));
        assert_eq!((t.internet.outside_sent, t.internet.outside_lost), (90, 2));
        assert_eq!(t.now.tally.breaks, 1);
    }
}
