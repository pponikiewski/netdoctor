//! One-shot diagnostic scan.
//!
//! Each check produces Findings. Severity drives ordering in the UI, and a
//! finding may point at the tweak that fixes it.

use std::net::Ipv4Addr;
use std::sync::Arc;

use crate::i18n;
use crate::monitor;
use crate::probe::icmp;
use crate::probe::netstate::{Medium, NetState};
use crate::settings::Settings;
use crate::store::{self, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Good,
    Info,
    Warn,
    Critical,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Critical => crate::i18n::sev_critical(),
            Severity::Warn => crate::i18n::sev_warning(),
            Severity::Info => crate::i18n::sev_info(),
            Severity::Good => crate::i18n::sev_ok(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub key: String,
    pub title: String,
    pub severity: Severity,
    pub detail: String,
    pub advice: String,
    pub tweak_id: Option<String>,
}

impl Finding {
    fn new(
        key: &str,
        title: impl Into<String>,
        severity: Severity,
        detail: impl Into<String>,
    ) -> Self {
        Finding {
            key: key.into(),
            title: title.into(),
            severity,
            detail: detail.into(),
            advice: String::new(),
            tweak_id: None,
        }
    }

    fn advise(mut self, advice: impl Into<String>) -> Self {
        self.advice = advice.into();
        self
    }

    fn fixed_by(mut self, id: &str) -> Self {
        self.tweak_id = Some(id.into());
        self
    }
}

pub type Progress = Arc<dyn Fn(&str, f32) + Send + Sync>;

/// Run every check. Ordered worst-first on return.
pub fn scan(
    net: &NetState,
    store: &Store,
    settings: &Settings,
    progress: Option<Progress>,
) -> Vec<Finding> {
    let steps: Vec<(&str, fn(&NetState, &Store, &Settings) -> Vec<Finding>)> = vec![
        (i18n::step_medium(), check_medium),
        (i18n::step_wifi(), check_wifi),
        (i18n::step_power(), check_power),
        (i18n::step_dns(), check_dns),
        (i18n::step_link(), check_local_link),
        (i18n::step_internet(), check_internet),
        (i18n::step_mtu(), check_mtu),
        (i18n::step_tcp(), check_tcp),
        (i18n::step_history(), check_history),
    ];

    let total = steps.len() as f32;
    let mut out = Vec::new();
    for (i, (label, f)) in steps.iter().enumerate() {
        if let Some(p) = &progress {
            p(label, i as f32 / total);
        }
        out.extend(f(net, store, settings));
    }
    if let Some(p) = &progress {
        p(i18n::step_done(), 1.0);
    }

    // Worst first, stable within a severity so related findings stay together.
    out.sort_by(|a, b| b.severity.cmp(&a.severity));
    out
}

pub fn summarise(findings: &[Finding]) -> String {
    let crit: Vec<_> = findings.iter().filter(|f| f.severity == Severity::Critical).collect();
    let warn: Vec<_> = findings.iter().filter(|f| f.severity == Severity::Warn).collect();
    if let Some(first) = crit.first() {
        return i18n::scan_critical(crit.len(), &first.title);
    }
    if let Some(first) = warn.first() {
        return i18n::scan_warnings(warn.len(), &first.title);
    }
    i18n::scan_all_healthy().into()
}

// ---------------------------------------------------------------------------

fn check_medium(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    if net.adapter_name.is_empty() {
        return vec![Finding::new(
            "medium",
            i18n::f_no_connection(),
            Severity::Critical,
            i18n::f_no_connection_detail(),
        )
        .advise(i18n::f_no_connection_advice())];
    }
    match net.medium {
        Medium::Ethernet => vec![Finding::new(
            "medium",
            i18n::f_wired(),
            Severity::Good,
            i18n::f_wired_detail(&net.adapter_name, net.link_speed_mbps),
        )
        .advise(i18n::f_wired_advice())],
        _ => vec![Finding::new(
            "medium",
            i18n::f_wifi(),
            Severity::Info,
            i18n::f_wifi_detail(&net.adapter_name, &net.ssid, net.link_speed_mbps),
        )
        .advise(i18n::f_wifi_advice())],
    }
}

fn check_wifi(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    if net.medium != Medium::Wifi {
        return Vec::new();
    }
    let Some(sig) = net.signal_pct else {
        return Vec::new();
    };
    let band = net.band().unwrap_or("?");
    let rssi = net
        .rssi_dbm
        .map(|r| format!(", {r} dBm"))
        .unwrap_or_default();
    let detail = i18n::f_wifi_quality_detail(
        band,
        &net.channel.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        &net.phy,
        &rssi,
        net.rx_mbps.unwrap_or(0),
    );

    let mut out = Vec::new();
    if sig < 45 {
        out.push(
            Finding::new("signal", i18n::f_signal_weak(sig), Severity::Critical, detail.clone())
                .advise(i18n::f_signal_weak_advice()),
        );
    } else if sig < 65 {
        out.push(
            Finding::new("signal", i18n::f_signal_mid(sig), Severity::Warn, detail.clone())
                .advise(i18n::f_signal_mid_advice()),
        );
    } else {
        out.push(Finding::new(
            "signal",
            i18n::f_signal_good(sig),
            Severity::Good,
            detail.clone(),
        ));
    }

    if band == "2.4 GHz" {
        out.push(
            Finding::new("band", i18n::f_band_24(), Severity::Warn, detail)
                .advise(i18n::f_band_24_advice()),
        );
    }
    out
}

fn check_power(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    use crate::optimize::{AdapterPowerSaving, Tweak};
    let state = AdapterPowerSaving.read(net);
    match state.optimal {
        Some(false) => vec![Finding::new(
            "power",
            i18n::f_power_bad(),
            Severity::Critical,
            state.text,
        )
        .advise(i18n::f_power_bad_advice())
        .fixed_by("adapter_power")],
        Some(true) => vec![Finding::new(
            "power",
            i18n::f_power_good(),
            Severity::Good,
            state.text,
        )],
        None => vec![Finding::new(
            "power",
            i18n::f_power_unknown(),
            Severity::Info,
            state.text,
        )],
    }
}

fn check_dns(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    use crate::probe::netstate::dns_lookup_ms;
    use crate::settings::DNS_TEST_HOST;

    let servers = if net.dns_servers.is_empty() {
        i18n::f_dns_from_dhcp().to_string()
    } else {
        net.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ")
    };

    let mut out = Vec::new();
    let (ms, err) = dns_lookup_ms(DNS_TEST_HOST);
    match (ms, err.is_empty()) {
        (_, false) => out.push(
            Finding::new("dns_resolve", i18n::f_dns_failing(), Severity::Critical, err)
                .advise(i18n::f_dns_failing_advice())
                .fixed_by("fast_dns"),
        ),
        (Some(ms), true) if ms > 150.0 => out.push(
            Finding::new(
                "dns_slow",
                i18n::f_dns_slow(ms),
                Severity::Warn,
                i18n::f_dns_servers(&servers),
            )
            .advise(i18n::f_dns_slow_advice())
            .fixed_by("fast_dns"),
        ),
        (Some(ms), true) => out.push(Finding::new(
            "dns_ok",
            i18n::f_dns_ok(ms),
            Severity::Good,
            i18n::f_dns_servers(&servers),
        )),
        (None, true) => out.push(Finding::new(
            "dns_unknown",
            i18n::f_dns_unknown(),
            Severity::Info,
            i18n::f_dns_servers(&servers),
        )),
    }

    if net.dns_is_router_only() {
        out.push(
            Finding::new(
                "dns_router",
                i18n::f_dns_router_only(),
                Severity::Warn,
                i18n::f_dns_servers(&servers),
            )
            .advise(i18n::f_dns_router_only_advice())
            .fixed_by("fast_dns"),
        );
    }
    out
}

fn check_local_link(net: &NetState, _s: &Store, cfg: &Settings) -> Vec<Finding> {
    let Some(gw) = net.gateway else {
        return vec![Finding::new(
            "gateway",
            i18n::f_no_gateway(),
            Severity::Critical,
            i18n::f_no_gateway_detail(),
        )
        .advise(i18n::f_no_gateway_advice())];
    };

    let samples = icmp::ping_series(gw, 10, cfg.ping_timeout_ms);
    let rtts: Vec<f64> = samples.iter().flatten().copied().collect();
    let stats = store::summarise(samples.len(), &rtts);

    if rtts.is_empty() {
        return vec![Finding::new(
            "gateway",
            i18n::f_router_silent(),
            Severity::Critical,
            i18n::f_router_silent_detail(&gw.to_string()),
        )
        .advise(i18n::f_router_silent_advice())];
    }

    let detail = i18n::f_stats_line(
        stats.avg.unwrap_or(0.0),
        stats.min.unwrap_or(0.0),
        stats.max.unwrap_or(0.0),
        stats.jitter.unwrap_or(0.0),
        stats.loss_pct,
    );

    if stats.loss_pct > 0.0 || stats.avg.unwrap_or(0.0) > 15.0 || stats.jitter.unwrap_or(0.0) > 10.0
    {
        vec![Finding::new("gateway", i18n::f_link_unstable(), Severity::Warn, detail)
            .advise(i18n::f_link_unstable_advice())]
    } else {
        vec![Finding::new("gateway", i18n::f_link_healthy(), Severity::Good, detail)]
    }
}

fn check_internet(_net: &NetState, _s: &Store, cfg: &Settings) -> Vec<Finding> {
    let host = Ipv4Addr::new(1, 1, 1, 1);
    let samples = icmp::ping_series(host, 15, cfg.ping_timeout_ms);
    let rtts: Vec<f64> = samples.iter().flatten().copied().collect();
    let stats = store::summarise(samples.len(), &rtts);

    if rtts.is_empty() {
        return vec![Finding::new(
            "internet",
            i18n::f_net_silent(),
            Severity::Critical,
            i18n::f_net_silent_detail(),
        )
        .advise(i18n::f_net_silent_advice())];
    }

    let avg = stats.avg.unwrap_or(0.0);
    let jitter = stats.jitter.unwrap_or(0.0);
    let spread = stats.max.unwrap_or(0.0) - stats.min.unwrap_or(0.0);
    let detail = i18n::f_stats_line(
        avg,
        stats.min.unwrap_or(0.0),
        stats.max.unwrap_or(0.0),
        jitter,
        stats.loss_pct,
    );

    let mut out = Vec::new();
    if stats.loss_pct > cfg.loss_ok_pct {
        out.push(
            Finding::new(
                "loss",
                i18n::f_loss(stats.loss_pct),
                Severity::Critical,
                detail.clone(),
            )
            .advise(i18n::f_loss_advice()),
        );
    }
    if jitter > cfg.jitter_ok_ms {
        out.push(
            Finding::new("jitter", i18n::f_jitter_high(jitter), Severity::Warn, detail.clone())
                .advise(i18n::f_jitter_high_advice(spread)),
        );
    }
    if avg > cfg.ping_bad_ms {
        out.push(
            Finding::new("ping", i18n::f_ping_high(avg), Severity::Warn, detail.clone())
                .advise(i18n::f_ping_high_advice()),
        );
    }
    if out.is_empty() {
        out.push(Finding::new(
            "internet",
            i18n::f_net_ok(avg),
            Severity::Good,
            detail,
        ));
    }
    out
}

fn check_mtu(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    use crate::optimize::{MtuFix, Tweak};
    let state = MtuFix.read(net);
    let current = state.snapshot["mtu"].as_u64().map(|v| v as u32);
    let Some(best) = MtuFix::probe_best_mtu(Ipv4Addr::new(1, 1, 1, 1)) else {
        return vec![Finding::new(
            "mtu",
            i18n::f_mtu_unmeasured(),
            Severity::Info,
            i18n::f_mtu_unmeasured_detail(),
        )];
    };

    match current {
        Some(cur) if best < cur => vec![Finding::new(
            "mtu",
            i18n::f_mtu_too_large(),
            Severity::Warn,
            i18n::f_mtu_too_large_detail(cur, best),
        )
        .advise(i18n::f_mtu_too_large_advice(best))
        .fixed_by("mtu")],
        _ => vec![Finding::new(
            "mtu",
            i18n::f_mtu_ok(current.unwrap_or(best)),
            Severity::Good,
            i18n::f_mtu_ok_detail(best),
        )],
    }
}

fn check_tcp(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    use crate::optimize::{NetworkThrottling, TcpAutotuning, Tweak};
    let mut out = Vec::new();
    for (tweak, good) in [
        (&TcpAutotuning as &dyn Tweak, i18n::f_tcp_autotuning_ok()),
        (&NetworkThrottling as &dyn Tweak, i18n::f_throttle_ok()),
    ] {
        let state = tweak.read(net);
        match state.optimal {
            Some(true) => out.push(Finding::new(tweak.id(), good, Severity::Good, state.text)),
            Some(false) => out.push(
                Finding::new(tweak.id(), tweak.title(), Severity::Info, state.text)
                    .advise(tweak.why())
                    .fixed_by(tweak.id()),
            ),
            None => {}
        }
    }
    out
}

fn check_history(_net: &NetState, store: &Store, _cfg: &Settings) -> Vec<Finding> {
    let events = store.events_since(24.0 * 3600.0);
    if events.is_empty() {
        return vec![Finding::new(
            "history",
            i18n::f_hist_none(),
            Severity::Good,
            i18n::f_hist_none_detail(),
        )];
    }

    // A BTreeMap rather than a HashMap: the sort below is stable, so it
    // preserves whatever order the grouping produced, and a HashMap's order is
    // seeded per map. Equally frequent scopes then swapped places between
    // scans and the "most important finding" headline changed for no reason.
    let mut by_scope: std::collections::BTreeMap<String, Vec<&store::Event>> =
        std::collections::BTreeMap::new();
    for e in &events {
        by_scope.entry(e.scope.clone()).or_default().push(e);
    }

    let mut out = Vec::new();
    let mut scopes: Vec<_> = by_scope.into_iter().collect();
    scopes.sort_by_key(|(scope, v)| {
        std::cmp::Reverse((v.len(), monitor::scope_rank(scope)))
    });

    for (scope, items) in scopes {
        let (title, advice) = match scope.as_str() {
            "lan" => (i18n::f_hist_lan(), i18n::f_hist_lan_advice()),
            "adapter" => (i18n::f_hist_adapter(), i18n::f_hist_adapter_advice()),
            "isp" => (i18n::f_hist_isp(), i18n::f_hist_isp_advice()),
            "dns" => (i18n::f_hist_dns(), i18n::f_hist_dns_advice()),
            _ => (i18n::f_hist_other(), i18n::f_hist_other_advice()),
        };

        let durations: Vec<f64> = items.iter().filter_map(|e| e.duration_s()).collect();
        let avg = if durations.is_empty() {
            0.0
        } else {
            durations.iter().sum::<f64>() / durations.len() as f64
        };
        let times: Vec<String> = items
            .iter()
            .take(6)
            .map(|e| format_clock(e.ts_start))
            .collect();

        out.push(
            Finding::new(
                &format!("hist_{scope}"),
                i18n::f_hist_title(title, items.len()),
                if items.len() >= 3 { Severity::Critical } else { Severity::Warn },
                i18n::f_hist_detail(avg, &times.join(", ")),
            )
            .advise(advice),
        );
    }
    out
}

/// Local wall-clock HH:MM for a unix timestamp, without pulling in a date crate.
pub fn format_clock(ts: f64) -> String {
    let secs_of_day = (ts as i64).rem_euclid(86400);
    // SystemTime has no timezone, so ask Windows for the local offset once.
    let offset = local_utc_offset_secs();
    let local = (secs_of_day + offset).rem_euclid(86400);
    format!("{:02}:{:02}", local / 3600, (local % 3600) / 60)
}

pub fn format_datetime(ts: f64) -> String {
    let offset = local_utc_offset_secs();
    let t = ts as i64 + offset;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02} {y:04} {:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// Hour of the day, 0-23, in the machine's own time zone. Grouping outages by
/// hour only says anything if the hour is the one the user lives in.
pub fn local_hour(ts: f64) -> i64 {
    (ts as i64 + local_utc_offset_secs()).div_euclid(3600).rem_euclid(24)
}

fn local_utc_offset_secs() -> i64 {
    use windows::Win32::System::Time::GetTimeZoneInformation;
    use windows::Win32::System::Time::TIME_ZONE_INFORMATION;
    unsafe {
        let mut tz = TIME_ZONE_INFORMATION::default();
        let rc = GetTimeZoneInformation(&mut tz);
        // Bias is minutes to ADD to local to get UTC, so invert it.
        let extra = match rc {
            2 => tz.DaylightBias, // TIME_ZONE_ID_DAYLIGHT
            _ => tz.StandardBias,
        };
        -((tz.Bias + extra) as i64) * 60
    }
}

/// Howard Hinnant's days-from-civil, inverted. Avoids a date dependency.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_come_back_worst_first() {
        let mut f = vec![
            Finding::new("a", "good", Severity::Good, ""),
            Finding::new("b", "critical", Severity::Critical, ""),
            Finding::new("c", "warn", Severity::Warn, ""),
        ];
        f.sort_by(|a, b| b.severity.cmp(&a.severity));
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[2].severity, Severity::Good);
    }

    #[test]
    fn summary_leads_with_the_worst_problem() {
        let f = vec![
            Finding::new("a", "Adapter sleeping", Severity::Critical, ""),
            Finding::new("b", "Slow DNS", Severity::Warn, ""),
        ];
        assert!(summarise(&f).contains("Adapter sleeping"));

        let clean = vec![Finding::new("a", "All good", Severity::Good, "")];
        assert!(summarise(&clean).contains("healthy"));
    }

    #[test]
    fn civil_date_conversion_matches_known_dates() {
        // 2024-02-29 was day 19782 since the epoch.
        assert_eq!(civil_from_days(19782), (2024, 2, 29));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn a_full_scan_of_the_live_machine_produces_findings() {
        let net = crate::probe::netstate::read();
        let store = Store::open_in_memory().unwrap();
        let findings = scan(&net, &store, &Settings::default(), None);
        assert!(!findings.is_empty());
        assert!(findings.windows(2).all(|w| w[0].severity >= w[1].severity));
        // Every finding must be presentable: a title and something to show.
        for f in &findings {
            assert!(!f.title.is_empty(), "finding {} has no title", f.key);
        }
    }
}
