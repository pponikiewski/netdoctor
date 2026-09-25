//! Plain-text report: everything the app knows, in a form you can paste into
//! a support ticket.
//!
//! Two halves. What the connection looks like now (the adapter, the last
//! hour's measurements, the path, the scan) is read on the UI thread, which
//! owns it. The outages over the chosen range, each with its cause, the
//! evidence for it, the Windows log around it and the path as it was when it
//! began, are put together on a worker thread: the log is a `wevtutil` launch
//! per outage, and a month of them is too long to hold a frame for.

use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::Result;

use super::{App, Job};
use crate::bandwidth::Grade;
use crate::cause::{self, Evidence};
use crate::diagnose::format_datetime;
use crate::i18n;
use crate::monitor::LeadSample;
use crate::probe::eventlog::{self, SysEvent};
use crate::probe::netstate::{Medium, NetState};
use crate::probe::path::PathReading;
use crate::store::{self, Event, Stats, Store};

/// How far back a report reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Day,
    Week,
    Month,
    All,
}

impl Range {
    pub const ALL: [Range; 4] = [Range::Day, Range::Week, Range::Month, Range::All];

    fn days(self) -> Option<u32> {
        match self {
            Range::Day => Some(1),
            Range::Week => Some(7),
            Range::Month => Some(30),
            Range::All => None,
        }
    }

    pub fn label(self) -> String {
        i18n::rep_range(self.days())
    }
}

/// Outages whose Windows log is read, newest first. Each read launches
/// `wevtutil` twice; the rest say theirs was not read rather than pretend the
/// log was quiet.
const LOG_READ_LIMIT: usize = 30;

/// Log lines printed per outage. The ones that explain it come first in the
/// cause line anyway; these are the evidence behind it.
const LOG_LINES: usize = 8;

/// Starts writing the report on a worker thread. The result arrives as
/// [`Job::ReportSaved`].
pub fn save_in_background(app: &mut App, range: Range) {
    if app.report_busy {
        return;
    }
    app.report_busy = true;
    let head = head(app);
    let tail = tail(app);
    let store = Arc::clone(&app.store);
    let keep_days = app.settings.keep_days;
    let dir = app.settings.report_dir_path();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let to = store::now();
        let from = range.days().map_or(0.0, |d| to - f64::from(d) * 86_400.0);
        let outages = outages_section(&store, from, to, &range.label(), keep_days, |e| {
            let (a, b) = eventlog::span_around(e);
            eventlog::window(a, b)
        });
        let result = write(&format!("{head}{outages}{tail}"), &dir).map_err(|e| e.to_string());
        let _ = tx.send(Job::ReportSaved(result));
    });
}

/// The title, the connection, the last hour's measurements and the path now.
fn head(app: &App) -> String {
    let mut out = String::new();
    let n = &app.net;

    let _ = writeln!(out, "{}, {}", i18n::rep_title(), format_datetime(store::now()));
    let _ = writeln!(out, "{}", "=".repeat(72));
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", i18n::rep_sec_connection());
    // The first two things a provider asks, and the two this computer cannot
    // read for itself, so they are only here when the user gave them.
    let st = &app.settings;
    if st.line_kind != crate::settings::LineKind::Unknown {
        let _ = writeln!(out, "  {:<14}: {}", i18n::rep_line(), st.line_kind.label());
    }
    if st.plan(false).is_some() || st.plan(true).is_some() {
        let plan = i18n::rep_plan_line(st.plan(false), st.plan(true));
        let _ = writeln!(out, "  {:<14}: {}", i18n::rep_plan(), plan);
    }
    let _ =
        writeln!(out, "  {:<14}: {} ({})", i18n::rep_adapter(), n.adapter_name, n.medium.label());
    let _ = writeln!(out, "  {:<14}: {}", i18n::rep_driver(), n.adapter_desc);
    let _ = writeln!(
        out,
        "  {:<14}: {}",
        i18n::rep_gateway(),
        n.gateway.map(|g| g.to_string()).unwrap_or_else(|| i18n::word_none().into())
    );
    let _ = writeln!(
        out,
        "  {:<14}: {}",
        i18n::rep_dns(),
        n.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ")
    );
    if n.medium == Medium::Wifi {
        let _ = writeln!(out, "  {:<14}: {} (BSSID {})", i18n::rep_ssid(), n.ssid, n.bssid);
        let _ = writeln!(out, "  {:<14}: {}", i18n::rep_signal(), signal_text(n));
        let _ = writeln!(
            out,
            "  {:<14}: {}",
            i18n::rep_rates(),
            i18n::rep_rates_line(n.rx_mbps, n.tx_mbps)
        );
    } else {
        let _ = writeln!(out, "  {:<14}: {} Mbps", i18n::rep_link_speed(), n.link_speed_mbps);
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "{}", i18n::rep_sec_measurements());
    for t in app.settings.targets() {
        let s = app.store.stats(&t.key, 3600.0);
        if s.count == 0 {
            continue;
        }
        let _ = writeln!(out, "{}", measurement_line(&t.label, &s));
    }

    // The hop table is the part of this report a provider cannot wave away,
    // so it goes in ahead of the outage list: the outages say something broke,
    // this says where.
    let path = app.monitor.path();
    if !path.hops.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", i18n::rep_sec_path());
        path_lines(&mut out, &path, "  ");
    }
    out
}

/// The load test, the scan and the log of changes.
fn tail(app: &App) -> String {
    let mut out = String::new();

    if app.bloat.grade_or_unknown() != Grade::Unknown {
        let b = &app.bloat;
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", i18n::rep_sec_bloat());
        let dash = |v: Option<f64>, p: usize| i18n::figure_or_dash(v, 0, p);
        let _ = writeln!(out, "  {:<14}: {} ms", i18n::rep_idle(), dash(b.idle_avg, 1));
        match b.loaded_avg {
            Some(v) => {
                let _ = writeln!(
                    out,
                    "  {:<14}: {}",
                    i18n::rep_loaded(),
                    i18n::rep_loaded_line(v, b.loaded_max, b.loaded_loss_pct)
                );
            }
            None => {
                let _ = writeln!(out, "  {:<14}: {}", i18n::rep_loaded(), i18n::rep_no_reply());
            }
        }
        let _ = writeln!(out, "  {:<14}: {} ms", i18n::rep_increase(), dash(b.bump_ms, 0));
        let _ = writeln!(out, "  {:<14}: {} Mbps", i18n::rep_throughput(), dash(b.mbps, 0));
        let _ = writeln!(
            out,
            "  {:<14}: {}, {}",
            i18n::rep_grade(),
            b.grade_or_unknown().letter(),
            b.grade_or_unknown().verdict()
        );
    }

    if !app.findings.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", i18n::rep_sec_diagnosis());

        // The verdict goes first and in full. A report is usually pasted into
        // a ticket, and the segment split is the part that decides whether the
        // person reading it is the right person to be reading it at all.
        let v = &app.verdict;
        let _ = writeln!(
            out,
            "  {}: {} — {}",
            i18n::verdict_heading(),
            v.segment.label(),
            v.confidence.label()
        );
        if let Some(split) = &v.split {
            let _ = writeln!(out, "  {split}");
        }
        let _ = writeln!(out, "  {}", v.cost);
        for (n, a) in v.actions.iter().enumerate() {
            let _ = writeln!(out, "  {}. {}", n + 1, a.text);
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "  {}", crate::diagnose::summarise(&app.findings));
        let _ = writeln!(out);
        for f in &app.findings {
            let _ = writeln!(out, "  [{}] {}", f.severity.label(), f.title);
            if !f.detail.is_empty() {
                let _ = writeln!(out, "      {}", f.detail);
            }
            if !f.advice.is_empty() {
                let _ = writeln!(out, "      -> {}", f.advice);
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "{}", i18n::rep_sec_changes());
    let log = app.store.tweak_log(50);
    if log.is_empty() {
        let _ = writeln!(out, "  {}", i18n::rep_none());
    }
    for row in &log {
        let _ = writeln!(
            out,
            "  {}  {:<18} {:<7} {}",
            format_datetime(row.ts),
            row.tweak_id,
            row.action,
            row.result
        );
    }
    out
}

/// Every outage in `[from, to]`, oldest first, with what was watched around
/// them, and for each one: its cause and the evidence for it, the Windows
/// log around it, the path when it began, and the minute before it.
///
/// `read_log` fetches the Windows log for one outage; a test passes a stub.
/// `keep_days` is how long measurements are kept, which bounds what can be
/// said about watching.
fn outages_section(
    store: &Store,
    from: f64,
    to: f64,
    range: &str,
    keep_days: i64,
    mut read_log: impl FnMut(&Event) -> Vec<SysEvent>,
) -> String {
    let mut out = String::new();
    let mut events: Vec<Event> = store
        .events_since(to - from)
        .into_iter()
        .filter(|e| e.ts_start >= from && e.ts_start <= to)
        .collect();
    events.reverse();

    let shown_from = if from > 0.0 { from } else { events.first().map_or(to, |e| e.ts_start) };
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        i18n::rep_outages_heading(range, &format_datetime(shown_from), &format_datetime(to))
    );
    let watched = store.observed_seconds(shown_from, to, store::OBSERVATION_GAP_S);
    let _ =
        writeln!(out, "{}", i18n::rep_watched(&i18n::span(watched), &i18n::span(to - shown_from)));
    if to - shown_from > keep_days.max(1) as f64 * 86_400.0 {
        let _ = writeln!(out, "{}", i18n::rep_samples_kept(keep_days));
    }

    if events.is_empty() {
        let _ = writeln!(out, "  {}", i18n::rep_none());
        return out;
    }

    // Keyed by the stored kind, so a slow line can be told from a dead one:
    // its minutes are poor quality, not downtime.
    let mut totals: std::collections::BTreeMap<&str, (usize, f64)> = Default::default();
    for e in &events {
        let t = totals.entry(e.kind.as_str()).or_default();
        t.0 += 1;
        t.1 += e.duration_s().unwrap_or(0.0);
    }
    for (kind, (count, length)) in &totals {
        let slow = *kind == crate::monitor::Status::Degraded.key();
        let line = i18n::rep_total(&i18n::event_kind(kind), *count, &i18n::span(*length), slow);
        let _ = writeln!(out, "{line}");
    }

    // Breaks get the full account: cause, log, path, the minute before.
    // Periods of poor quality are listed a line each after them. Given the
    // full account too, a day of 320 of them buried the 19 real outages
    // under ninety pages.
    let slow_key = crate::monitor::Status::Degraded.key();
    let (slow, breaks): (Vec<&Event>, Vec<&Event>) =
        events.iter().partition(|e| e.kind == slow_key);
    let n = breaks.len();
    for (i, &e) in breaks.iter().enumerate() {
        let _ = writeln!(out);
        let length = e
            .duration_s()
            .map(|d| format!("{d:.0} s"))
            .unwrap_or_else(|| i18n::rep_ongoing().into());
        let _ = writeln!(
            out,
            "  #{}  {}  {}  {}",
            i + 1,
            format_datetime(e.ts_start),
            length,
            i18n::event_kind(&e.kind)
        );
        const IN: &str = "      ";
        if !e.detail.is_empty() {
            let _ = writeln!(out, "{IN}{}", e.detail);
        }

        let ctx = store.event_context(e.id);
        let unwatched = ctx
            .as_ref()
            .and_then(|c| c.context_end_json())
            .and_then(|v| v["unwatched_s"].as_f64())
            .filter(|s| *s > 0.0);
        if let Some(secs) = unwatched {
            let _ = writeln!(out, "{IN}{}", i18n::rep_unwatched(secs));
        }

        let evidence = ctx.as_ref().and_then(Evidence::from_context);
        let log = (n - i <= LOG_READ_LIMIT).then(|| read_log(e));
        let tweaks = store.tweaks_between(e.ts_start - 3600.0, e.ts_start);
        let router = store.router_between(
            e.ts_start - cause::ROUTER_MARGIN_S,
            e.ts_end.unwrap_or(e.ts_start) + cause::ROUTER_MARGIN_S,
        );
        let causes = cause::analyse(
            e,
            evidence.as_ref(),
            &events,
            &tweaks,
            log.as_deref().unwrap_or(&[]),
            &router,
        );
        for (k, c) in causes.iter().take(3).enumerate() {
            let title = i18n::cause_title(c.code);
            let line = if k == 0 {
                i18n::rep_cause(&title, c.confidence.label(), &c.evidence)
            } else {
                i18n::rep_also(&title, c.confidence.label(), &c.evidence)
            };
            let _ = writeln!(out, "{IN}{line}");
        }

        match &log {
            None => {
                let _ = writeln!(out, "{IN}{}", i18n::rep_log_not_read());
            }
            Some(lines) if lines.is_empty() => {
                let _ = writeln!(out, "{IN}{}", i18n::rep_log_quiet());
            }
            Some(lines) => {
                let _ = writeln!(out, "{IN}{}", i18n::rep_log_heading());
                for l in lines.iter().take(LOG_LINES) {
                    let _ = writeln!(
                        out,
                        "{IN}  {}  {} {}  {}",
                        i18n::clock_offset(l.offset_from(e.ts_start)),
                        l.provider,
                        l.id,
                        l.detail
                    );
                }
            }
        }

        match evidence.as_ref().and_then(|ev| ev.path.as_ref()) {
            Some(path) => {
                let _ = writeln!(out, "{IN}{}", i18n::rep_path_then());
                path_lines(&mut out, path, "        ");
            }
            None => {
                let _ = writeln!(out, "{IN}{}", i18n::rep_path_unrecorded());
            }
        }

        if let Some(line) = evidence.as_ref().and_then(|ev| lead_minute(&ev.lead, e.ts_start)) {
            let _ = writeln!(out, "{IN}{line}");
        }
    }

    if !slow.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", i18n::rep_slow_heading(slow.len()));
        for e in slow {
            let length = e
                .duration_s()
                .map_or_else(|| i18n::rep_ongoing().into(), |d| format!("{d:>4.0} s"));
            // The first sentence says what was poor; the rest is in the app.
            let what = e.detail.split(". ").next().unwrap_or("").trim_end_matches('.');
            let _ = writeln!(out, "  {}  {length}  {what}", format_datetime(e.ts_start));
        }
    }
    out
}

/// The minute of sweeps before `t0`, as one line: the average round trip to
/// the router and to the internet, how many sweeps reached the internet, and
/// where the signal went. `None` when the row carries no lead-up.
fn lead_minute(lead: &[LeadSample], t0: f64) -> Option<String> {
    let minute: Vec<&LeadSample> =
        lead.iter().filter(|s| s.ts >= t0 - 60.0 && s.ts <= t0).collect();
    if minute.is_empty() {
        return None;
    }
    let mean = |v: Vec<f64>| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    let router = mean(minute.iter().filter_map(|s| s.gateway_ms).collect());
    let internet = mean(minute.iter().filter_map(|s| s.internet_ms).collect());
    let reached = minute.iter().filter(|s| s.internet_ok).count();
    let mut line = i18n::rep_lead_minute(
        &i18n::figure_or_dash(router, 0, 1),
        &i18n::figure_or_dash(internet, 0, 1),
        reached,
        minute.len(),
    );
    let rssi: Vec<i32> = minute.iter().filter_map(|s| s.rssi_dbm).collect();
    if let (Some(first), Some(last)) = (rssi.first(), rssi.last()) {
        line.push_str(&i18n::rep_lead_signal(*first, *last));
    }
    Some(line)
}

/// The hop table, one line per hop, and the hop the trouble starts at.
fn path_lines(out: &mut String, path: &PathReading, indent: &str) {
    for h in &path.hops {
        // A silent hop is reported as silent. Printing "100% loss" next to a
        // router that simply does not answer echoes would be the most
        // alarming line in a document meant to be trusted.
        let measured = if h.silent {
            i18n::path_no_answer().to_string()
        } else {
            format!(
                "{:>5.0}% loss  {}",
                h.loss_pct,
                h.avg_ms.map(|v| format!("{v:.0} ms")).unwrap_or_else(|| "-".into())
            )
        };
        let _ = writeln!(
            out,
            "{indent}{:>2}  {:<16} {:<30} {}",
            h.ttl,
            h.addr,
            i18n::path_owner(h.owner),
            measured
        );
    }
    if let Some(b) = &path.blame {
        let owner = i18n::path_owner(b.owner);
        let line = match b.added_ms {
            Some(added) => i18n::path_blame_delay(b.ttl, &b.addr.to_string(), added, owner),
            None => i18n::path_blame_loss(b.ttl, &b.addr.to_string(), b.loss_pct, owner),
        };
        let _ = writeln!(out, "{indent}-> {line}");
    }
}

/// One target's line in the measurements section.
fn measurement_line(label: &str, s: &Stats) -> String {
    i18n::rep_stats_line(label, s.count, s.loss_pct, s.avg, s.min, s.max, s.jitter)
}

/// Signal, RSSI and channel of a Wi-Fi link.
fn signal_text(n: &NetState) -> String {
    format!(
        "{}%{}, {}",
        n.signal_pct.map_or_else(|| "?".into(), |p| p.to_string()),
        n.rssi_dbm.map(|r| format!(" / {r} dBm")).unwrap_or_default(),
        i18n::rep_channel_line(
            &n.channel.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
            n.band().unwrap_or("?"),
            &n.phy,
        )
    )
}

/// Writes the report into `dir` as a PDF named after the moment it was made,
/// and returns the path. Each report is its own file: the one before is
/// evidence too. Without a usable font it is written as plain text instead,
/// which says the same thing.
fn write(text: &str, dir: &std::path::Path) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    let stem = format!("{} {}", i18n::rep_file_stem(), crate::diagnose::file_stamp(store::now()));
    let (bytes, ext) = match crate::pdf::render(text, i18n::pdf_page) {
        Ok(pdf) => (pdf, "pdf"),
        Err(_) => (text.as_bytes().to_vec(), "txt"),
    };
    let path = free_name(dir, &stem, ext);
    std::fs::write(&path, bytes)?;
    Ok(path.display().to_string())
}

/// `stem.ext` in `dir`, or `stem (2).ext` and so on when that is taken, so
/// two reports in the same minute do not overwrite each other.
fn free_name(dir: &std::path::Path, stem: &str, ext: &str) -> std::path::PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| dir.join(format!("{stem} ({n}).{ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_report_in_the_same_minute_does_not_overwrite_the_first() {
        let dir = std::env::temp_dir().join(format!("netdoctor-names-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let first = free_name(&dir, "report", "pdf");
        let _ = std::fs::write(&first, b"x");
        let second = free_name(&dir, "report", "pdf");
        assert_ne!(first, second);
        assert!(second.display().to_string().ends_with("report (2).pdf"), "{}", second.display());
        let _ = std::fs::remove_dir_all(&dir);
    }
    use crate::probe::eventlog::Kind;
    use crate::probe::path::{HopReading, Owner};
    use std::net::Ipv4Addr;

    #[test]
    fn a_reading_that_never_came_is_printed_as_missing_not_as_zero() {
        // A target that lost every packet has no latency. The report printed
        // "avg 0.00 ms", which reads as the fastest line in the document.
        let dead = crate::store::summarise(60, &[]);
        let line = measurement_line("1.1.1.1", &dead);
        assert!(line.contains("100.0%"), "{line}");
        assert!(!line.contains("0.00"), "no invented zero: {line}");

        // One reply has no jitter: there is nothing to compare it with.
        let one = crate::store::summarise(60, &[12.0]);
        assert!(!measurement_line("x", &one).contains(" 0.00"), "{}", measurement_line("x", &one));

        // A Wi-Fi link whose signal could not be read is not at 0%.
        let n = NetState { medium: Medium::Wifi, ..Default::default() };
        assert!(!signal_text(&n).starts_with("0%"), "{}", signal_text(&n));
    }

    fn a_path() -> PathReading {
        PathReading {
            hops: vec![HopReading {
                ttl: 2,
                addr: Ipv4Addr::new(100, 64, 7, 1),
                owner: Owner::Edge,
                loss_pct: 40.0,
                avg_ms: Some(18.0),
                samples: 30,
                silent: false,
            }],
            blame: None,
        }
    }

    #[test]
    fn each_outage_comes_with_its_cause_its_log_and_the_path_it_broke_on() {
        let store = Store::open_in_memory().unwrap();
        let lead: Vec<LeadSample> = (0..30)
            .map(|i| LeadSample {
                ts: store::now() - 30.0 + i as f64,
                gateway_ms: Some(2.0),
                internet_ms: Some(14.0),
                internet_ok: i < 20,
                rssi_dbm: Some(-50 - i),
                ..Default::default()
            })
            .collect();
        let ctx = serde_json::json!({ "medium": "Wi-Fi", "lead_up": lead, "path": a_path() });
        let id = store.open_event("isp_down", "isp", "router answers", &ctx.to_string()).unwrap();
        store.close_event_at(id, store::now() + 45.0, r#"{"unwatched_s": 12}"#).unwrap();

        let log = |_: &Event| {
            vec![SysEvent {
                ts: store::now() - 5.0,
                provider: "Microsoft-Windows-NetworkProfile".into(),
                id: 10001,
                kind: Kind::LinkDown,
                reason: None,
                detail: "Name=WiFi".into(),
            }]
        };
        let now = store::now();
        let text = outages_section(&store, now - 86_400.0, now + 60.0, "range", 14, log);

        assert!(text.contains("router answers"), "{text}");
        assert!(text.contains(&i18n::rep_unwatched(12.0)), "the unwatched seconds: {text}");
        assert!(text.contains(&i18n::cause_title("isp_brief")), "the cause: {text}");
        assert!(text.contains("10001") && text.contains("Name=WiFi"), "the log line: {text}");
        assert!(text.contains("100.64.7.1"), "the path as it was then: {text}");
        assert!(text.contains("20 of 30") || text.contains("20 z 30"), "the minute before: {text}");
        assert!(text.contains("-50") && text.contains("-79"), "where the signal went: {text}");
        assert!(text.contains(&i18n::rep_total(&i18n::event_kind("isp_down"), 1, "45 s", false)));
    }

    #[test]
    fn a_report_keeps_to_its_range_and_says_what_it_did_not_read() {
        let store = Store::open_in_memory().unwrap();
        // Forty days ago, then 31 more today; one has no stored path.
        store.open_event("lan_down", "lan", "old one", "{}").unwrap();
        store.reshape_events_for_test(-40.0 * 86_400.0, 10.0);
        let today: Vec<i64> = (0..31)
            .map(|i| store.open_event("lan_down", "lan", &format!("n{i}"), "{}").unwrap())
            .collect();
        for id in &today {
            store.close_event_at(*id, store::now() + 5.0, "").unwrap();
        }

        let mut reads = 0;
        let now = store::now();
        let text = outages_section(&store, now - 30.0 * 86_400.0, now + 60.0, "30", 14, |_| {
            reads += 1;
            Vec::new()
        });
        assert!(!text.contains("old one"), "forty days ago is outside thirty");
        assert_eq!(reads, LOG_READ_LIMIT, "only the newest outages have their log read");
        assert!(text.contains(i18n::rep_log_not_read()), "and the rest say so");
        assert!(text.contains(i18n::rep_path_unrecorded()), "a row with no path says so");
        assert!(text.contains(&i18n::rep_samples_kept(14)), "thirty days reach past fourteen");
    }
}
