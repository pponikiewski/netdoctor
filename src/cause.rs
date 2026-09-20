//! From a logged outage to a named cause and a fix worth applying.
//!
//! The monitor already decides *where* a connection broke — router, adapter,
//! ISP, DNS. That is a location, not a reason, and a location on its own is
//! not actionable: "the router stopped answering" is equally true of a laptop
//! carried out of range, a Wi-Fi card put to sleep by the power plan, a roam
//! to a weaker access point, and a router that genuinely crashed. Each of
//! those has a different fix.
//!
//! The evidence for telling them apart was already being collected and stored
//! on every outage — the connection state, plus the three minutes of sweeps
//! leading up to it — and then read by nothing. This module reads it.
//!
//! The rules are deliberately plain and explain themselves through the
//! evidence line they emit, so a verdict can be argued with. Where a cause has
//! a matching entry in the Optimise tab, it carries that tweak's id and the UI
//! can take the user straight to it.

use crate::i18n;
use crate::monitor::LeadSample;
use crate::probe::eventlog::{Kind, SysEvent};
use crate::store::{Event, EventContext, TweakLogRow};

/// How much the evidence supports the cause. This is about the strength of the
/// reading, not the severity of the outage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Possible,
    Likely,
    Certain,
}

impl Confidence {
    pub fn label(&self) -> &'static str {
        match self {
            Confidence::Certain => i18n::conf_certain(),
            Confidence::Likely => i18n::conf_likely(),
            Confidence::Possible => i18n::conf_possible(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cause {
    /// Stable code, translated for display. Never shown raw.
    pub code: &'static str,
    pub confidence: Confidence,
    /// The numbers this verdict was read off, already localised.
    pub evidence: String,
    /// Id of the tweak in the Optimise tab that addresses this, when one does.
    pub fix_tweak: Option<&'static str>,
}

impl Cause {
    fn new(code: &'static str, confidence: Confidence, evidence: String) -> Self {
        Cause { code, confidence, evidence, fix_tweak: None }
    }

    fn with_fix(mut self, tweak: &'static str) -> Self {
        self.fix_tweak = Some(tweak);
        self
    }

    pub fn title(&self) -> String {
        i18n::cause_title(self.code)
    }

    pub fn advice(&self) -> String {
        i18n::cause_advice(self.code)
    }
}

/// An outage's stored evidence, parsed into something the rules can read.
pub struct Evidence {
    pub lead: Vec<LeadSample>,
    pub roamed: bool,
    pub medium: String,
    pub rssi_dbm: Option<i32>,
    pub channel: Option<u32>,
    pub dns_is_router_only: bool,
    pub dns_error: String,
    pub up: bool,
    pub rssi_at_recovery: Option<i32>,
}

impl Evidence {
    /// Parses one outage's stored context. The caller does this once and keeps
    /// the result: it is 33 KB of JSON, and the History tab used to parse it
    /// twice per frame.
    pub fn from_context(ctx: &EventContext) -> Option<Evidence> {
        let v = ctx.context_json()?;
        let lead: Vec<LeadSample> = v
            .get("lead_up")
            .and_then(|l| serde_json::from_value(l.clone()).ok())
            .unwrap_or_default();
        Some(Evidence {
            lead,
            roamed: v["roamed"].as_bool().unwrap_or(false),
            medium: v["medium"].as_str().unwrap_or_default().to_string(),
            rssi_dbm: v["rssi_dbm"].as_i64().map(|n| n as i32),
            channel: v["channel"].as_u64().map(|n| n as u32),
            dns_is_router_only: v["dns_is_router_only"].as_bool().unwrap_or(false),
            dns_error: v["dns_error"].as_str().unwrap_or_default().to_string(),
            up: v["up"].as_bool().unwrap_or(true),
            rssi_at_recovery: ctx
                .context_end_json()
                .and_then(|e| e["rssi_dbm"].as_i64())
                .map(|n| n as i32),
        })
    }

    /// RSSI at the start of the recorded lead-up and at its end. Needs a few
    /// readings to mean anything; two samples are noise, not a trend.
    fn rssi_trend(&self) -> Option<(i32, i32)> {
        let vals: Vec<i32> = self.lead.iter().filter_map(|s| s.rssi_dbm).collect();
        if vals.len() < 5 {
            return None;
        }
        // Average the ends rather than trusting single readings: RSSI is noisy
        // enough that one outlier would invent or hide a trend.
        let window = (vals.len() / 4).max(2);
        let mean = |slice: &[i32]| -> i32 {
            (slice.iter().map(|v| *v as f64).sum::<f64>() / slice.len() as f64).round() as i32
        };
        Some((mean(&vals[..window]), mean(&vals[vals.len() - window..])))
    }

    /// Did the access point change during the lead-up?
    fn bssid_change(&self) -> Option<(String, String)> {
        let seen: Vec<&String> =
            self.lead.iter().map(|s| &s.bssid).filter(|b| !b.is_empty()).collect();
        let first = seen.first()?;
        let last = seen.last()?;
        (first != last).then(|| ((*first).clone(), (*last).clone()))
    }

    /// Was the adapter reporting a live link right before it dropped? An
    /// adapter that goes from up to down while the radio was still healthy did
    /// not walk out of range — something switched it off.
    fn dropped_while_up(&self) -> bool {
        let down_at = self.lead.iter().position(|s| !s.up);
        match down_at {
            Some(i) if i > 0 => self.lead[..i].iter().rev().take(10).all(|s| s.up),
            _ => false,
        }
    }

    /// Round-trip to the router across the lead-up, for spotting a link that
    /// was already struggling before it failed outright.
    fn gateway_trend(&self) -> Option<(f64, f64)> {
        let vals: Vec<f64> = self.lead.iter().filter_map(|s| s.gateway_ms).collect();
        if vals.len() < 6 {
            return None;
        }
        let window = (vals.len() / 4).max(2);
        let mean = |slice: &[f64]| slice.iter().sum::<f64>() / slice.len() as f64;
        Some((mean(&vals[..window]), mean(&vals[vals.len() - window..])))
    }

    /// Wi-Fi receive rate at the start and end of the lead-up.
    fn rate_trend(&self) -> Option<(u32, u32)> {
        let vals: Vec<u32> = self.lead.iter().filter_map(|s| s.rx_mbps).collect();
        if vals.len() < 5 {
            return None;
        }
        Some((*vals.first()?, *vals.last()?))
    }

    fn is_wifi(&self) -> bool {
        !self.medium.is_empty() && self.lead.iter().any(|s| s.rssi_dbm.is_some())
            || self.rssi_dbm.is_some()
    }
}

/// How long before an outage a configuration change is still a suspect.
const TWEAK_SUSPECT_WINDOW_S: f64 = 20.0 * 60.0;

/// Reads an outage. `evidence` is the row's parsed context, `None` when the
/// row has none or it did not parse; `history` is other outages for
/// recurrence, `tweaks` are changes applied shortly before this one, and `log`
/// is what Windows itself wrote down around it — see
/// [`crate::probe::eventlog`].
///
/// The evidence is a parameter rather than something read from `event`
/// because [`Event`] no longer carries the context: the caller fetches it for
/// the one row on screen and parses it once. That also makes it impossible to
/// analyse a row whose context was never loaded and get a confident "no
/// evidence" back.
pub fn analyse(
    event: &Event,
    evidence: Option<&Evidence>,
    history: &[Event],
    tweaks: &[TweakLogRow],
    log: &[SysEvent],
) -> Vec<Cause> {
    let mut out = Vec::new();

    // A change applied minutes earlier outranks every signal reading: if the
    // user broke it themselves, nothing else is worth saying first.
    if let Some(t) = tweaks
        .iter()
        .filter(|t| t.ts < event.ts_start && event.ts_start - t.ts <= TWEAK_SUSPECT_WINDOW_S)
        .max_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal))
    {
        let mins = ((event.ts_start - t.ts) / 60.0).round() as i64;
        let mut cause = Cause::new(
            "after_tweak",
            Confidence::Likely,
            i18n::ev_after_tweak(&i18n::tweak_name(&t.tweak_id), mins),
        );
        cause.fix_tweak = known_tweak_id(&t.tweak_id);
        out.push(cause);
    }

    // What the OS recorded outranks what the probes inferred, so it is read
    // before any of the signal rules — and before the evidence check below,
    // because a row too old to carry a context can still be explained by the
    // log, which was written by someone else and is still there.
    log_rules(event, log, &mut out);

    let Some(ev) = evidence else {
        if out.is_empty() {
            // Rows written before the app read its own context back, or a
            // failed serialisation. Saying so is more useful than guessing.
            out.push(Cause::new("no_evidence", Confidence::Possible, i18n::ev_none()));
        }
        return out;
    };

    match event.scope.as_str() {
        "adapter" => adapter_rules(ev, &mut out),
        "lan" => lan_rules(ev, &mut out),
        "isp" => isp_rules(event, history, &mut out),
        "dns" => dns_rules(ev, &mut out),
        _ => degraded_rules(ev, &mut out),
    }

    recurrence(event, history, &mut out);

    if out.is_empty() {
        // "No single cause stands out" is only honest when there was something
        // to read. A row from before the lead-up was stored has a context but
        // no history in it, and most rules need the history — saying the
        // evidence is inconclusive would blame the outage for a gap in the
        // recording.
        let code = if ev.lead.is_empty() { "no_evidence" } else { "unclear" };
        let evidence = if ev.lead.is_empty() { i18n::ev_none() } else { i18n::ev_unclear() };
        out.push(Cause::new(code, Confidence::Possible, evidence));
    }

    // Strongest reading first; ties keep the order the rules produced, which
    // runs from most specific to most general.
    out.sort_by_key(|c| std::cmp::Reverse(c.confidence));
    out
}

/// How far before an outage a suspend still explains it. A machine that went
/// to sleep does not have a network problem, and three minutes is long enough
/// to cover the gap between the last sweep and the log line.
const SLEEP_WINDOW_S: f64 = 180.0;

/// How long before an outage, and how long after it ended, a log line is
/// still about it.
///
/// The caller fetches a narrow slice of the log, and every rule here used to
/// rely on that entirely. An outage that was never closed — the app was
/// killed or the machine lost power while it was running — has no end, so
/// that slice runs to the present and grows by a day every day. Any driver
/// fault or DHCP failure logged since would then be read as `Certain`
/// evidence for an outage it has nothing to do with. The rules carry their
/// own bound now, so the reading cannot be wider than the claim.
const LOG_LEAD_S: f64 = 180.0;
const LOG_TRAIL_S: f64 = 120.0;

/// Verdicts read straight out of the Windows event log.
///
/// Everything else in this module argues from a latency series. These do not
/// argue: the operating system recorded the event, named it, and in the
/// wireless case wrote down the 802.11 reason code for it. That is why they
/// come back `Certain` where the mapping is unambiguous, and why they are
/// produced before the inference rules rather than alongside them.
fn log_rules(event: &Event, log: &[SysEvent], out: &mut Vec<Cause>) {
    let t0 = event.ts_start;
    let at = |e: &SysEvent| i18n::clock_offset(e.offset_from(t0));
    // Counted from here, not from `out.len() == 0`. A cause pushed before
    // this function ran — `after_tweak` always is — used to suppress the
    // link-down rule below, so a tweak applied twenty minutes earlier could
    // hide the log line saying the interface went down, and a registry tweak
    // that cannot unplug a cable became the headline explanation for one.
    let before = out.len();
    // An open outage has no end to measure from, so the trailing edge is
    // pinned to its start rather than to "now".
    let t_end = event.ts_end.unwrap_or(t0);
    let near = |e: &&SysEvent| e.ts >= t0 - LOG_LEAD_S && e.ts <= t_end + LOG_TRAIL_S;

    if let Some(e) = log
        .iter()
        .find(|e| e.kind == Kind::Sleep && (-SLEEP_WINDOW_S..=5.0).contains(&e.offset_from(t0)))
    {
        out.push(Cause::new("log_sleep", Confidence::Certain, i18n::ev_log_sleep(&at(e))));
    } else if let Some(e) =
        log.iter().find(|e| e.kind == Kind::Resume && (-30.0..=90.0).contains(&e.offset_from(t0)))
    {
        // Waking is not sleeping: the radio has to re-associate and the DHCP
        // lease has to be confirmed, and an outage that fills exactly that gap
        // is the resume sequence, not a fault in it.
        out.push(Cause::new("log_resume", Confidence::Likely, i18n::ev_log_resume(&at(e))));
    }

    if let Some(e) = log.iter().filter(near).find(|e| e.kind == Kind::DriverFault) {
        out.push(
            Cause::new(
                "log_driver_fault",
                Confidence::Certain,
                i18n::ev_log_driver(&e.provider, e.id, &at(e)),
            )
            .with_fix("stack_reset"),
        );
    }

    if let Some(e) = log.iter().filter(near).find(|e| e.kind == Kind::WlanDisconnect) {
        out.push(wlan_disconnect_cause(e, &at(e)));
    } else if let Some(e) = log.iter().filter(near).find(|e| e.kind == Kind::WlanAuthFail) {
        out.push(Cause::new(
            "log_wlan_auth",
            Confidence::Certain,
            i18n::ev_log_wlan_auth(e.id, &at(e)),
        ));
    }

    if let Some(e) = log.iter().filter(near).find(|e| e.kind == Kind::DhcpFail) {
        out.push(Cause::new("log_dhcp", Confidence::Certain, i18n::ev_log_dhcp(&at(e))));
    }

    if let Some(e) = log.iter().filter(near).find(|e| e.kind == Kind::DuplicateIp) {
        out.push(Cause::new(
            "log_duplicate_ip",
            Confidence::Certain,
            i18n::ev_log_duplicate_ip(&at(e)),
        ));
    }

    // The interface going down is only news when nothing more specific in the
    // log already said why it did.
    if out.len() == before {
        if let Some(e) = log.iter().filter(near).find(|e| e.kind == Kind::LinkDown) {
            out.push(Cause::new(
                "log_link_down",
                Confidence::Likely,
                i18n::ev_log_link_down(&at(e)),
            ));
        }
    }

    // A quiet log during a WAN outage is evidence too, and it is the evidence
    // a provider argues against hardest: nothing on this machine went wrong.
    // It is only worth saying when the log had something to say at all —
    // "empty" and "unavailable" look identical from here.
    if event.scope == "isp" && !log.is_empty() && !log.iter().any(|e| e.kind.is_fault()) {
        out.push(Cause::new("log_clean_isp", Confidence::Likely, i18n::ev_log_clean(log.len())));
    }
}

/// 802.11 reason codes worth naming. The rest are shown as their number: an
/// honest "reason 71" beats a confident wrong sentence, and the number is what
/// a support line will ask for anyway.
fn wlan_disconnect_cause(e: &SysEvent, at: &str) -> Cause {
    match e.reason {
        // Disassociated due to inactivity. The access point stopped hearing
        // from a card that Windows had quietly powered down, which is the
        // single most common cause of "it drops when I leave it alone".
        Some(4) => {
            Cause::new("log_wlan_inactivity", Confidence::Certain, i18n::ev_log_wlan_reason(4, at))
                .with_fix("adapter_power")
        }
        // Handshake and key failures: the credentials or the key rotation,
        // not the radio.
        Some(r @ (2 | 15 | 23)) => {
            Cause::new("log_wlan_auth", Confidence::Certain, i18n::ev_log_wlan_reason(r, at))
        }
        // The access point turned us away rather than losing us.
        Some(r @ 5..=7) => {
            Cause::new("log_wlan_ap_rejected", Confidence::Certain, i18n::ev_log_wlan_reason(r, at))
        }
        Some(r) => {
            Cause::new("log_wlan_deauth", Confidence::Likely, i18n::ev_log_wlan_reason(r, at))
        }
        None => Cause::new("log_wlan_deauth", Confidence::Likely, i18n::ev_log_wlan_plain(at)),
    }
}

fn adapter_rules(ev: &Evidence, out: &mut Vec<Cause>) {
    let healthy_radio = ev.rssi_dbm.is_none_or(|r| r > -70);

    if ev.dropped_while_up() && healthy_radio {
        out.push(
            Cause::new(
                "adapter_powered_down",
                Confidence::Likely,
                i18n::ev_adapter_powered_down(ev.rssi_dbm),
            )
            .with_fix("adapter_power"),
        );
        // The power plan is the other half of the same story: Windows can park
        // the radio through either setting, and one does not imply the other.
        out.push(
            Cause::new("adapter_power_plan", Confidence::Possible, i18n::ev_adapter_power_plan())
                .with_fix("wlan_power_plan"),
        );
    }

    if let Some(r) = ev.rssi_dbm.filter(|r| *r <= -75) {
        out.push(Cause::new("out_of_range", Confidence::Likely, i18n::ev_rssi_low(r)));
    }

    if !ev.dropped_while_up() && ev.rssi_dbm.is_none() && !ev.up {
        out.push(
            Cause::new("adapter_or_driver", Confidence::Possible, i18n::ev_adapter_absent())
                .with_fix("stack_reset"),
        );
    }
}

fn lan_rules(ev: &Evidence, out: &mut Vec<Cause>) {
    if let Some((from, to)) = ev.bssid_change() {
        out.push(Cause::new("roaming", Confidence::Likely, i18n::ev_roam(&from, &to)));
    } else if ev.roamed {
        out.push(Cause::new("roaming", Confidence::Possible, i18n::ev_roam_flag()));
    }

    match ev.rssi_trend() {
        // A signal sliding away over the lead-up is the classic "walked into
        // the next room" outage, and it is unmistakable once plotted.
        Some((from, to)) if from - to >= 10 => {
            out.push(Cause::new("signal_fade", Confidence::Certain, i18n::ev_rssi_fade(from, to)));
        }
        // Steady and strong right up to the drop: the radio was fine, so the
        // router or the airtime around it was not.
        Some((_, to)) if to >= -60 => {
            let cause = if ev.channel.is_some_and(|c| (1..=14).contains(&c)) {
                Cause::new("airtime_24ghz", Confidence::Likely, i18n::ev_crowded_24(ev.channel))
            } else {
                Cause::new("router_side", Confidence::Likely, i18n::ev_signal_was_fine(to))
            };
            out.push(cause);
        }
        _ => {}
    }

    if let Some(r) = ev.rssi_dbm.filter(|r| *r <= -72) {
        if !out.iter().any(|c| c.code == "signal_fade") {
            out.push(Cause::new("weak_signal", Confidence::Likely, i18n::ev_rssi_low(r)));
        }
    }

    // Recovering into a noticeably better signal than it failed on means the
    // link was marginal, whatever the absolute numbers looked like.
    if let (Some(start), Some(end)) = (ev.rssi_dbm, ev.rssi_at_recovery) {
        if end - start >= 12 {
            out.push(Cause::new(
                "marginal_link",
                Confidence::Likely,
                i18n::ev_recovered_stronger(start, end),
            ));
        }
    }

    if !ev.is_wifi() {
        out.push(Cause::new("cable_or_router", Confidence::Likely, i18n::ev_wired()));
    }
}

fn isp_rules(event: &Event, history: &[Event], out: &mut Vec<Cause>) {
    let long = event.duration_s().is_some_and(|d| d > 120.0);
    let code = if long { "isp_sustained" } else { "isp_brief" };
    let conf = if long { Confidence::Likely } else { Confidence::Possible };
    out.push(Cause::new(code, conf, i18n::ev_isp(event.duration_s())));

    // Repeated WAN drops are the evidence an ISP will actually engage with,
    // and a single one is not.
    let similar = history.iter().filter(|e| e.scope == "isp").count();
    if similar >= 3 {
        out.push(Cause::new("isp_pattern", Confidence::Likely, i18n::ev_isp_pattern(similar)));
    }
}

fn dns_rules(ev: &Evidence, out: &mut Vec<Cause>) {
    if ev.dns_is_router_only {
        out.push(
            Cause::new("dns_router_only", Confidence::Likely, i18n::ev_dns_router_only())
                .with_fix("fast_dns"),
        );
    } else {
        out.push(
            Cause::new("dns_resolver", Confidence::Likely, i18n::ev_dns_error(&ev.dns_error))
                .with_fix("fast_dns"),
        );
    }
}

fn degraded_rules(ev: &Evidence, out: &mut Vec<Cause>) {
    if let Some((from, to)) = ev.gateway_trend() {
        // Latency to the router climbing while the router is still answering
        // is saturation on this side of it, not a problem further out.
        if to > from * 3.0 && to > 30.0 {
            out.push(Cause::new(
                "local_saturation",
                Confidence::Likely,
                i18n::ev_latency_climb(from, to),
            ));
        }
    }

    if let Some((from, to)) = ev.rate_trend() {
        if from > 0 && to * 2 <= from {
            out.push(Cause::new("rate_collapse", Confidence::Likely, i18n::ev_rate_drop(from, to)));
        }
    }

    if let Some((from, to)) = ev.rssi_trend() {
        if from - to >= 8 {
            out.push(Cause::new("signal_fade", Confidence::Likely, i18n::ev_rssi_fade(from, to)));
        }
    }
}

/// Outages that repeat at the same hour are a schedule, not bad luck — a
/// neighbour's appliance, a router's nightly resync, a backup job.
fn recurrence(event: &Event, history: &[Event], out: &mut Vec<Cause>) {
    let same: Vec<&Event> = history.iter().filter(|e| e.scope == event.scope).collect();
    if same.len() < 3 {
        return;
    }
    let this_hour = crate::diagnose::local_hour(event.ts_start);
    let matching =
        same.iter().filter(|e| crate::diagnose::local_hour(e.ts_start) == this_hour).count();

    if matching >= 3 && matching * 2 >= same.len() {
        out.push(Cause::new(
            "time_pattern",
            Confidence::Likely,
            i18n::ev_time_pattern(matching, this_hour),
        ));
    }
}

/// Maps a logged tweak id back to the `&'static str` the Optimise tab uses, so
/// a suspected change can be opened for reverting. An unknown id — a tweak
/// removed since it was logged — simply has no destination.
fn known_tweak_id(id: &str) -> Option<&'static str> {
    [
        "adapter_power",
        "fast_dns",
        "mtu",
        "nagle_off",
        "net_throttling",
        "stack_reset",
        "tcp_autotuning",
        "wlan_power_plan",
    ]
    .into_iter()
    .find(|known| *known == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lead(points: &[(f64, i32, bool)]) -> serde_json::Value {
        serde_json::Value::Array(
            points
                .iter()
                .map(|(ts, rssi, up)| {
                    serde_json::json!({
                        "ts": ts, "status": "ok", "up": up, "bssid": "aa:bb",
                        "rssi_dbm": rssi, "gateway_ms": 3.0, "internet_ok": true
                    })
                })
                .collect(),
        )
    }

    /// A logged outage together with its stored context.
    ///
    /// Production keeps the two apart — the list query is hot, the context is
    /// 33 KB — but a rule test is exactly the pairing of "this happened" with
    /// "this is what we recorded", so the tests below hold them together and
    /// [`verdicts`] takes them apart again the way the UI does.
    struct Outage {
        row: Event,
        ctx: EventContext,
    }

    fn event(scope: &str, context: serde_json::Value, dur: f64) -> Outage {
        Outage {
            row: Event {
                id: 1,
                ts_start: 1_000.0,
                ts_end: Some(1_000.0 + dur),
                kind: format!("{scope}_down"),
                scope: scope.into(),
                detail: String::new(),
            },
            ctx: EventContext { context: context.to_string(), context_end: None },
        }
    }

    /// Parses the context and runs the rules, which is what the History tab
    /// does with the row it has selected.
    fn verdicts(
        o: &Outage,
        history: &[Event],
        tweaks: &[TweakLogRow],
        log: &[SysEvent],
    ) -> Vec<Cause> {
        let evidence = Evidence::from_context(&o.ctx);
        analyse(&o.row, evidence.as_ref(), history, tweaks, log)
    }

    #[test]
    fn a_fading_signal_is_named_and_not_blamed_on_the_router() {
        let ctx = serde_json::json!({
            "medium": "Wi-Fi", "rssi_dbm": -82, "up": true,
            "lead_up": lead(&[
                (1.0, -55, true), (2.0, -58, true), (3.0, -64, true),
                (4.0, -71, true), (5.0, -78, true), (6.0, -83, true),
            ]),
        });
        let causes = verdicts(&event("lan", ctx, 40.0), &[], &[], &[]);
        assert_eq!(causes[0].code, "signal_fade");
        assert_eq!(causes[0].confidence, Confidence::Certain);
        assert!(!causes.iter().any(|c| c.code == "router_side"));
    }

    fn sysevent(kind: Kind, ts: f64) -> SysEvent {
        SysEvent {
            ts,
            provider: "netwtw".into(),
            id: 5002,
            kind,
            reason: None,
            detail: String::new(),
        }
    }

    #[test]
    fn what_the_log_recorded_survives_an_unrelated_tweak() {
        // Windows wrote down that the interface went down. A registry tweak
        // applied five minutes earlier cannot unplug a cable, and must not
        // take the place of that evidence — which it did, because the
        // link-down rule skipped itself whenever *anything* was already in
        // the list, and `after_tweak` is always added first.
        let ctx = serde_json::json!({ "medium": "Ethernet", "up": false, "lead_up": [] });
        let log = vec![sysevent(Kind::LinkDown, 990.0)];
        let tweak = TweakLogRow {
            ts: 1_000.0 - 300.0,
            tweak_id: "nagle".into(),
            action: "apply".into(),
            result: "ok".into(),
        };

        let causes = verdicts(&event("lan", ctx, 30.0), &[], &[tweak], &log);
        assert!(
            causes.iter().any(|c| c.code == "log_link_down"),
            "got {:?}",
            causes.iter().map(|c| c.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_fault_logged_hours_away_is_not_evidence_for_this_outage() {
        // An outage the app never closed hands us an open-ended slice of the
        // log. A driver fault from three hours later is not what broke it,
        // and `Certain` is the worst possible label to put on that guess.
        let ctx = serde_json::json!({ "medium": "Ethernet", "up": true, "lead_up": [] });
        let far_away = vec![sysevent(Kind::DriverFault, 1_000.0 + 3.0 * 3600.0)];

        let causes = verdicts(&event("lan", ctx.clone(), 30.0), &[], &[], &far_away);
        assert!(!causes.iter().any(|c| c.code == "log_driver_fault"));

        // The same fault inside the outage still counts.
        let nearby = vec![sysevent(Kind::DriverFault, 1_010.0)];
        let causes = verdicts(&event("lan", ctx, 30.0), &[], &[], &nearby);
        assert!(causes.iter().any(|c| c.code == "log_driver_fault"));
    }

    #[test]
    fn a_steady_strong_signal_points_away_from_the_radio() {
        let ctx = serde_json::json!({
            "medium": "Wi-Fi", "rssi_dbm": -48, "channel": 44, "up": true,
            "lead_up": lead(&[
                (1.0, -47, true), (2.0, -48, true), (3.0, -47, true),
                (4.0, -49, true), (5.0, -48, true), (6.0, -48, true),
            ]),
        });
        let causes = verdicts(&event("lan", ctx, 30.0), &[], &[], &[]);
        assert!(causes.iter().any(|c| c.code == "router_side"));
        assert!(!causes.iter().any(|c| c.code == "signal_fade"));
    }

    #[test]
    fn a_crowded_24ghz_channel_is_separated_from_a_router_fault() {
        let ctx = serde_json::json!({
            "medium": "Wi-Fi", "rssi_dbm": -50, "channel": 6, "up": true,
            "lead_up": lead(&[
                (1.0, -50, true), (2.0, -51, true), (3.0, -50, true),
                (4.0, -50, true), (5.0, -52, true), (6.0, -50, true),
            ]),
        });
        let causes = verdicts(&event("lan", ctx, 30.0), &[], &[], &[]);
        assert!(causes.iter().any(|c| c.code == "airtime_24ghz"));
    }

    #[test]
    fn an_adapter_that_drops_with_a_healthy_radio_suggests_power_saving() {
        let ctx = serde_json::json!({
            "medium": "Wi-Fi", "rssi_dbm": -52, "up": false,
            "lead_up": lead(&[
                (1.0, -52, true), (2.0, -51, true), (3.0, -52, true),
                (4.0, -53, true), (5.0, -52, false), (6.0, -52, false),
            ]),
        });
        let causes = verdicts(&event("adapter", ctx, 90.0), &[], &[], &[]);
        let power = causes.iter().find(|c| c.code == "adapter_powered_down").unwrap();
        assert_eq!(power.fix_tweak, Some("adapter_power"));
    }

    #[test]
    fn a_recent_tweak_outranks_the_signal_reading() {
        let ctx = serde_json::json!({
            "medium": "Wi-Fi", "rssi_dbm": -82, "up": true,
            "lead_up": lead(&[
                (1.0, -55, true), (2.0, -60, true), (3.0, -66, true),
                (4.0, -72, true), (5.0, -79, true), (6.0, -84, true),
            ]),
        });
        let tweaks = vec![TweakLogRow {
            ts: 1_000.0 - 300.0,
            tweak_id: "mtu".into(),
            action: "apply".into(),
            result: "ok".into(),
        }];
        let causes = verdicts(&event("lan", ctx.clone(), 40.0), &[], &tweaks, &[]);
        let tweak = causes.iter().find(|c| c.code == "after_tweak").unwrap();
        assert_eq!(tweak.fix_tweak, Some("mtu"));
        // A change from an hour earlier is no longer a suspect.
        let stale = vec![TweakLogRow { ts: 1_000.0 - 3_600.0, ..tweaks[0].clone() }];
        assert!(!verdicts(&event("lan", ctx.clone(), 40.0), &[], &stale, &[])
            .iter()
            .any(|c| c.code == "after_tweak"));
    }

    #[test]
    fn dns_served_only_by_the_router_gets_the_resolver_fix() {
        let ctx = serde_json::json!({ "dns_is_router_only": true, "dns_error": "timeout" });
        let causes = verdicts(&event("dns", ctx, 15.0), &[], &[], &[]);
        assert_eq!(causes[0].code, "dns_router_only");
        assert_eq!(causes[0].fix_tweak, Some("fast_dns"));
    }

    #[test]
    fn an_old_row_without_context_says_so_instead_of_guessing() {
        let mut e = event("lan", serde_json::json!({}), 20.0);
        e.ctx.context = String::new();
        let causes = verdicts(&e, &[], &[], &[]);
        assert_eq!(causes.len(), 1);
        assert_eq!(causes[0].code, "no_evidence");
    }

    #[test]
    fn a_row_with_state_but_no_lead_up_admits_the_gap() {
        // Exactly the shape of an entry written by the previous version: a
        // context, but nothing about what preceded the outage. Claiming the
        // evidence is inconclusive would misplace the blame.
        let ctx = serde_json::json!({
            "adapter": "WiFi", "ssid": "home", "bssid": "c8:7f:54:b0:15:44",
            "channel": 108, "rssi_dbm": -61, "signal_pct": 78,
        });
        let causes = verdicts(&event("internet", ctx, 94.0), &[], &[], &[]);
        assert_eq!(causes.len(), 1);
        assert_eq!(causes[0].code, "no_evidence");
    }

    #[test]
    fn repeated_wan_drops_become_a_pattern() {
        let outages: Vec<Outage> = (0..4)
            .map(|i| {
                let mut e = event("isp", serde_json::json!({}), 200.0);
                e.row.ts_start = 1_000.0 + (i as f64) * 86_400.0;
                e
            })
            .collect();
        let hist: Vec<Event> = outages.iter().map(|o| o.row.clone()).collect();
        let causes = verdicts(&outages[0], &hist, &[], &[]);
        assert!(causes.iter().any(|c| c.code == "isp_pattern"));
        assert!(causes.iter().any(|c| c.code == "time_pattern"));
    }

    #[test]
    fn noise_in_a_single_reading_does_not_invent_a_trend() {
        // One bad sample in an otherwise flat series must not read as a fade.
        let ctx = serde_json::json!({
            "medium": "Wi-Fi", "rssi_dbm": -50, "channel": 44, "up": true,
            "lead_up": lead(&[
                (1.0, -50, true), (2.0, -88, true), (3.0, -50, true),
                (4.0, -50, true), (5.0, -51, true), (6.0, -50, true),
            ]),
        });
        let causes = verdicts(&event("lan", ctx, 30.0), &[], &[], &[]);
        assert!(!causes.iter().any(|c| c.code == "signal_fade"));
    }

    // -----------------------------------------------------------------------
    // what the OS wrote down
    // -----------------------------------------------------------------------

    /// A log line at `offset` seconds from the start of the test outage.
    fn log(kind: Kind, offset: f64, reason: Option<u32>) -> SysEvent {
        SysEvent {
            ts: 1_000.0 + offset,
            provider: "Microsoft-Windows-WLAN-AutoConfig".into(),
            id: 8003,
            kind,
            reason,
            detail: String::new(),
        }
    }

    fn wifi_ctx() -> serde_json::Value {
        serde_json::json!({
            "medium": "Wi-Fi", "rssi_dbm": -48, "channel": 44, "up": true,
            "lead_up": lead(&[
                (1.0, -47, true), (2.0, -48, true), (3.0, -47, true),
                (4.0, -49, true), (5.0, -48, true), (6.0, -48, true),
            ]),
        })
    }

    #[test]
    fn a_logged_inactivity_deauth_beats_every_inferred_verdict() {
        // Without the log this is the "steady strong signal, blame the router"
        // case. The log says the access point dropped an idle card, which is a
        // different fault with a different fix.
        let entries = [log(Kind::WlanDisconnect, -2.0, Some(4))];
        let causes = verdicts(&event("lan", wifi_ctx(), 30.0), &[], &[], &entries);

        assert_eq!(causes[0].code, "log_wlan_inactivity");
        assert_eq!(causes[0].confidence, Confidence::Certain);
        assert_eq!(causes[0].fix_tweak, Some("adapter_power"));
        assert!(
            causes[0].evidence.contains('4'),
            "the reason code belongs in the evidence: {}",
            causes[0].evidence
        );
    }

    #[test]
    fn an_unnamed_reason_code_is_reported_rather_than_invented() {
        let entries = [log(Kind::WlanDisconnect, -1.0, Some(71))];
        let causes = verdicts(&event("lan", wifi_ctx(), 30.0), &[], &[], &entries);
        let c = causes.iter().find(|c| c.code == "log_wlan_deauth").expect("still a verdict");
        assert_eq!(c.confidence, Confidence::Likely, "an unmapped code is weaker evidence");
        assert!(c.evidence.contains("71"));
    }

    #[test]
    fn a_suspend_explains_the_outage_and_a_wake_does_not_pretend_to() {
        let slept = [log(Kind::Sleep, -12.0, None)];
        let causes = verdicts(&event("lan", wifi_ctx(), 30.0), &[], &[], &slept);
        assert_eq!(causes[0].code, "log_sleep");

        // A sleep from an hour earlier is not this outage's explanation.
        let stale = [log(Kind::Sleep, -3_600.0, None)];
        assert!(!verdicts(&event("lan", wifi_ctx(), 30.0), &[], &[], &stale)
            .iter()
            .any(|c| c.code == "log_sleep"));
    }

    #[test]
    fn a_driver_fault_routes_to_the_stack_reset() {
        let mut e = log(Kind::DriverFault, -3.0, None);
        e.provider = "Netwtw10".into();
        e.id = 5002;
        let causes = verdicts(&event("adapter", wifi_ctx(), 30.0), &[], &[], &[e]);

        let c = causes.iter().find(|c| c.code == "log_driver_fault").unwrap();
        assert_eq!(c.fix_tweak, Some("stack_reset"));
        assert!(c.evidence.contains("Netwtw10") && c.evidence.contains("5002"));
    }

    #[test]
    fn an_interface_drop_only_speaks_when_nothing_more_specific_did() {
        let bare = [log(Kind::LinkDown, 0.0, None)];
        assert!(verdicts(&event("lan", wifi_ctx(), 30.0), &[], &[], &bare)
            .iter()
            .any(|c| c.code == "log_link_down"));

        // Next to a disconnect that names its reason, "the interface went
        // down" is a restatement, not a second finding.
        let both = [log(Kind::WlanDisconnect, -1.0, Some(4)), log(Kind::LinkDown, 0.0, None)];
        assert!(!verdicts(&event("lan", wifi_ctx(), 30.0), &[], &[], &both)
            .iter()
            .any(|c| c.code == "log_link_down"));
    }

    #[test]
    fn a_quiet_log_during_a_wan_outage_is_itself_the_finding() {
        let quiet = [log(Kind::Resume, -900.0, None), log(Kind::WlanConnect, -890.0, None)];
        let causes = verdicts(&event("isp", wifi_ctx(), 300.0), &[], &[], &quiet);
        assert!(causes.iter().any(|c| c.code == "log_clean_isp"));

        // The same silence during a local outage proves nothing, and claiming
        // it would hand the user an argument they cannot win.
        assert!(!verdicts(&event("lan", wifi_ctx(), 300.0), &[], &[], &quiet)
            .iter()
            .any(|c| c.code == "log_clean_isp"));

        // Nor does it hold once the log names a fault here.
        let faulty = [log(Kind::DhcpFail, -5.0, None)];
        assert!(!verdicts(&event("isp", wifi_ctx(), 300.0), &[], &[], &faulty)
            .iter()
            .any(|c| c.code == "log_clean_isp"));
    }

    #[test]
    fn an_entry_with_no_stored_context_can_still_be_explained_by_the_log() {
        // The row predates the app storing a context, so every inference rule
        // is blind. The log was written by someone else and is still there.
        let mut e = event("lan", serde_json::json!(null), 30.0);
        e.ctx.context = String::new();
        let causes = verdicts(&e, &[], &[], &[log(Kind::DhcpFail, -4.0, None)]);

        assert_eq!(causes[0].code, "log_dhcp");
        assert!(
            !causes.iter().any(|c| c.code == "no_evidence"),
            "there was evidence; it just did not come from this app"
        );
    }
}
