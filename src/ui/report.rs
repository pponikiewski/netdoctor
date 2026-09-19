//! Plain-text report: everything the app knows, in a form you can paste into
//! a support ticket.

use std::fmt::Write as _;

use anyhow::Result;

use super::App;
use crate::bandwidth::Grade;
use crate::diagnose::format_datetime;
use crate::i18n;
use crate::probe::netstate::Medium;

pub fn build(app: &App) -> String {
    let mut out = String::new();
    let n = &app.net;

    let _ = writeln!(
        out,
        "{}, {}",
        i18n::rep_title(),
        format_datetime(crate::store::now())
    );
    let _ = writeln!(out, "{}", "=".repeat(72));
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", i18n::rep_sec_connection());
    let _ = writeln!(
        out,
        "  {:<14}: {} ({})",
        i18n::rep_adapter(),
        n.adapter_name,
        n.medium.label()
    );
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
        let _ = writeln!(
            out,
            "  {:<14}: {}%{}, {}",
            i18n::rep_signal(),
            n.signal_pct.unwrap_or(0),
            n.rssi_dbm.map(|r| format!(" / {r} dBm")).unwrap_or_default(),
            i18n::rep_channel_line(
                &n.channel.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
                n.band().unwrap_or("?"),
                &n.phy,
            )
        );
        let _ = writeln!(
            out,
            "  {:<14}: {}",
            i18n::rep_rates(),
            i18n::rep_rates_line(n.rx_mbps.unwrap_or(0), n.tx_mbps.unwrap_or(0))
        );
    } else {
        let _ = writeln!(
            out,
            "  {:<14}: {} Mbps",
            i18n::rep_link_speed(),
            n.link_speed_mbps
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "{}", i18n::rep_sec_measurements());
    for t in app.settings.targets() {
        let s = app.store.stats(&t.key, 3600.0);
        if s.count == 0 {
            continue;
        }
        let _ = writeln!(
            out,
            "{}",
            i18n::rep_stats_line(
                &t.label,
                s.count,
                s.loss_pct,
                s.avg.unwrap_or(0.0),
                s.min.unwrap_or(0.0),
                s.max.unwrap_or(0.0),
                s.jitter.unwrap_or(0.0),
            )
        );
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
            e.duration_s().map(|d| format!("{d:.0}s")).unwrap_or_else(|| i18n::hist_ongoing().into()),
            i18n::event_kind(&e.kind),
            e.detail
        );
    }

    if app.bloat.grade_or_unknown() != Grade::Unknown {
        let b = &app.bloat;
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", i18n::rep_sec_bloat());
        let _ = writeln!(out, "  {:<14}: {:.1} ms", i18n::rep_idle(), b.idle_avg.unwrap_or(0.0));
        match b.loaded_avg {
            Some(v) => {
                let _ = writeln!(
                    out,
                    "  {:<14}: {}",
                    i18n::rep_loaded(),
                    i18n::rep_loaded_line(v, b.loaded_max.unwrap_or(0.0), b.loaded_loss_pct)
                );
            }
            None => {
                let _ = writeln!(out, "  {:<14}: {}", i18n::rep_loaded(), i18n::rep_no_reply());
            }
        }
        let _ = writeln!(out, "  {:<14}: {:.0} ms", i18n::rep_increase(), b.bump_ms.unwrap_or(0.0));
        let _ = writeln!(out, "  {:<14}: {:.0} Mbps", i18n::rep_throughput(), b.mbps.unwrap_or(0.0));
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
        let _ = writeln!(out, "  {}: {} — {}", i18n::verdict_heading(), v.segment.label(), v.confidence.label());
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

/// Writes the report next to the database and returns the path.
pub fn save(app: &App) -> Result<String> {
    let dir = crate::settings::data_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("netdoctor-report.txt");
    std::fs::write(&path, build(app))?;
    Ok(path.display().to_string())
}
