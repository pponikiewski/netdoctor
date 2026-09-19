//! One-shot diagnostic scan.
//!
//! Each check produces Findings. Severity drives ordering in the UI, and a
//! finding may point at the tweak that fixes it.

use std::net::Ipv4Addr;
use std::sync::Arc;

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
            Severity::Critical => "CRITICAL",
            Severity::Warn => "WARNING",
            Severity::Info => "INFO",
            Severity::Good => "OK",
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
        ("Checking adapter and medium", check_medium),
        ("Checking Wi-Fi quality", check_wifi),
        ("Checking adapter power management", check_power),
        ("Checking DNS configuration", check_dns),
        ("Measuring the link to the router", check_local_link),
        ("Measuring internet latency", check_internet),
        ("Checking MTU", check_mtu),
        ("Checking TCP settings", check_tcp),
        ("Reviewing outage history", check_history),
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
        p("Done", 1.0);
    }

    // Worst first, stable within a severity so related findings stay together.
    out.sort_by(|a, b| b.severity.cmp(&a.severity));
    out
}

pub fn summarise(findings: &[Finding]) -> String {
    let crit: Vec<_> = findings.iter().filter(|f| f.severity == Severity::Critical).collect();
    let warn: Vec<_> = findings.iter().filter(|f| f.severity == Severity::Warn).collect();
    if let Some(first) = crit.first() {
        return format!("{} serious problem(s) found. Most important: {}", crit.len(), first.title);
    }
    if let Some(first) = warn.first() {
        return format!(
            "No failures, but {} thing(s) could be improved. Most important: {}",
            warn.len(),
            first.title
        );
    }
    "The network looks healthy — nothing needs attention.".into()
}

// ---------------------------------------------------------------------------

fn check_medium(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    if net.adapter_name.is_empty() {
        return vec![Finding::new(
            "medium",
            "No active connection",
            Severity::Critical,
            "No adapter holds a default route.",
        )
        .advise("Check the cable, or turn Wi-Fi on.")];
    }
    match net.medium {
        Medium::Ethernet => vec![Finding::new(
            "medium",
            "Wired connection",
            Severity::Good,
            format!("{}, {} Mbps", net.adapter_name, net.link_speed_mbps),
        )
        .advise("The best possible starting point for latency.")],
        _ => vec![Finding::new(
            "medium",
            "Connected over Wi-Fi",
            Severity::Info,
            format!(
                "{} — {}, {} Mbps link rate",
                net.adapter_name, net.ssid, net.link_speed_mbps
            ),
        )
        .advise(
            "Wi-Fi always has higher jitter than a cable and is vulnerable to interference. \
             If the drops happen mostly while gaming, a cable removes several causes at once.",
        )],
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
    let detail = format!(
        "Band {band}, channel {}, {}{rssi}, {} Mbps receive",
        net.channel.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        net.phy,
        net.rx_mbps.unwrap_or(0)
    );

    let mut out = Vec::new();
    if sig < 45 {
        out.push(
            Finding::new("signal", format!("Weak Wi-Fi signal ({sig}%)"), Severity::Critical, detail.clone())
                .advise(
                    "At this level the card drops frames and will periodically disconnect. Move \
                     closer, reposition the router, or switch to 2.4 GHz for range at the cost of speed.",
                ),
        );
    } else if sig < 65 {
        out.push(
            Finding::new("signal", format!("Mediocre Wi-Fi signal ({sig}%)"), Severity::Warn, detail.clone())
                .advise("Fine for browsing, but latency will spike under load."),
        );
    } else {
        out.push(Finding::new(
            "signal",
            format!("Wi-Fi signal is good ({sig}%)"),
            Severity::Good,
            detail.clone(),
        ));
    }

    if band == "2.4 GHz" {
        out.push(
            Finding::new("band", "Running on the 2.4 GHz band", Severity::Warn, detail)
                .advise(
                    "2.4 GHz is shared with microwaves, Bluetooth and every neighbour. If the \
                     router offers 5 GHz, connect to that SSID instead.",
                ),
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
            "Windows is allowed to power down the network adapter",
            Severity::Critical,
            state.text,
        )
        .advise(
            "This is the most common cause of drops \"out of nowhere\" on a laptop: the card \
             suspends while idle and takes seconds to come back. Fix it on the Optimise tab.",
        )
        .fixed_by("adapter_power")],
        Some(true) => vec![Finding::new(
            "power",
            "Adapter power saving is disabled",
            Severity::Good,
            state.text,
        )],
        None => vec![Finding::new(
            "power",
            "Could not read adapter power management",
            Severity::Info,
            state.text,
        )],
    }
}

fn check_dns(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    use crate::probe::netstate::dns_lookup_ms;
    use crate::settings::DNS_TEST_HOST;

    let servers = if net.dns_servers.is_empty() {
        "from DHCP".to_string()
    } else {
        net.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ")
    };

    let mut out = Vec::new();
    let (ms, err) = dns_lookup_ms(DNS_TEST_HOST);
    match (ms, err.is_empty()) {
        (_, false) => out.push(
            Finding::new("dns_resolve", "Name resolution is failing", Severity::Critical, err)
                .advise(
                    "Pings by IP may work while nothing loads in a browser. Switch to 1.1.1.1 on \
                     the Optimise tab.",
                )
                .fixed_by("fast_dns"),
        ),
        (Some(ms), true) if ms > 150.0 => out.push(
            Finding::new(
                "dns_slow",
                format!("Slow DNS ({ms:.0} ms)"),
                Severity::Warn,
                format!("Servers: {servers}"),
            )
            .advise("Every new connection waits on this, which is why pages seem to stall before loading.")
            .fixed_by("fast_dns"),
        ),
        (Some(ms), true) => out.push(Finding::new(
            "dns_ok",
            format!("DNS responds quickly ({ms:.0} ms)"),
            Severity::Good,
            format!("Servers: {servers}"),
        )),
        (None, true) => out.push(Finding::new(
            "dns_unknown",
            "DNS timing unavailable",
            Severity::Info,
            format!("Servers: {servers}"),
        )),
    }

    if net.dns_is_router_only() {
        out.push(
            Finding::new(
                "dns_router",
                "The router is the only DNS server",
                Severity::Warn,
                format!("DNS: {servers}"),
            )
            .advise(
                "When the router stalls or reboots this looks exactly like an internet outage. \
                 Adding 1.1.1.1 as a second resolver removes that single point of failure.",
            )
            .fixed_by("fast_dns"),
        );
    }
    out
}

fn check_local_link(net: &NetState, _s: &Store, cfg: &Settings) -> Vec<Finding> {
    let Some(gw) = net.gateway else {
        return vec![Finding::new(
            "gateway",
            "No default gateway",
            Severity::Critical,
            "This machine has no route to the network.",
        )
        .advise("Check DHCP on the router.")];
    };

    let samples = icmp::ping_series(gw, 10, cfg.ping_timeout_ms);
    let rtts: Vec<f64> = samples.iter().flatten().copied().collect();
    let stats = store::summarise(samples.len(), &rtts);

    if rtts.is_empty() {
        return vec![Finding::new(
            "gateway",
            "The router is not answering",
            Severity::Critical,
            format!("10/10 packets lost to {gw}."),
        )
        .advise("The problem is between this PC and the router, not at the ISP.")];
    }

    let detail = format!(
        "avg {:.1} ms, min {:.1}, max {:.1}, jitter {:.1} ms, loss {:.0}%",
        stats.avg.unwrap_or(0.0),
        stats.min.unwrap_or(0.0),
        stats.max.unwrap_or(0.0),
        stats.jitter.unwrap_or(0.0),
        stats.loss_pct
    );

    if stats.loss_pct > 0.0 || stats.avg.unwrap_or(0.0) > 15.0 || stats.jitter.unwrap_or(0.0) > 10.0
    {
        vec![Finding::new("gateway", "The link to the router is unstable", Severity::Warn, detail)
            .advise(
                "A ping to your own router should be under 5 ms with no loss. This points at the \
                 PC-to-router hop — Wi-Fi, cabling, or an overloaded router — not at the ISP.",
            )]
    } else {
        vec![Finding::new("gateway", "The link to the router is healthy", Severity::Good, detail)]
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
            "No response from the internet",
            Severity::Critical,
            "15/15 packets lost to 1.1.1.1.",
        )
        .advise("If the router still answers, the fault is on the WAN/ISP side.")];
    }

    let avg = stats.avg.unwrap_or(0.0);
    let jitter = stats.jitter.unwrap_or(0.0);
    let spread = stats.max.unwrap_or(0.0) - stats.min.unwrap_or(0.0);
    let detail = format!(
        "avg {avg:.1} ms, min {:.1}, max {:.1}, jitter {jitter:.1} ms, loss {:.0}%",
        stats.min.unwrap_or(0.0),
        stats.max.unwrap_or(0.0),
        stats.loss_pct
    );

    let mut out = Vec::new();
    if stats.loss_pct > cfg.loss_ok_pct {
        out.push(
            Finding::new(
                "loss",
                format!("Packet loss {:.0}%", stats.loss_pct),
                Severity::Critical,
                detail.clone(),
            )
            .advise(
                "Above 2% games start to stutter and TCP throughput collapses. Check the router \
                 link first — if that is clean, the problem is further upstream.",
            ),
        );
    }
    if jitter > cfg.jitter_ok_ms {
        out.push(
            Finding::new("jitter", format!("High jitter ({jitter:.0} ms)"), Severity::Warn, detail.clone())
                .advise(format!(
                    "Latency swings by {spread:.0} ms between packets. This is what \"lagging \
                     despite a good ping\" actually is — typical of Wi-Fi and of a saturated link."
                )),
        );
    }
    if avg > cfg.ping_bad_ms {
        out.push(
            Finding::new("ping", format!("High latency ({avg:.0} ms)"), Severity::Warn, detail.clone())
                .advise("Run a traceroute to see which hop the delay appears at."),
        );
    }
    if out.is_empty() {
        out.push(Finding::new(
            "internet",
            format!("Internet latency is normal ({avg:.0} ms)"),
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
            "Could not measure MTU",
            Severity::Info,
            "The test needs ICMP with the don't-fragment flag, which some networks block.",
        )];
    };

    match current {
        Some(cur) if best < cur => vec![Finding::new(
            "mtu",
            "MTU is larger than the path supports",
            Severity::Warn,
            format!("Configured {cur}, but only {best} passes without fragmentation."),
        )
        .advise(format!(
            "Packets above {best} get dropped along the way. The symptom is that ping works but \
             some pages never finish loading. Note that some paths rate-limit the probe, so it is \
             worth re-running this before changing anything."
        ))
        .fixed_by("mtu")],
        _ => vec![Finding::new(
            "mtu",
            format!("MTU looks correct ({})", current.unwrap_or(best)),
            Severity::Good,
            format!("Largest unfragmented packet corresponds to MTU {best}."),
        )],
    }
}

fn check_tcp(net: &NetState, _s: &Store, _cfg: &Settings) -> Vec<Finding> {
    use crate::optimize::{NetworkThrottling, TcpAutotuning, Tweak};
    let mut out = Vec::new();
    for (tweak, good) in [
        (&TcpAutotuning as &dyn Tweak, "TCP auto-tuning is set correctly"),
        (&NetworkThrottling as &dyn Tweak, "Multimedia packet throttle is lifted"),
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
            "No outages recorded in the last 24 hours",
            Severity::Good,
            "The monitor logs every interruption — leave it running to catch the next one.",
        )];
    }

    let mut by_scope: std::collections::HashMap<String, Vec<&store::Event>> =
        std::collections::HashMap::new();
    for e in &events {
        by_scope.entry(e.scope.clone()).or_default().push(e);
    }

    let mut out = Vec::new();
    let mut scopes: Vec<_> = by_scope.into_iter().collect();
    scopes.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));

    for (scope, items) in scopes {
        let (title, advice) = match scope.as_str() {
            "lan" => (
                "Drops between this PC and the router",
                "The culprit is Wi-Fi, the adapter, or the router itself. Start with adapter \
                 power management and a driver update.",
            ),
            "adapter" => (
                "The adapter lost its network association",
                "The card disconnected from the SSID. That is the driver, power saving, or too \
                 weak a signal.",
            ),
            "isp" => (
                "Drops on the WAN/ISP side",
                "The router answered but the internet did not. No Windows setting fixes this — \
                 it is evidence for a support ticket. Show them these timestamps.",
            ),
            "dns" => ("DNS failures", "The link was up but names would not resolve. Changing DNS fixes this."),
            _ => ("Periods of degraded quality", "The connection worked, but with lag and loss."),
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
                format!("{title} — {}× in the last 24 h", items.len()),
                if items.len() >= 3 { Severity::Critical } else { Severity::Warn },
                format!("Average duration {avg:.0} s. Most recent: {}", times.join(", ")),
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
