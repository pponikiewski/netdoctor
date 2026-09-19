//! Plain-text report: everything the app knows, in a form you can paste into
//! a support ticket.

use std::fmt::Write as _;

use anyhow::Result;

use super::App;
use crate::bandwidth::Grade;
use crate::diagnose::format_datetime;
use crate::probe::netstate::Medium;

pub fn build(app: &App) -> String {
    let mut out = String::new();
    let n = &app.net;

    let _ = writeln!(out, "NetDoctor report — {}", format_datetime(crate::store::now()));
    let _ = writeln!(out, "{}", "=".repeat(72));
    let _ = writeln!(out);

    let _ = writeln!(out, "CONNECTION");
    let _ = writeln!(out, "  adapter    : {} ({})", n.adapter_name, n.medium.label());
    let _ = writeln!(out, "  driver     : {}", n.adapter_desc);
    let _ = writeln!(
        out,
        "  gateway    : {}",
        n.gateway.map(|g| g.to_string()).unwrap_or_else(|| "none".into())
    );
    let _ = writeln!(
        out,
        "  DNS        : {}",
        n.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ")
    );
    if n.medium == Medium::Wifi {
        let _ = writeln!(out, "  SSID       : {} (BSSID {})", n.ssid, n.bssid);
        let _ = writeln!(
            out,
            "  signal     : {}%{}, channel {} ({}), {}",
            n.signal_pct.unwrap_or(0),
            n.rssi_dbm.map(|r| format!(" / {r} dBm")).unwrap_or_default(),
            n.channel.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
            n.band().unwrap_or("?"),
            n.phy
        );
        let _ = writeln!(
            out,
            "  rates      : {} Mbps receive / {} Mbps transmit",
            n.rx_mbps.unwrap_or(0),
            n.tx_mbps.unwrap_or(0)
        );
    } else {
        let _ = writeln!(out, "  link speed : {} Mbps", n.link_speed_mbps);
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "MEASUREMENTS (last hour)");
    for t in app.settings.targets() {
        let s = app.store.stats(&t.key, 3600.0);
        if s.count == 0 {
            continue;
        }
        let _ = writeln!(
            out,
            "  {:<14} samples {:>5}, loss {:>5.1}%, avg {:>7.2} ms, min {:>6.2}, max {:>7.2}, jitter {:>6.2}",
            t.label,
            s.count,
            s.loss_pct,
            s.avg.unwrap_or(0.0),
            s.min.unwrap_or(0.0),
            s.max.unwrap_or(0.0),
            s.jitter.unwrap_or(0.0)
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "OUTAGES (last 24 hours)");
    let events = app.store.events_since(24.0 * 3600.0);
    if events.is_empty() {
        let _ = writeln!(out, "  none");
    }
    for e in &events {
        let _ = writeln!(
            out,
            "  {}  {:<9} {:<9} {}",
            format_datetime(e.ts_start),
            e.duration_s().map(|d| format!("{d:.0}s")).unwrap_or_else(|| "ongoing".into()),
            e.scope,
            e.detail
        );
    }

    if app.bloat.grade_or_unknown() != Grade::Unknown {
        let b = &app.bloat;
        let _ = writeln!(out);
        let _ = writeln!(out, "LATENCY UNDER LOAD (bufferbloat)");
        let _ = writeln!(out, "  idle       : {:.1} ms", b.idle_avg.unwrap_or(0.0));
        match b.loaded_avg {
            Some(v) => {
                let _ = writeln!(
                    out,
                    "  loaded     : {v:.1} ms (max {:.0}, loss {:.0}%)",
                    b.loaded_max.unwrap_or(0.0),
                    b.loaded_loss_pct
                );
            }
            None => {
                let _ = writeln!(out, "  loaded     : no reply");
            }
        }
        let _ = writeln!(out, "  increase   : {:.0} ms", b.bump_ms.unwrap_or(0.0));
        let _ = writeln!(out, "  throughput : {:.0} Mbps", b.mbps.unwrap_or(0.0));
        let _ = writeln!(
            out,
            "  grade      : {} — {}",
            b.grade_or_unknown().letter(),
            b.grade_or_unknown().verdict()
        );
    }

    if !app.findings.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "DIAGNOSIS");
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
    let _ = writeln!(out, "SETTINGS CHANGES");
    let log = app.store.tweak_log(50);
    if log.is_empty() {
        let _ = writeln!(out, "  none");
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
