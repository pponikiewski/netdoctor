//! What Windows itself recorded around an outage.
//!
//! Every other probe in this app infers. It pings, it watches a counter, it
//! reads a trend and argues from it. Meanwhile the operating system was
//! writing down what actually happened — that the access point deauthenticated
//! us and why, that the adapter driver faulted and reset, that the machine
//! went to sleep, that the DHCP lease could not be renewed — and nothing ever
//! read it back.
//!
//! This module reads it back. A line from the event log is not a deduction
//! from latency: it is the OS naming the event, with its own reason code. That
//! makes it the strongest evidence available on Windows, and it costs one
//! process launch per outage the user actually opens.
//!
//! Two channels are queried. `System` carries power transitions, the network
//! profile going up and down, DHCP, duplicate addresses and driver faults.
//! `Microsoft-Windows-WLAN-AutoConfig/Operational` carries the wireless story
//! proper, including the 802.11 reason code on a disconnect, which is the one
//! field that separates "we were pushed off" from "we walked away".
//!
//! The rendered message text is deliberately never used: `wevtutil` localises
//! it, so matching on it would work on an English machine and quietly fail
//! everywhere else — the same trap the `netsh` and `powercfg` readers already
//! avoid. Only structured fields are parsed: provider, id, level, timestamp
//! and the `EventData` name/value pairs.

use std::collections::BTreeMap;

/// What a logged event means for a connection. The raw provider and id are
/// kept alongside, so an event that maps to nothing specific can still be
/// shown rather than silently dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The machine suspended. Everything that follows is explained by it.
    Sleep,
    /// The machine woke. A drop right after this is a resume race, not a fault.
    Resume,
    /// The wireless link went down, usually with an 802.11 reason code.
    WlanDisconnect,
    /// Association or authentication was refused.
    WlanAuthFail,
    WlanConnect,
    /// The OS saw the interface itself go down — not merely unreachable.
    LinkDown,
    LinkUp,
    DhcpFail,
    DuplicateIp,
    /// The adapter's own driver logged an error or reset itself.
    DriverFault,
}

impl Kind {
    /// Whether this is a fault worth surfacing on its own, as opposed to a
    /// transition that only matters next to one.
    pub fn is_fault(&self) -> bool {
        matches!(
            self,
            Kind::WlanDisconnect
                | Kind::WlanAuthFail
                | Kind::LinkDown
                | Kind::DhcpFail
                | Kind::DuplicateIp
                | Kind::DriverFault
        )
    }
}

#[derive(Debug, Clone)]
pub struct SysEvent {
    pub ts: f64,
    pub provider: String,
    pub id: u32,
    pub kind: Kind,
    /// The 802.11 or WLAN reason code on a disconnect, where one was logged.
    pub reason: Option<u32>,
    /// The structured fields worth showing, already flattened to `k=v`.
    pub detail: String,
}

impl SysEvent {
    /// Seconds between this log line and a moment of interest; negative means
    /// the log line came first.
    pub fn offset_from(&self, ts: f64) -> f64 {
        self.ts - ts
    }
}

/// Everything both channels recorded between two unix timestamps, oldest
/// first.
///
/// An empty result is not an error and is not distinguished from one: a
/// machine with the WLAN channel disabled, a wired machine, and a genuinely
/// quiet ten minutes all mean the same thing here — the log has nothing to
/// add, so the other evidence stands alone.
pub fn window(from: f64, to: f64) -> Vec<SysEvent> {
    let query = format!(
        "*[System[TimeCreated[@SystemTime>='{}' and @SystemTime<='{}']]]",
        iso_utc(from),
        iso_utc(to)
    );

    let mut out = Vec::new();
    for channel in ["System", "Microsoft-Windows-WLAN-AutoConfig/Operational"] {
        out.extend(read_channel(channel, &query));
    }
    out.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// How many events a single channel may contribute. A window around one
/// outage holds a handful; the cap is there so a machine logging a fault every
/// second cannot turn a click into a stall.
const MAX_PER_CHANNEL: usize = 200;

/// The command line, built apart from the call so a test can run the real one
/// rather than a copy of it that could drift.
fn channel_args(channel: &str, query: &str) -> [String; 5] {
    [
        "qe".to_string(),
        channel.to_string(),
        format!("/q:{query}"),
        "/f:xml".to_string(),
        format!("/c:{MAX_PER_CHANNEL}"),
    ]
}

fn read_channel(channel: &str, query: &str) -> Vec<SysEvent> {
    let owned = channel_args(channel, query);
    let args: Vec<&str> = owned.iter().map(String::as_str).collect();

    // A channel that does not exist, or that this user may not read, exits
    // non-zero. That is a fact about the machine, not a failure worth
    // reporting: the caller gets fewer lines of evidence and says so.
    let Ok(xml) = crate::optimize::run("wevtutil", &args) else {
        return Vec::new();
    };

    xml.split("<Event ").skip(1).filter_map(parse_event).collect()
}

fn parse_event(chunk: &str) -> Option<SysEvent> {
    let provider = attr(chunk, "<Provider ", "Name")?;
    let id: u32 = text(chunk, "EventID")?.parse().ok()?;
    let ts = parse_iso_utc(&attr(chunk, "<TimeCreated ", "SystemTime")?)?;
    let level: u32 = text(chunk, "Level").and_then(|v| v.parse().ok()).unwrap_or(4);

    let data = event_data(chunk);
    let kind = classify(&provider, id, level)?;
    let reason = reason_code(&data);

    Some(SysEvent { ts, provider, id, kind, reason, detail: flatten(&data) })
}

/// The disconnect reason, as a number.
///
/// A record carries both a `Reason` — a sentence, which the provider
/// localises, so it is never read — and a `ReasonCode`, which is the number
/// this app reports and a support line asks for. Looking for the first field
/// whose name merely starts with "reason" finds the sentence, fails to parse
/// it, and throws away the code sitting two fields later; hence the explicit
/// order. Code zero means the provider logged no reason at all, which is not
/// the same as reason number zero and must not be printed as one.
fn reason_code(data: &[(String, String)]) -> Option<u32> {
    for wanted in ["reasoncode", "failurereason", "reason"] {
        let found = data
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == wanted)
            .and_then(|(_, v)| parse_code(v))
            .filter(|c| *c != 0);
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Which (provider, id) pairs mean something here.
///
/// The list is deliberately short and explicit. A table of every id Windows
/// can emit would be a maintenance burden and mostly noise; these are the ones
/// that answer "why did the connection stop", and everything else only gets in
/// if the driver logged it as an error.
fn classify(provider: &str, id: u32, level: u32) -> Option<Kind> {
    let p = provider.to_ascii_lowercase();
    let short = p.rsplit('-').next().unwrap_or(&p);

    let known = match (p.as_str(), id) {
        // 42 is classic S3 sleep. 506 is entering Modern Standby, which is how
        // most laptops sold since ~2020 sleep; without it their sleep was only
        // ever visible as the 507 that ends it.
        ("microsoft-windows-kernel-power", 42 | 506) => Some(Kind::Sleep),
        ("microsoft-windows-kernel-power", 107 | 507) => Some(Kind::Resume),
        ("microsoft-windows-power-troubleshooter", 1) => Some(Kind::Resume),
        ("microsoft-windows-networkprofile", 10000) => Some(Kind::LinkUp),
        ("microsoft-windows-networkprofile", 10001) => Some(Kind::LinkDown),
        ("microsoft-windows-wlan-autoconfig", 8001) => Some(Kind::WlanConnect),
        ("microsoft-windows-wlan-autoconfig", 8003) => Some(Kind::WlanDisconnect),
        // 8002 is a failure to connect, 11006 an 802.1X failure, 12013 an MSM
        // security failure. 11004 is *not* here on purpose: on real hardware
        // it is logged during every successful association, and treating it
        // as a fault produced a confident "authentication failed" on a link
        // that had just come up cleanly.
        ("microsoft-windows-wlan-autoconfig", 8002 | 11006 | 12013) => Some(Kind::WlanAuthFail),
        ("tcpip" | "tcpip6", 4198 | 4199) => Some(Kind::DuplicateIp),
        _ => None,
    };
    if known.is_some() {
        return known;
    }

    // DHCPv6 logs its service starting and stopping on every network, with
    // IPv6 or without, and a network without IPv6 has no v6 lease to lose.
    // The monitor measures IPv4, so nothing it logs explains an outage here.
    if p.contains("dhcpv6") {
        return None;
    }
    if matches!(short, "dhcp") || p.contains("dhcp-client") {
        // The DHCP client logs its successes too; only a failure to get or
        // renew a lease explains an outage. The ids are the ones its manifest
        // (`Get-WinEvent -ListProvider Microsoft-Windows-Dhcp-Client`) marks
        // as errors and warnings about the lease: 1001 no address, 1002
        // refused (NACK), 1003 not renewed, 1006 not configured. 1005 in the
        // same manifest is an address already in use. 1046, 1047, 50066 and
        // 50067 used to be here and are informational: the fallback
        // configuration read, an address attached for an SSID.
        return match id {
            1001 | 1002 | 1003 | 1006 => Some(Kind::DhcpFail),
            1005 => Some(Kind::DuplicateIp),
            _ => None,
        };
    }

    // Anything else has to be an error or worse, from something that plausibly
    // is the network stack, or it is not evidence — it is scrollback.
    (level <= 2 && looks_like_network(&p)).then_some(Kind::DriverFault)
}

/// Provider-name fragments that belong to a network adapter driver or the
/// stack above it. Matching a name rather than a documented id is a guess, so
/// it is only ever applied to events the provider already flagged as errors.
const NETWORK_PROVIDERS: [&str; 18] = [
    "netwtw",
    "netwlv",
    "netwns",
    "netwbw",
    "e1dexpress",
    "e1inexpress",
    "e1rexpress",
    "rtlwlan",
    "rt640",
    "athr",
    "bcmpcie",
    "bcmwl",
    "qcamain",
    "mrvlpcie",
    "vwifi",
    "ndis",
    "netbt",
    "wlan",
];

fn looks_like_network(provider_lower: &str) -> bool {
    NETWORK_PROVIDERS.iter().any(|n| provider_lower.contains(n))
        || provider_lower.contains("wifi")
        || provider_lower.contains("ethernet")
}

// ---------------------------------------------------------------------------
// a scanner, not an XML parser
// ---------------------------------------------------------------------------
//
// The shape of `wevtutil /f:xml` output is fixed and shallow, and the five
// fields wanted from it are all attributes or leaf text. Pulling in an XML
// crate to find them would add a dependency and a build to a binary whose
// whole pitch is that it has neither.

fn attr(chunk: &str, tag: &str, name: &str) -> Option<String> {
    let start = chunk.find(tag)? + tag.len();
    let rest = &chunk[start..];
    let end = rest.find('>')?;
    let inside = &rest[..end];

    let at = inside.find(&format!("{name}="))? + name.len() + 1;
    let after = &inside[at..];
    let quote = after.chars().next()?;
    let value_end = after[1..].find(quote)?;
    Some(unescape(&after[1..1 + value_end]))
}

fn text(chunk: &str, tag: &str) -> Option<String> {
    let open = chunk.find(&format!("<{tag}"))?;
    let rest = &chunk[open..];
    let body = rest.find('>')? + 1;
    let close = rest.find(&format!("</{tag}>"))?;
    (close > body).then(|| unescape(rest[body..close].trim()))
}

/// The `<Data Name='x'>v</Data>` pairs, in the order logged. Unnamed `Data`
/// elements — older providers emit positional data — are keyed by position so
/// they are not silently merged into one.
fn event_data(chunk: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = chunk;
    let mut positional = 0;

    while let Some(i) = rest.find("<Data") {
        rest = &rest[i..];
        let Some(open_end) = rest.find('>') else { break };
        let head = &rest[..open_end];
        let name =
            head.find("Name=").and_then(|_| attr(rest, "<Data", "Name")).unwrap_or_else(|| {
                positional += 1;
                positional.to_string()
            });

        let body = &rest[open_end + 1..];
        let Some(close) = body.find("</Data>") else { break };
        let value = unescape(body[..close].trim());
        if !value.is_empty() {
            out.push((name, value));
        }
        rest = &body[close..];
    }
    out
}

/// Fields worth showing first. A record carries a dozen, most of them
/// identifiers; these are the ones that say *which* network and *why*.
/// Alphabetical order put `BSSType` and `ConnectionId` in the line and pushed
/// the SSID and the reason out of it.
const PREFERRED: [&str; 5] = ["SSID", "BSSID", "ReasonCode", "Reason", "ProfileName"];

/// One line of `k=v`, deduplicated and kept short enough to sit in a table
/// cell. The full record is in Event Viewer; this is a fingerprint.
fn flatten(data: &[(String, String)]) -> String {
    const SKIP: [&str; 4] = ["ProcessId", "ThreadId", "ActivityId", "InterfaceGuid"];
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for (k, v) in data {
        if SKIP.contains(&k.as_str()) || v.len() > 64 {
            continue;
        }
        seen.entry(k.as_str()).or_insert(v.as_str());
    }

    let rank = |k: &str| PREFERRED.iter().position(|p| p.eq_ignore_ascii_case(k)).unwrap_or(9);
    let mut rows: Vec<(&str, &str)> = seen.into_iter().collect();
    rows.sort_by_key(|(k, _)| rank(k));
    rows.iter().take(4).map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("  ")
}

/// Reason codes are logged as decimal by some providers and as `0x…` by
/// others, sometimes with the interesting half in the low word.
fn parse_code(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    v.parse().ok()
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

// ---------------------------------------------------------------------------
// time, in the one format the log speaks
// ---------------------------------------------------------------------------

fn iso_utc(ts: f64) -> String {
    let t = ts as i64;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (y, m, d) = crate::diagnose::civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// `2026-09-19T08:13:22.1234567Z` back to a unix timestamp. The log always
/// writes UTC with this layout, so the parse is positional and any deviation
/// is treated as unreadable rather than guessed at.
fn parse_iso_utc(s: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let y = num(0, 4)?;
    let m = num(5, 7)?;
    let d = num(8, 10)?;
    let h = num(11, 13)?;
    let mi = num(14, 16)?;
    let sec = num(17, 19)?;
    let frac = s
        .get(20..)
        .and_then(|f| f.trim_end_matches('Z').get(..3))
        .and_then(|f| f.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 1000.0;

    Some((days_from_civil(y, m, d) * 86400 + h * 3600 + mi * 60 + sec) as f64 + frac)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const WLAN_8003: &str = r#"<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'>
<System><Provider Name='Microsoft-Windows-WLAN-AutoConfig' Guid='{abc}'/>
<EventID>8003</EventID><Version>0</Version><Level>3</Level>
<TimeCreated SystemTime='2026-09-19T08:13:22.1234567Z'/>
<Execution ProcessID='1' ThreadID='2'/><Channel>Microsoft-Windows-WLAN-AutoConfig/Operational</Channel>
</System><EventData><Data Name='InterfaceGuid'>{d}</Data><Data Name='SSID'>home&amp;away</Data>
<Data Name='BSSID'>aa:bb:cc:dd:ee:ff</Data><Data Name='BSSType'>Infrastructure</Data>
<Data Name='Reason'>The network is disconnected by the driver.</Data>
<Data Name='ConnectionId'>0x1</Data><Data Name='ReasonCode'>4</Data></EventData></Event>"#;

    fn parse(xml: &str) -> SysEvent {
        parse_event(xml.split("<Event ").nth(1).unwrap()).expect("a well-formed record parses")
    }

    /// The one test that actually talks to `wevtutil`.
    ///
    /// Everything else here feeds fixtures to the parser, which is why a
    /// misspelled tool name went unnoticed for so long: the name is a string
    /// nothing verified.
    ///
    /// It asserts on the raw XML rather than on `read_channel`'s output,
    /// because `parse_event` keeps only the faults `classify` knows. A healthy
    /// machine yields zero of those, so a count of parsed events would measure
    /// the week's luck instead of the plumbing. The window is a week for the
    /// same reason: the System channel of an idle machine can be silent for an
    /// hour, and was on the machine this was written on.
    #[cfg(windows)]
    #[test]
    fn wevtutil_answers_the_query_read_channel_sends() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is past 1970")
            .as_secs_f64();
        let query = format!(
            "*[System[TimeCreated[@SystemTime>='{}' and @SystemTime<='{}']]]",
            iso_utc(now - 7.0 * 86400.0),
            iso_utc(now)
        );

        let owned = channel_args("System", &query);
        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        let xml = crate::optimize::run("wevtutil", &args).expect("wevtutil runs and exits zero");
        assert!(xml.contains("<Event "), "the System channel returned no records for a week");

        // The full path has to survive the same call without panicking.
        let _ = read_channel("System", &query);
    }

    #[test]
    fn a_wlan_disconnect_yields_its_reason_code() {
        let e = parse(WLAN_8003);
        assert_eq!(e.kind, Kind::WlanDisconnect);
        assert_eq!(e.id, 8003);
        assert_eq!(e.reason, Some(4), "reason 4 is the inactivity deauth");
        assert_eq!(e.provider, "Microsoft-Windows-WLAN-AutoConfig");
    }

    #[test]
    fn entities_are_decoded_and_noise_fields_dropped() {
        let e = parse(WLAN_8003);
        assert!(e.detail.contains("SSID=home&away"), "got {}", e.detail);
        assert!(
            !e.detail.contains("InterfaceGuid"),
            "a GUID is an identifier, not evidence a person can read"
        );
    }

    #[test]
    fn timestamps_round_trip_through_the_logs_own_format() {
        let e = parse(WLAN_8003);
        assert_eq!(iso_utc(e.ts), "2026-09-19T08:13:22.000Z");
        // 2026-09-19T08:13:22Z, checked against an independent conversion.
        assert!((e.ts - 1_789_805_602.123).abs() < 0.01, "got {}", e.ts);
    }

    #[test]
    fn a_localised_message_body_is_never_needed() {
        // The same record as a Polish machine renders it: the <Message> is in
        // another language and every field this module reads is untouched.
        let pl = WLAN_8003.replace(
            "</EventData>",
            "</EventData><RenderingInfo><Message>Sieć bezprzewodowa rozłączona</Message></RenderingInfo>",
        );
        let e = parse(&pl);
        assert_eq!(e.kind, Kind::WlanDisconnect);
        assert_eq!(e.reason, Some(4));
    }

    #[test]
    fn an_unremarkable_information_event_is_not_evidence() {
        let xml = WLAN_8003
            .replace("Microsoft-Windows-WLAN-AutoConfig", "Microsoft-Windows-Winlogon")
            .replace("<EventID>8003</EventID>", "<EventID>7001</EventID>")
            .replace("<Level>3</Level>", "<Level>4</Level>");
        assert!(
            parse_event(xml.split("<Event ").nth(1).unwrap()).is_none(),
            "an unrelated provider at information level says nothing about the network"
        );
    }

    #[test]
    fn a_driver_error_gets_in_on_its_level() {
        let xml = WLAN_8003
            .replace("Microsoft-Windows-WLAN-AutoConfig", "Netwtw10")
            .replace("<EventID>8003</EventID>", "<EventID>5002</EventID>")
            .replace("<Level>3</Level>", "<Level>2</Level>");
        let e = parse(&xml);
        assert_eq!(e.kind, Kind::DriverFault);

        // The same provider at information level is routine chatter.
        let info = xml.replace("<Level>2</Level>", "<Level>4</Level>");
        assert!(parse_event(info.split("<Event ").nth(1).unwrap()).is_none());
    }

    #[test]
    fn the_sentence_named_reason_does_not_shadow_the_numeric_one() {
        // A real 8003 carries both, sentence first. Taking the first field
        // whose name starts with "reason" finds prose, fails to parse it, and
        // silently drops the code the whole verdict hangs on.
        let e = parse(WLAN_8003);
        assert_eq!(e.reason, Some(4));
    }

    #[test]
    fn a_reason_code_of_zero_is_no_reason_at_all() {
        let none = WLAN_8003
            .replace("<Data Name='ReasonCode'>4</Data>", "<Data Name='ReasonCode'>0</Data>");
        assert_eq!(
            parse(&none).reason,
            None,
            "zero means the provider logged no reason, not reason number zero"
        );
    }

    #[test]
    fn the_identifying_fields_survive_the_four_field_budget() {
        // Alphabetical order put BSSType and ConnectionId in the line and
        // pushed out the SSID and the reason, which are the entire point.
        let d = parse(WLAN_8003).detail;
        assert!(d.contains("SSID=") && d.contains("ReasonCode=4"), "got {d}");
        assert!(!d.contains("BSSType"), "got {d}");
    }

    #[test]
    fn the_chatter_of_a_successful_association_is_not_a_failure() {
        // 11004 is logged on this machine during every clean connect, right
        // before the 8001 that says it worked.
        assert_eq!(classify("Microsoft-Windows-WLAN-AutoConfig", 11004, 4), None);
        assert_eq!(
            classify("Microsoft-Windows-WLAN-AutoConfig", 8002, 3),
            Some(Kind::WlanAuthFail)
        );
    }

    #[test]
    fn hex_reason_codes_parse_as_well_as_decimal() {
        assert_eq!(parse_code("0x0000000F"), Some(15));
        assert_eq!(parse_code(" 23 "), Some(23));
        assert_eq!(parse_code("nonsense"), None);
    }

    #[test]
    fn sleep_and_resume_are_recognised_apart() {
        assert_eq!(classify("Microsoft-Windows-Kernel-Power", 42, 4), Some(Kind::Sleep));
        // Modern Standby: entering and leaving it.
        assert_eq!(classify("Microsoft-Windows-Kernel-Power", 506, 4), Some(Kind::Sleep));
        assert_eq!(classify("Microsoft-Windows-Kernel-Power", 507, 4), Some(Kind::Resume));
        assert_eq!(classify("Microsoft-Windows-Power-Troubleshooter", 1, 4), Some(Kind::Resume));
        assert_eq!(classify("Microsoft-Windows-NetworkProfile", 10001, 4), Some(Kind::LinkDown));
    }

    #[test]
    fn a_dhcp_success_is_not_a_dhcp_failure() {
        assert_eq!(classify("Microsoft-Windows-Dhcp-Client", 1003, 3), Some(Kind::DhcpFail));
        assert_eq!(
            classify("Microsoft-Windows-Dhcp-Client", 60000, 4),
            None,
            "the DHCP client logs routine renewals too"
        );
    }

    #[test]
    fn only_the_dhcp_ids_the_manifest_calls_failures_are_failures() {
        // Read off the provider manifests (`Get-WinEvent -ListProvider`) on
        // Windows 11 26200. 1046 and 1047 are the fallback configuration
        // being read, 50066 and 50067 an address attached for an SSID: all
        // informational, and 50066/50067 come with ordinary Wi-Fi
        // connections. They were each a certain "DHCP lease failed".
        const V4: &str = "Microsoft-Windows-Dhcp-Client";
        for routine in [1046, 1047, 50066, 50067, 50036, 50103] {
            assert_eq!(classify(V4, routine, 4), None, "{routine} is informational");
        }
        for failed in [1001, 1002, 1003, 1006] {
            assert_eq!(classify(V4, failed, 2), Some(Kind::DhcpFail), "{failed}");
        }
        // 1005 is the address already in use on the network.
        assert_eq!(classify(V4, 1005, 3), Some(Kind::DuplicateIp));

        // DHCPv6 logs its service starting and stopping on every network,
        // IPv6 or not (51046, 51047, 51057 on this machine, no IPv6 at all),
        // and a network without IPv6 has no DHCPv6 lease to lose. It never
        // explains an IPv4 outage.
        const V6: &str = "Microsoft-Windows-DHCPv6-Client";
        for id in [1000, 1003, 51046, 51047, 51057, 51062] {
            assert_eq!(classify(V6, id, 2), None, "DHCPv6 {id}");
        }
    }
}
