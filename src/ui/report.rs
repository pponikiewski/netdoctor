//! Plain-text report: everything the app knows, in a form you can paste into
//! a support ticket.

use std::fmt::Write as _;

use anyhow::Result;

use super::App;
use crate::bandwidth::Grade;
use crate::diagnose::format_datetime;
use crate::i18n;
use crate::probe::netstate::{Medium, NetState};
use crate::store::Stats;

pub fn build(app: &App) -> String {
    let mut out = String::new();
    let n = &app.net;

    let _ = writeln!(out, "{}, {}", i18n::rep_title(), format_datetime(crate::store::now()));
    let _ = writeln!(out, "{}", "=".repeat(72));
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", i18n::rep_sec_connection());
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
        for h in &path.hops {
            // A silent hop is reported as silent. Printing "100% loss" next
            // to a router that simply does not answer echoes would be the
            // most alarming line in a document meant to be trusted.
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
                "  {:>2}  {:<16} {:<20} {}",
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
            let _ = writeln!(out, "  -> {line}");
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "{}", i18n::rep_sec_outages());
    let events = app.store.events_since(24.0 * 3600.0);
    if events.is_empty() {
        let _ = writeln!(out, "  {}", i18n::rep_none());
    }
    for e in &events {
        let _ = writeln!(
            out,
            "  {}  {:<9} {:<9} {}",
            format_datetime(e.ts_start),
            e.duration_s()
                .map(|d| format!("{d:.0}s"))
                .unwrap_or_else(|| i18n::hist_ongoing().into()),
            i18n::event_kind(&e.kind),
            e.detail
        );
    }

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

/// Writes the report next to the database and returns the path.
pub fn save(app: &App) -> Result<String> {
    let dir = crate::settings::data_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("netdoctor-report.txt");
    std::fs::write(&path, build(app))?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
