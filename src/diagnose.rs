//! One-shot diagnostic scan.
//!
//! Each check produces Findings. Severity drives ordering in the UI, and a
//! finding may point at the tweak that fixes it.
//!
//! A list of findings is not a diagnosis, though. "Jitter is high", "the
//! signal is 60%" and "TCP autotuning is off" can all be true at once while
//! the user still has no idea which of them is *the* reason a call breaks up.
//! So the scan does two things a checklist does not:
//!
//! * It **cuts the chain into segments** — this PC, the router, the
//!   provider's first hop, the open internet — and attributes each millisecond
//!   and each lost packet to the segment that introduced it. Latency measured
//!   only at the far end cannot tell a tired Wi-Fi card from a congested
//!   provider, and those have opposite fixes.
//! * It **reproduces the complaint** instead of only sampling an idle line.
//!   Most "the internet is slow" is bufferbloat, which by definition is
//!   invisible until something saturates the link.
//!
//! Those measurements are then collapsed into a single [`Verdict`]: one
//! segment, how sure we are, what it costs the user in terms they recognise,
//! and at most three actions worth taking.

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bandwidth::{self, BloatResult, Grade};
use crate::cause::Confidence;
use crate::i18n;
use crate::longrun::{self, LongRun, Trouble};
use crate::monitor;
use crate::probe::icmp;
use crate::probe::netstate::{self, LinkCounters, Medium, NetState};
use crate::settings::Settings;
use crate::store::{self, Stats, Store};

/// Where the scan aims everything that has to leave the building.
pub(crate) const ANCHOR: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
/// The monitor's key for `ANCHOR`, which is how the baseline is looked up.
pub(crate) const ANCHOR_KEY: &str = "cloudflare";
/// A second anchor on another operator's network. One address can be
/// filtered by a network in between, and a verdict that the internet is gone
/// should not rest on one address answering.
pub(crate) const ANCHOR_ALT: Ipv4Addr = Ipv4Addr::new(8, 8, 8, 8);
/// The monitor's key for `ANCHOR_ALT`.
pub(crate) const ANCHOR_ALT_KEY: &str = "google";
/// A seven-day window is long enough to average out one bad evening and short
/// enough that a line which genuinely changed does not stay judged by its past.
const BASELINE_WINDOW_S: f64 = 7.0 * 86400.0;
/// Below this, the median is noise rather than a baseline.
const BASELINE_MIN_SAMPLES: usize = 200;

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

    /// A finding with only a key and a severity, for tests outside this
    /// module that need one.
    #[cfg(test)]
    pub fn new_for_test(key: &str, severity: Severity) -> Self {
        Finding::new(key, key, severity, "")
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

// ---------------------------------------------------------------------------
// The verdict
// ---------------------------------------------------------------------------

/// A link in the chain between the user and whatever they were trying to
/// reach. The whole point of the scan is to name exactly one of these, because
/// each has a different owner and a different fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    /// This PC to the router: Wi-Fi, the cable, the adapter's own settings.
    Lan,
    /// The queue on the way out. Owned by the router, felt on the provider's
    /// uplink, which is why it deserves naming separately from either.
    Uplink,
    /// The router to the provider's network.
    Isp,
    /// Past the provider, where nobody local has any influence.
    Internet,
    /// Name resolution, which fails independently of the path working.
    Dns,
    /// Nothing on the wire — a setting on this machine.
    Config,
    /// Nothing found.
    Healthy,
    /// The scan could not send its pings, so there is no chain to cut.
    Unmeasured,
}

impl Segment {
    pub fn label(&self) -> &'static str {
        match self {
            Segment::Lan => i18n::seg_lan(),
            Segment::Uplink => i18n::seg_uplink(),
            Segment::Isp => i18n::seg_isp(),
            Segment::Internet => i18n::seg_internet(),
            Segment::Dns => i18n::seg_dns(),
            Segment::Config => i18n::seg_config(),
            Segment::Healthy => i18n::seg_healthy(),
            Segment::Unmeasured => i18n::seg_unmeasured(),
        }
    }
}

/// One thing worth doing, in the order it is worth doing it.
#[derive(Debug, Clone)]
pub struct Action {
    pub text: String,
    pub tweak_id: Option<String>,
}

/// The answer the user came for.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub segment: Segment,
    pub confidence: Confidence,
    /// Where the round trip is actually spent, when it could be split.
    pub split: Option<String>,
    /// The consequence, in terms the user recognises from using the machine.
    pub cost: String,
    pub actions: Vec<Action>,
}

impl Default for Verdict {
    fn default() -> Self {
        Verdict {
            segment: Segment::Healthy,
            confidence: Confidence::Possible,
            split: None,
            cost: i18n::verdict_none().into(),
            actions: Vec::new(),
        }
    }
}

/// Everything the scan measured, kept separate from how it is worded. The
/// verdict is read off these numbers; the findings only describe them.
#[derive(Debug, Clone, Default)]
pub struct Measurements {
    pub gateway: Option<Stats>,
    /// The first hop past the router, when it is willing to answer.
    pub edge: Option<(Ipv4Addr, Stats)>,
    /// That hop turned out to be the user's own second box rather than the
    /// provider's, so its latency belongs to the LAN share, not the ISP's.
    pub edge_is_local: bool,
    pub internet: Option<Stats>,
    /// This machine's own median RTT and how many samples it rests on.
    pub baseline: Option<(f64, usize)>,
    pub dns_ms: Option<f64>,
    pub tcp_ms: Option<f64>,
    pub tcp_blocked: bool,
    pub load: Option<BloatResult>,
    pub medium: Medium,
    pub signal_pct: Option<u32>,
    /// The router the pings went to, so a row of numbers can say whose they are.
    pub gateway_addr: Option<Ipv4Addr>,
    /// Why no ping series could be sent at all. Every latency above is then
    /// empty for that reason, not because anything stayed silent.
    pub blind: Option<String>,
    /// The long measurement, when the user asked for one and it ran.
    pub long: Option<LongRun>,
}

/// Asking the scan to keep watching for minutes after its quick checks.
#[derive(Clone)]
pub struct LongOpts {
    pub secs: usize,
    /// Set by the UI to stop early; what was recorded until then is kept.
    pub cancel: Arc<AtomicBool>,
}

impl Measurements {
    fn avg(stats: &Option<Stats>) -> Option<f64> {
        stats.as_ref().and_then(|s| s.avg)
    }

    /// Milliseconds contributed by each segment, rather than the cumulative
    /// round trip each probe happens to report. A hop that answers in 40 ms
    /// when the router answers in 38 is not slow; it inherited 38 of them.
    pub fn shares(&self) -> Option<(f64, f64, f64)> {
        let measured = Self::avg(&self.internet)?;
        // Every share is a slice of the one end-to-end measurement, so each
        // boundary is clamped between the one before it and that total. A hop
        // reporting *less* than the hop before it is ordinary — routers
        // deprioritise ICMP addressed to themselves, and jitter moves every
        // reading — but subtracting the raw numbers then invents milliseconds
        // the round trip never contained, and the split is shown to the user
        // as a measurement.
        let lan = Self::avg(&self.gateway)?.clamp(0.0, measured);
        match self.edge.as_ref().and_then(|(_, s)| s.avg) {
            // A hop that is still the user's own equipment extends the local
            // chain instead of starting the provider's; charging its
            // milliseconds to the ISP is how a double-NAT household ends up
            // filing a support ticket about its own spare router.
            Some(edge) => {
                let edge = edge.clamp(lan, measured);
                if self.edge_is_local {
                    Some((edge, 0.0, measured - edge))
                } else {
                    Some((lan, edge - lan, measured - edge))
                }
            }
            // Without the middle hop the remainder cannot be attributed, so it
            // is reported whole rather than guessed at.
            None => Some((lan, 0.0, measured - lan)),
        }
    }
}

/// The result of a scan: the measurements' story, and the answer.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub findings: Vec<Finding>,
    pub verdict: Verdict,
    /// The numbers themselves, for anyone who wants to check the verdict
    /// rather than take it on trust.
    pub measurements: Measurements,
}

// ---------------------------------------------------------------------------
// The chain, as the scan saw it
// ---------------------------------------------------------------------------

/// What the scan can say about one link of the chain. The states are kept
/// apart because they mean different things to the person reading them: a
/// router that ignores pings while traffic flows is not a dead router, and a
/// link that was never probed is not a link that failed.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkState {
    /// Answered; the numbers are the round trip to the far end of this link.
    Measured(Stats),
    /// Nothing came back, and nothing else got through either.
    Silent,
    /// No echo, but traffic demonstrably crossed it: a filter, not a fault.
    Filtered,
    /// This link was not probed, or its far end could not be identified.
    Unknown,
    /// No probe could be sent at all.
    NotMeasured,
}

/// One link: this PC to the router, the router to the provider, the provider
/// to the open internet.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub state: LinkState,
    /// Who answered at the far end, when the scan knows.
    pub addr: Option<Ipv4Addr>,
    /// Milliseconds this link added on top of the one before it, when the
    /// split could be made.
    pub added_ms: Option<f64>,
    /// The far hop is still the user's own equipment (double NAT).
    pub local: bool,
}

/// The three links, read off the measurements and the findings that
/// describe what did not answer. Nothing here is inferred beyond what the
/// findings already concluded.
pub fn chain(findings: &[Finding], m: &Measurements) -> [Link; 3] {
    let has = |key: &str| findings.iter().any(|f| f.key == key);
    let blank = |state| Link { state, addr: None, added_ms: None, local: false };
    if m.blind.is_some() {
        return [
            blank(LinkState::NotMeasured),
            blank(LinkState::NotMeasured),
            blank(LinkState::NotMeasured),
        ];
    }
    let shares = m.shares();

    let lan = match &m.gateway {
        Some(s) => Link {
            state: LinkState::Measured(s.clone()),
            addr: m.gateway_addr,
            added_ms: shares.map(|(lan, _, _)| lan),
            local: false,
        },
        None if has("gateway_mute") => Link { addr: m.gateway_addr, ..blank(LinkState::Filtered) },
        None if has("gateway_silent") => Link { addr: m.gateway_addr, ..blank(LinkState::Silent) },
        None => Link { addr: m.gateway_addr, ..blank(LinkState::Unknown) },
    };

    let isp = match &m.edge {
        Some((addr, s)) => Link {
            state: LinkState::Measured(s.clone()),
            addr: Some(*addr),
            // A local hop's milliseconds were folded into the LAN share, so
            // this link has no share of its own to show.
            added_ms: shares.filter(|_| !m.edge_is_local).map(|(_, isp, _)| isp),
            local: m.edge_is_local,
        },
        // An edge that ignores pings is common and says nothing on its own.
        None => blank(LinkState::Unknown),
    };

    let internet = match &m.internet {
        Some(s) => Link {
            state: LinkState::Measured(s.clone()),
            addr: None,
            added_ms: shares.map(|(_, _, far)| far),
            local: false,
        },
        None if has("icmp_filtered") => blank(LinkState::Filtered),
        None if has("internet_silent") => blank(LinkState::Silent),
        None => blank(LinkState::Unknown),
    };

    [lan, isp, internet]
}

impl Segment {
    /// Which of the three links this segment is, when it is one of them.
    /// The uplink queue sits between the router and the provider.
    pub fn link_index(&self) -> Option<usize> {
        match self {
            Segment::Lan => Some(0),
            Segment::Uplink | Segment::Isp => Some(1),
            Segment::Internet => Some(2),
            _ => None,
        }
    }
}

pub type Progress = Arc<dyn Fn(&str, f32) + Send + Sync>;

/// Run every check. Findings come back worst-first, with a verdict on top.
///
/// `deep` adds the load test. It costs about twenty seconds and briefly
/// saturates the line, which is exactly why it is the check that finds what
/// the others cannot — but it is not something to do behind the user's back.
pub fn scan(
    net: &NetState,
    store: &Store,
    settings: &Settings,
    deep: bool,
    long: Option<LongOpts>,
    progress: Option<Progress>,
) -> Scan {
    // With a long measurement the quick checks are the first fifth of the
    // bar, and the minutes of watching are the rest.
    let outer = progress.clone();
    let quick_share = if long.is_some() { 0.2 } else { 1.0 };
    let progress: Option<Progress> = progress
        .map(|p| Arc::new(move |label: &str, frac: f32| p(label, frac * quick_share)) as Progress);
    let say = |label: &str, frac: f32| {
        if let Some(p) = &progress {
            p(label, frac);
        }
    };

    let mut m = Measurements {
        medium: net.medium.clone(),
        signal_pct: net.signal_pct,
        gateway_addr: net.gateway,
        ..Default::default()
    };
    let mut out = Vec::new();

    say(i18n::step_medium(), 0.02);
    out.extend(check_medium(net, store, settings));

    say(i18n::step_wifi(), 0.06);
    out.extend(check_wifi(net, store, settings));

    say(i18n::step_power(), 0.10);
    out.extend(check_power(net, store, settings));

    // Every probe that touches the wire runs at once. Partly for the twenty
    // seconds it saves, but mainly because the segment split is a subtraction
    // between three measurements: taking them ten seconds apart, across a link
    // whose whole complaint is that it changes, compares numbers that were
    // never true at the same moment.
    say(i18n::step_path(), 0.14);
    // The wire probes are a few hundred frames of known traffic, so the
    // adapter's error counters read around them say whether the cable is
    // corrupting frames right now, not only at some point since boot.
    let counters_before = netstate::link_counters(net.if_index);
    let wire = measure_wire(net, settings);
    let counters_after = netstate::link_counters(net.if_index);
    m.blind = wire.blind.clone();
    out.extend(check_link(net, counters_before, counters_after));

    out.extend(report_dns(net, &wire, &mut m));
    out.extend(report_wire(net, store, settings, &wire, &mut m));

    say(i18n::step_mtu(), 0.50);
    out.extend(check_mtu(net, store, settings));

    say(i18n::step_tcp(), 0.56);
    out.extend(check_tcp(net, store, settings));

    if deep {
        say(i18n::step_load(), 0.60);
        let inner = progress.clone();
        // The load test reports its own 0..1; fold it into the tail of ours.
        let nested: Option<bandwidth::Progress> = inner.map(|p| {
            Arc::new(move |label: &str, frac: f32| p(label, 0.60 + frac * 0.32))
                as bandwidth::Progress
        });
        out.extend(check_load(settings, nested, &mut m));
    } else {
        out.push(Finding::new(
            "load",
            i18n::f_load_skipped(),
            Severity::Info,
            i18n::f_load_skipped_detail(),
        ));
    }

    say(i18n::step_history(), 0.94);
    out.extend(check_history(net, store, settings));

    if let Some(opts) = long {
        out.extend(check_long(opts, settings, outer.clone(), &mut m));
    }

    if let Some(p) = &outer {
        p(i18n::step_done(), 1.0);
    }

    // Worst first, stable within a severity so related findings stay together.
    out.sort_by_key(|f| std::cmp::Reverse(f.severity));
    let verdict = judge(&out, &m, settings);
    Scan { findings: out, verdict, measurements: m }
}

/// Turn the measurements into one segment, one cost and an ordered plan.
///
/// The order below is the order of certainty, not of severity: a segment that
/// went silent is known, a segment that lost packets is nearly known, and a
/// segment that merely contributed the most milliseconds is an inference. The
/// first rule that fires wins, so a hard break is never buried under a
/// millisecond comparison.
pub fn judge(findings: &[Finding], m: &Measurements, cfg: &Settings) -> Verdict {
    let shares = m.shares();
    let split = shares.map(|(lan, isp, far)| i18n::verdict_split(lan, isp, far));

    let has = |key: &str| findings.iter().any(|f| f.key == key);
    // Several keys are emitted whatever the outcome — `medium` describes a
    // healthy Wi-Fi link as readily as a missing one — so a rule that keys off
    // a failure has to ask for the severity as well, not just the subject.
    let failed =
        |key: &str| findings.iter().any(|f| f.key == key && f.severity == Severity::Critical);
    let worst_of = |seg: Segment| actions_for(findings, seg);

    let mut v = Verdict { split: split.clone(), ..Default::default() };

    // 1. Hard breaks. Each of these is an observation, not a judgement.
    // A mute middle hop is deliberately absent: routers that drop their own
    // ICMP are ordinary, and `internet_silent` is what distinguishes one of
    // those from a provider that has actually gone down.
    let broken = [
        ("medium", Segment::Lan),
        ("gateway_silent", Segment::Lan),
        ("internet_silent", Segment::Isp),
        ("tcp_blocked", Segment::Internet),
        ("dns_resolve", Segment::Dns),
    ];
    for (key, seg) in broken {
        if failed(key) {
            v.segment = seg;
            v.confidence = Confidence::Certain;
            v.cost = cost_for(seg, m);
            v.actions = worst_of(seg);
            return v;
        }
    }

    // Everything below reads the pings. Without them the only honest verdict
    // is that there is none; a clean-looking rest would read as "healthy".
    if has("icmp_blind") {
        v.segment = Segment::Unmeasured;
        v.confidence = Confidence::Possible;
        v.cost = i18n::cost_unmeasured().into();
        v.actions = actions_for_key(findings, "icmp_blind");
        return v;
    }

    // 2. Loss, attributed to the first segment that shows it. Loss that is
    //    already present at the router did not come from the internet.
    let loss_seg = loss_origin(m, cfg.loss_ok_pct);
    if let Some((seg, pct)) = loss_seg {
        v.segment = seg;
        v.confidence = Confidence::Likely;
        v.cost = i18n::cost_loss(pct);
        // The `loss` finding is filed under Internet because that is where it
        // was measured, but the origin above may be nearer. Without this the
        // verdict "your LAN is dropping packets" can arrive with no next step
        // at all, because the one finding that has advice was filtered out.
        v.actions = worst_of(seg);
        if v.actions.is_empty() {
            v.actions = actions_for_key(findings, "loss");
        }
        return v;
    }

    // 3. Bufferbloat. An idle line that falls apart the moment it is used is
    //    the commonest cause of "it's slow" and never shows up in a ping.
    if let Some(load) = &m.load {
        if matches!(load.grade_or_unknown(), Grade::D | Grade::F) {
            let bump = load.worst_bump().unwrap_or(0.0);
            v.segment = Segment::Uplink;
            v.confidence = Confidence::Certain;
            v.cost = i18n::cost_load(bump);
            v.actions = worst_of(Segment::Uplink);
            return v;
        }
    }

    // 3b. The long run. Minutes of the same three links on one clock, with
    //     every bad second charged to where it started, is an observation and
    //     outranks the inferences below. It only speaks when it found
    //     something: a quiet five minutes proves the fault was not happening
    //     then, not that it never does.
    if let Some((seg, share, count)) = m.long.as_ref().and_then(|r| r.culprit()) {
        v.segment = seg;
        // One link carrying nearly all of it is a pattern; a spread is a lead.
        v.confidence =
            if share >= 0.7 && count >= 2 { Confidence::Likely } else { Confidence::Possible };
        v.cost = i18n::cost_long(count, m.long.as_ref().map_or(0, |r| r.ticks.len()));
        v.actions = actions_for_key(findings, "long_run");
        v.actions.extend(worst_of(seg).into_iter().take(2));
        return v;
    }

    // 4. Latency, blamed on whichever segment actually contributed it. The
    //    threshold is this machine's own history where there is enough of it,
    //    so a satellite link is not told it is broken for being satellite.
    if let Some((lan, isp, far)) = shares {
        let total = lan + isp + far;
        // Where this machine has a history, that history decides — and it
        // decides *both ways*. Falling back to the fixed threshold whenever
        // it happens to fire was the same as not having a baseline at all: a
        // satellite or LTE link sits above `ping_bad_ms` permanently, so
        // every scan named the provider on a line running exactly as it
        // always does. The settings threshold is what we use when we have no
        // history to compare against, not an override for when we do.
        let latency_is_a_fault = match m.baseline {
            Some((usual, _)) => total > usual * 1.6 && total - usual > 15.0,
            None => has("ping"),
        };
        if latency_is_a_fault {
            let seg = if lan >= isp && lan >= far {
                Segment::Lan
            } else if isp >= far {
                Segment::Isp
            } else {
                Segment::Internet
            };
            v.segment = seg;
            // Attribution without the middle hop is an educated guess.
            v.confidence = if m.edge.is_some() { Confidence::Likely } else { Confidence::Possible };
            v.cost = i18n::cost_latency(total);
            v.actions = worst_of(seg);
            return v;
        }
    }

    // 5. Jitter. On Wi-Fi it is almost always the air; on a cable it is not.
    if has("jitter") {
        let jitter = m.internet.as_ref().and_then(|s| s.jitter).unwrap_or(0.0);
        let wifi = m.medium == Medium::Wifi;
        let seg = if wifi { Segment::Lan } else { Segment::Isp };
        v.segment = seg;
        // Unstable latency on a Wi-Fi link that is also weak is not a
        // coincidence worth hedging about.
        v.confidence = match m.signal_pct {
            Some(pct) if wifi && pct < 65 => Confidence::Certain,
            _ => Confidence::Likely,
        };
        v.cost = i18n::cost_jitter(jitter);
        v.actions = worst_of(seg);
        return v;
    }

    // 6. DNS. The line is fine; the wait happens before it is used.
    if has("dns_slow") || has("dns_router") {
        v.segment = Segment::Dns;
        v.confidence = Confidence::Likely;
        v.cost = i18n::cost_dns(m.dns_ms.unwrap_or(0.0));
        v.actions = worst_of(Segment::Dns);
        return v;
    }

    // 7. Nothing is wrong *now*. A scan is a thirty-second window, and the
    //    complaint that brought the user here is usually about something that
    //    happens twice an evening. The recorded outages are the only evidence
    //    of that, and a clean instant reading must not be allowed to overrule
    //    them — it can only say the fault was not happening while we looked.
    //    `hist_other` records a degradation whose scope was never established,
    //    so it is skipped rather than blamed on the nearest segment.
    let recorded = findings
        .iter()
        .filter(|f| f.key.starts_with("hist_"))
        .find_map(|f| segment_of(&f.key).map(|seg| (f, seg)));
    if let Some((f, seg)) = recorded {
        v.segment = seg;
        v.confidence = Confidence::Possible;
        v.cost = i18n::cost_intermittent().into();
        v.actions = worst_of(seg);
        if v.actions.is_empty() && !f.advice.is_empty() {
            v.actions = vec![Action { text: f.advice.clone(), tweak_id: f.tweak_id.clone() }];
        }
        return v;
    }

    // 8. Nothing is wrong on the wire, but something on this machine is set
    //    against itself. Worth saying, never worth alarming about.
    let config = worst_of(Segment::Config);
    if !config.is_empty() {
        v.segment = Segment::Config;
        v.confidence = Confidence::Possible;
        v.cost = i18n::cost_config().into();
        v.actions = config;
        return v;
    }

    v.segment = Segment::Healthy;
    // Clean is only certain when the far end was measured. With the pings
    // filtered, a working connection is known, its loss and latency are not.
    v.confidence = if m.internet.is_some() { Confidence::Certain } else { Confidence::Likely };
    v.cost = i18n::cost_none().into();
    v
}

/// The first segment along the chain where packets start going missing.
///
/// `tolerated` is the user's own threshold. It used to be hardcoded at 1%
/// while the findings honoured the setting, so a scan could report loss as
/// acceptable in the list and blame a segment for it in the verdict.
fn loss_origin(m: &Measurements, tolerated: f64) -> Option<(Segment, f64)> {
    let lan = m.gateway.as_ref().map(|s| s.loss_pct).unwrap_or(0.0);
    let edge = m.edge.as_ref().map(|(_, s)| s.loss_pct).unwrap_or(0.0);
    let far = m.internet.as_ref().map(|s| s.loss_pct).unwrap_or(0.0);
    // A hop that deprioritises its own ICMP replies reports loss that the
    // traffic through it never sees, so the middle hop only counts as the
    // origin when the far end is losing packets too.
    if lan > tolerated {
        Some((Segment::Lan, lan))
    } else if edge > tolerated && far > tolerated {
        // Same rule as the latency split: a hop we walked to that is still the
        // user's own equipment is part of their LAN, whatever it costs.
        let seg = if m.edge_is_local { Segment::Lan } else { Segment::Isp };
        Some((seg, edge.max(far)))
    } else if far > tolerated {
        Some((Segment::Internet, far))
    } else {
        None
    }
}

/// The advice carried by one named finding, as an action. Used when a rule
/// knows which finding drove it but the finding is filed under a different
/// segment than the verdict landed on.
fn actions_for_key(findings: &[Finding], key: &str) -> Vec<Action> {
    findings
        .iter()
        .filter(|f| f.key == key && !f.advice.is_empty())
        .map(|f| Action { text: f.advice.clone(), tweak_id: f.tweak_id.clone() })
        .collect()
}

/// The findings that belong to a segment, worst first, as at most three
/// actions. A plan longer than three items is a list again.
fn actions_for(findings: &[Finding], seg: Segment) -> Vec<Action> {
    findings
        .iter()
        .filter(|f| f.severity >= Severity::Warn || f.tweak_id.is_some())
        .filter(|f| segment_of(&f.key) == Some(seg))
        .filter(|f| !f.advice.is_empty())
        .take(3)
        .map(|f| Action { text: f.advice.clone(), tweak_id: f.tweak_id.clone() })
        .collect()
}

/// Which segment a finding speaks about. Keys are stable; titles are not.
fn segment_of(key: &str) -> Option<Segment> {
    Some(match key {
        "medium" | "signal" | "band" | "gateway" | "gateway_silent" | "hist_lan" => Segment::Lan,
        "power" | "mtu" | "tcp_autotuning" | "net_throttling" | "hist_adapter" => Segment::Config,
        "load" => Segment::Uplink,
        "edge" | "edge_silent" | "internet_silent" | "hist_isp" => Segment::Isp,
        "loss" | "jitter" | "ping" | "internet" | "baseline" | "tcp_blocked" | "tcp_slow" => {
            Segment::Internet
        }
        "dns_resolve" | "dns_slow" | "dns_router" | "hist_dns" => Segment::Dns,
        _ => return None,
    })
}

fn cost_for(seg: Segment, m: &Measurements) -> String {
    match seg {
        Segment::Dns => i18n::cost_dns(m.dns_ms.unwrap_or(0.0)),
        Segment::Uplink => i18n::cost_load(m.load.as_ref().and_then(|l| l.bump_ms).unwrap_or(0.0)),
        Segment::Healthy => i18n::cost_none().into(),
        Segment::Unmeasured => i18n::cost_unmeasured().into(),
        _ => i18n::cost_down().into(),
    }
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

/// 169.254.0.0/16, the address Windows gives itself when DHCP does not answer.
///
/// `Ipv4Addr::is_private` does not cover it — that is 10/8, 172.16/12 and
/// 192.168/16 — so before this the app had no name for one of the most common
/// domestic failures there is. A card with a link-local address is working;
/// what failed is the conversation with the router, and "no connection" is the
/// wrong thing to tell someone whose cable is plugged in and whose lights are
/// on.
pub fn is_apipa(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 169 && o[1] == 254
}

/// Frames the cable corrupted, and a link that negotiated a fraction of
/// what gigabit hardware would.
///
/// Ethernet only. Wi-Fi drivers count errors by their own rules (some count
/// every retransmission, some nothing at all), so a figure from them would
/// look like evidence and mean nothing.
// ponytail: Wi-Fi retry rates would need the driver's own statistics (WLAN
// `wlan_intf_opcode_statistics`), not the generic interface row.
fn check_link(
    net: &NetState,
    before: Option<LinkCounters>,
    after: Option<LinkCounters>,
) -> Vec<Finding> {
    if net.medium != Medium::Ethernet {
        return Vec::new();
    }
    let mut out = Vec::new();

    let during = before.zip(after).and_then(|(b, a)| a.since(&b));
    let finding = match (during, after) {
        (Some(d), _) if corrupting(&d) => Some((i18n::f_link_errors_now(), Severity::Warn, d)),
        // Nothing during the scan: say what the totals show, but as history.
        (_, Some(total)) if corrupting(&total) => {
            Some((i18n::f_link_errors_before(), Severity::Info, total))
        }
        _ => None,
    };
    if let Some((title, severity, c)) = finding {
        out.push(
            Finding::new(
                "link_errors",
                title,
                severity,
                i18n::f_link_errors_detail(c.errors, c.packets, error_pct(&c)),
            )
            .advise(i18n::f_link_errors_advice()),
        );
    }

    // Zero is "not reported", not a speed.
    if (1..=100).contains(&net.link_speed_mbps) {
        out.push(
            Finding::new(
                "link_slow",
                i18n::f_link_slow(),
                Severity::Info,
                i18n::f_link_slow_detail(&net.adapter_name, net.link_speed_mbps),
            )
            .advise(i18n::f_link_slow_advice()),
        );
    }
    out
}

/// Fewest frames a share of errors is read from: two bad frames out of ten
/// is noise, not a rate.
const LINK_MIN_PACKETS: u64 = 200;
/// A healthy cable corrupts essentially nothing; one frame in a thousand is
/// already a cable worth replacing.
const LINK_ERROR_PCT: f64 = 0.1;

fn error_pct(c: &LinkCounters) -> f64 {
    if c.packets == 0 {
        return 0.0;
    }
    c.errors as f64 * 100.0 / c.packets as f64
}

fn corrupting(c: &LinkCounters) -> bool {
    c.packets >= LINK_MIN_PACKETS && c.errors > 0 && error_pct(c) >= LINK_ERROR_PCT
}

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

    // Checked before the medium is reported, because it outranks it: on a
    // link-local address the link itself is up and nothing beyond this
    // machine is reachable, so describing the Wi-Fi quality first would bury
    // the one thing that is wrong.
    if net.local_ip.is_some_and(is_apipa) {
        let ip = net.local_ip.map(|i| i.to_string()).unwrap_or_default();
        return vec![Finding::new(
            "medium",
            i18n::f_apipa(),
            Severity::Critical,
            i18n::f_apipa_detail(&net.adapter_name, &ip),
        )
        .advise(i18n::f_apipa_advice())];
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
    let rssi = net.rssi_dbm.map(|r| format!(", {r} dBm")).unwrap_or_default();
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
        out.push(Finding::new("signal", i18n::f_signal_good(sig), Severity::Good, detail.clone()));
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
        Some(false) => {
            vec![Finding::new("power", i18n::f_power_bad(), Severity::Critical, state.text)
                .advise(i18n::f_power_bad_advice())
                .fixed_by("adapter_power")]
        }
        Some(true) => vec![Finding::new("power", i18n::f_power_good(), Severity::Good, state.text)],
        None => vec![Finding::new("power", i18n::f_power_unknown(), Severity::Info, state.text)],
    }
}

fn report_dns(net: &NetState, wire: &Wire, m: &mut Measurements) -> Vec<Finding> {
    let servers = if net.dns_servers.is_empty() {
        i18n::f_dns_from_dhcp().to_string()
    } else {
        net.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ")
    };

    // A resolver the user runs (Pi-hole, AdGuard, a company server) is
    // checked like any other, but the public one is never offered in its place.
    let own = net.dns_is_own_resolver();
    let offer_fix = |f: Finding| if own { f } else { f.fixed_by("fast_dns") };
    let mut out = Vec::new();
    let (ms, err) = (wire.dns.0, wire.dns.1.clone());
    m.dns_ms = ms;
    match (ms, err.is_empty()) {
        (_, false) => out.push(offer_fix(
            Finding::new("dns_resolve", i18n::f_dns_failing(), Severity::Critical, err).advise(
                if own { i18n::f_dns_own_failing_advice() } else { i18n::f_dns_failing_advice() },
            ),
        )),
        (Some(ms), true) if ms > 150.0 => out.push(offer_fix(
            Finding::new(
                "dns_slow",
                i18n::f_dns_slow(ms),
                Severity::Warn,
                i18n::f_dns_servers(&servers),
            )
            .advise(i18n::f_dns_slow_advice()),
        )),
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

/// Formats a `Stats` the way every latency finding shows it.
fn stats_line(s: &Stats) -> String {
    i18n::f_stats_line(s.avg, s.min, s.max, s.jitter, s.loss_pct)
}

/// The cadence every segment is measured at. Slow enough that three series
/// running side by side stay well under the traffic of a single web page, so
/// the scan measures the link rather than itself.
const PROBE_GAP_MS: u64 = 60;

/// `Err` carries why nothing could be sent, which is not the same as nothing
/// coming back.
fn measure(host: Ipv4Addr, count: usize, cfg: &Settings) -> Result<Stats, String> {
    let samples = icmp::ping_series(host, count, cfg.ping_timeout_ms, PROBE_GAP_MS)
        .map_err(|e| e.describe())?;
    let rtts: Vec<f64> = samples.iter().flatten().copied().collect();
    Ok(store::summarise(samples.len(), &rtts))
}

/// The hop past the gateway that the scan measured, and whether it turned out
/// to be someone else's equipment or still the user's own.
struct Edge {
    addr: Ipv4Addr,
    stats: Stats,
    /// True when every hop we could see is still inside the house: a second
    /// router, a mesh node, a modem left in router mode. Blaming the provider
    /// for latency added by the user's own spare router is exactly the kind of
    /// wrong answer this scan exists to stop giving.
    local: bool,
}

/// Every reading that has to touch the network, taken simultaneously.
#[derive(Default)]
struct Wire {
    gateway: Option<Stats>,
    edge: Option<Edge>,
    internet: Option<Stats>,
    /// The same, to [`ANCHOR_ALT`].
    internet_alt: Option<Stats>,
    /// A connection to port 443 of either anchor: `Ok` when one accepted.
    tcp: Option<Result<f64, String>>,
    /// Which anchor accepted it.
    tcp_via: Option<Ipv4Addr>,
    dns: (Option<f64>, String),
    /// Why a ping series could not be sent at all, when one could not. The
    /// three latencies are then not readings, and nothing may be read off
    /// them: a router that was never asked is not a silent router.
    blind: Option<String>,
}

/// 100.64.0.0/10, the carrier-grade NAT range. Unlike the RFC 1918 ranges this
/// one is unambiguous: a household never numbers itself out of it, so a hop
/// here is the provider however private the address looks.
fn is_cgnat(a: Ipv4Addr) -> bool {
    let o = a.octets();
    o[0] == 100 && (64..128).contains(&o[1])
}

/// Whether an answer from `a` says anything about the internet.
///
/// A private address is the user's own equipment: a Pi-hole, a NAS, a second
/// router. A link-local one is this machine's failed lease, and a CGNAT one is
/// the provider's own network, which [`find_edge`] already treats as the
/// provider's edge rather than the open internet. None of them answering can
/// show that the line past the provider works.
pub fn is_public(a: Ipv4Addr) -> bool {
    !(a.is_private()
        || a.is_loopback()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_multicast()
        || is_apipa(a)
        || is_cgnat(a))
}

/// Walk the path and pick the hop that represents the provider's edge.
///
/// The first hop past the gateway is not automatically the provider: on a
/// double-NAT setup — a second router, an ISP box left in router mode, a mesh
/// controller — it is another of the user's own devices, and charging its
/// latency to the provider produces a confident, wrong verdict. So a private
/// hop is walked past, and only a public or CGNAT address is treated as the
/// edge. If the whole visible path is private, the last private hop is
/// measured anyway and flagged as local, because the milliseconds are real
/// even when the owner is not who we assumed.
fn find_edge(gw: Ipv4Addr, cfg: &Settings) -> Option<(Ipv4Addr, bool)> {
    // Six hops clears the customer edge on any residential line, and a short
    // walk keeps a path of silent routers from costing the user a minute.
    let hops = icmp::traceroute(ANCHOR, 6, cfg.ping_timeout_ms);
    let mut first_private = None;

    for addr in hops.iter().filter_map(|h| h.addr) {
        if addr == gw {
            continue;
        }
        // A link-local hop is this machine's own failure to get an address,
        // not somebody's edge router; treating it as public would name it as
        // the provider's and measure the latency to a nonexistent lease.
        if (!addr.is_private() && !is_apipa(addr)) || is_cgnat(addr) {
            return Some((addr, false));
        }
        first_private.get_or_insert(addr);
    }
    first_private.map(|addr| (addr, true))
}

/// Take every network reading at once.
///
/// These probes are independent, and running them concurrently is worth it
/// twice over: the scan stops spending half a minute waiting on timeouts one
/// at a time, and — the part that actually matters — the three latencies the
/// segment split subtracts from each other finally describe the same instant.
fn measure_wire(net: &NetState, cfg: &Settings) -> Wire {
    use crate::probe::netstate::resolvers_answer;
    use crate::settings::DNS_TEST_HOST;

    let mut wire = Wire::default();

    std::thread::scope(|s| {
        let gateway = net.gateway.map(|gw| s.spawn(move || measure(gw, 10, cfg)));
        let internet = s.spawn(|| measure(ANCHOR, 15, cfg));
        let internet_alt = s.spawn(|| measure(ANCHOR_ALT, 15, cfg));
        let tcp = s.spawn(|| tcp_probe(ANCHOR, cfg));
        let tcp_alt = s.spawn(|| tcp_probe(ANCHOR_ALT, cfg));
        // Straight to the adapter's resolvers, past the Windows cache (see
        // `resolvers_answer`), and asked twice before a failure stands: a
        // scan is one reading, and one lost reply is not a broken resolver.
        let dns = s.spawn(|| {
            let first = resolvers_answer(&net.dns_servers, DNS_TEST_HOST);
            if first.1.is_empty() {
                first
            } else {
                resolvers_answer(&net.dns_servers, DNS_TEST_HOST)
            }
        });
        // The traceroute has to finish before its result can be pinged, so the
        // whole two-step sequence lives on one thread rather than blocking the
        // others behind it.
        let edge = net.gateway.map(|gw| {
            s.spawn(move || {
                find_edge(gw, cfg).map(|(addr, local)| (addr, measure(addr, 10, cfg), local))
            })
        });

        // A series that could not be sent leaves its leg empty and says why.
        let mut blind = None;
        let mut taken = |r: Result<Stats, String>| r.map_err(|e| blind = Some(e)).ok();

        wire.gateway = gateway.and_then(|h| h.join().ok()).and_then(&mut taken);
        wire.internet = internet.join().ok().and_then(&mut taken);
        wire.internet_alt = internet_alt.join().ok().and_then(&mut taken);
        // Either anchor accepting a connection is a working transport; only
        // both refusing is a block.
        (wire.tcp, wire.tcp_via) = match (tcp.join().ok(), tcp_alt.join().ok()) {
            (Some(Ok(ms)), _) => (Some(Ok(ms)), Some(ANCHOR)),
            (_, Some(Ok(ms))) => (Some(Ok(ms)), Some(ANCHOR_ALT)),
            (first, second) => (first.or(second), None),
        };
        wire.dns = dns.join().unwrap_or((None, String::new()));
        wire.edge = edge
            .and_then(|h| h.join().ok())
            .flatten()
            .and_then(|(addr, stats, local)| taken(stats).map(|stats| Edge { addr, stats, local }));
        wire.blind = blind;
    });

    wire
}

/// Whether anything past the router answered: either anchor to a ping, or
/// either to a TCP connection.
fn internet_answered(wire: &Wire) -> bool {
    let echoed = |s: &Option<Stats>| s.as_ref().is_some_and(|s| s.avg.is_some());
    echoed(&wire.internet) || echoed(&wire.internet_alt) || matches!(wire.tcp, Some(Ok(_)))
}

/// The findings read off the pings and the TCP probe.
///
/// When a ping series could not be sent, none of the three latencies is a
/// reading, so the findings that compare or judge them are not made at all.
/// Only what does not depend on them stays: a missing gateway is known from
/// the adapter, and the TCP probe went out on its own socket.
fn report_wire(
    net: &NetState,
    store: &Store,
    cfg: &Settings,
    wire: &Wire,
    m: &mut Measurements,
) -> Vec<Finding> {
    let mut out = Vec::new();
    match &wire.blind {
        None => {
            out.extend(report_link(net, wire, m));
            out.extend(report_edge(wire, m));
            out.extend(report_internet(store, cfg, wire, m));
        }
        Some(why) => {
            if net.gateway.is_none() {
                out.extend(report_link(net, wire, m));
            }
            out.push(
                Finding::new("icmp_blind", i18n::f_icmp_blind(), Severity::Warn, why.clone())
                    .advise(i18n::f_icmp_blind_advice()),
            );
        }
    }
    out.extend(report_reachability(wire, m));
    out
}

fn report_link(net: &NetState, wire: &Wire, m: &mut Measurements) -> Vec<Finding> {
    let Some(gw) = net.gateway else {
        return vec![Finding::new(
            "gateway_silent",
            i18n::f_no_gateway(),
            Severity::Critical,
            i18n::f_no_gateway_detail(),
        )
        .advise(i18n::f_no_gateway_advice())];
    };

    let stats = match &wire.gateway {
        Some(s) if s.avg.is_some() => s.clone(),
        // Plenty of routers drop pings addressed to themselves and forward
        // everything else. With traffic getting through, a silent router is
        // a setting on it, not a dead link: calling that "LAN, clear" sent
        // people to reboot a router that was working.
        _ if internet_answered(wire) => {
            return vec![Finding::new(
                "gateway_mute",
                i18n::f_router_mute(),
                Severity::Info,
                i18n::f_router_mute_detail(&gw.to_string()),
            )]
        }
        _ => {
            return vec![Finding::new(
                "gateway_silent",
                i18n::f_router_silent(),
                Severity::Critical,
                i18n::f_router_silent_detail(&gw.to_string()),
            )
            .advise(i18n::f_router_silent_advice())]
        }
    };

    let detail = stats_line(&stats);
    let unstable = stats.loss_pct > 0.0
        || stats.avg.unwrap_or(0.0) > 15.0
        || stats.jitter.unwrap_or(0.0) > 10.0;
    m.gateway = Some(stats);

    if unstable {
        vec![Finding::new("gateway", i18n::f_link_unstable(), Severity::Warn, detail)
            .advise(i18n::f_link_unstable_advice())]
    } else {
        vec![Finding::new("gateway", i18n::f_link_healthy(), Severity::Good, detail)]
    }
}

/// What the hop past the router turned out to be. This is the measurement that
/// makes the rest of the scan able to assign blame at all: without it, every
/// millisecond past the gateway is one undivided lump, and "your Wi-Fi" and
/// "your provider" look identical.
fn report_edge(wire: &Wire, m: &mut Measurements) -> Vec<Finding> {
    let Some(edge) = &wire.edge else {
        return vec![Finding::new(
            "edge_unknown",
            i18n::f_edge_unknown(),
            Severity::Info,
            i18n::f_edge_unknown_detail(),
        )];
    };

    let addr = edge.addr.to_string();
    let Some(avg) = edge.stats.avg else {
        // On the path but mute. Routers that drop ICMP entirely are common, so
        // this is only alarming when the far end is also unreachable — which
        // the internet check decides, not this one.
        return vec![Finding::new(
            "edge_unknown",
            i18n::f_edge_unknown(),
            Severity::Info,
            i18n::f_edge_silent_detail(&addr),
        )];
    };

    let detail = i18n::f_edge_detail(&addr, &stats_line(&edge.stats));
    let added = avg - Measurements::avg(&m.gateway).unwrap_or(0.0);
    let loss = edge.stats.loss_pct;
    m.edge = Some((edge.addr, edge.stats.clone()));
    m.edge_is_local = edge.local;

    // A second box of the user's own is a different finding with a different
    // owner, even when the numbers coming off it are identical.
    if edge.local {
        return if added > 20.0 {
            vec![Finding::new(
                "gateway",
                i18n::f_local_hop_slow(&addr, added),
                Severity::Warn,
                detail,
            )
            .advise(i18n::f_local_hop_slow_advice())]
        } else {
            vec![Finding::new("edge", i18n::f_local_hop(&addr, avg), Severity::Info, detail)
                .advise(i18n::f_local_hop_advice())]
        };
    }

    if loss > 2.0 {
        vec![Finding::new("edge", i18n::f_edge_lossy(loss), Severity::Warn, detail)
            .advise(i18n::f_edge_lossy_advice())]
    } else if added > 40.0 {
        vec![Finding::new("edge", i18n::f_edge_slow(added), Severity::Warn, detail)
            .advise(i18n::f_edge_slow_advice())]
    } else {
        vec![Finding::new("edge", i18n::f_edge_found(&addr, avg), Severity::Good, detail)]
    }
}

fn report_internet(
    store: &Store,
    cfg: &Settings,
    wire: &Wire,
    m: &mut Measurements,
) -> Vec<Finding> {
    // Of the anchors that answered, the one that lost less. Loss introduced
    // on the shared path shows on both; loss on one alone is that responder,
    // or a filter in front of it.
    let answered = |s: &Option<Stats>| s.clone().filter(|s| s.avg.is_some());
    let (primary, alt) = (answered(&wire.internet), answered(&wire.internet_alt));
    let on_primary = match (&primary, &alt) {
        (Some(p), Some(a)) => p.loss_pct <= a.loss_pct,
        (p, _) => p.is_some(),
    };
    let Some(stats) = (if on_primary { primary } else { alt }) else {
        // Neither anchor echoed. A connection that still got through means
        // the pings were filtered, not that the internet is gone: only all
        // three agreeing is an outage.
        if matches!(wire.tcp, Some(Ok(_))) {
            return vec![Finding::new(
                "icmp_filtered",
                i18n::f_icmp_filtered(),
                Severity::Warn,
                i18n::f_icmp_filtered_detail(&format!("{ANCHOR}, {ANCHOR_ALT}")),
            )];
        }
        return vec![Finding::new(
            "internet_silent",
            i18n::f_net_silent(),
            Severity::Critical,
            i18n::f_net_silent_detail(),
        )
        .advise(i18n::f_net_silent_advice())];
    };
    let Some(avg) = stats.avg else { return Vec::new() };

    let jitter = stats.jitter.unwrap_or(0.0);
    let spread = stats.max.unwrap_or(0.0) - stats.min.unwrap_or(0.0);
    let detail = stats_line(&stats);
    let loss = stats.loss_pct;
    m.internet = Some(stats);

    // This machine's own history, where there is enough of it. A threshold
    // from a settings file describes a hypothetical line; this describes the
    // one in front of the user, and only it can say "worse than usual".
    let key = if on_primary { ANCHOR_KEY } else { ANCHOR_ALT_KEY };
    let history = store.stats(key, BASELINE_WINDOW_S);
    if history.count >= BASELINE_MIN_SAMPLES {
        // The median, not the mean. A week that contained one bad evening has
        // a mean pulled up by it, and a baseline that has absorbed the fault
        // is a baseline that will not report the next one.
        if let Some(usual) = history.median {
            m.baseline = Some((usual, history.count));
        }
    }

    let mut out = Vec::new();
    if loss > cfg.loss_ok_pct {
        out.push(
            Finding::new("loss", i18n::f_loss(loss), Severity::Critical, detail.clone())
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
    if let Some((usual, count)) = m.baseline {
        // A line that is normally 90 ms is not broken for being 90 ms, and one
        // that is normally 8 ms is in trouble at 30 long before any fixed
        // threshold would notice.
        if avg > usual * 1.6 && avg - usual > 15.0 {
            out.push(
                Finding::new(
                    "baseline",
                    i18n::f_baseline_worse(avg, usual),
                    Severity::Warn,
                    i18n::f_baseline_detail(avg, usual, count),
                )
                .advise(i18n::f_baseline_advice()),
            );
        }
    }
    if out.is_empty() {
        out.push(Finding::new("internet", i18n::f_net_ok(avg), Severity::Good, detail));
    }
    out
}

/// Ping proves a path exists. It does not prove anything the user cares about
/// can travel down it. A captive portal, a corporate firewall or a misbehaving
/// proxy all answer ICMP happily while every page times out, and that failure
/// is invisible to every other check in this scan.
///
/// The probe goes to a literal address so that a broken resolver cannot be
/// mistaken for a broken transport — DNS is checked separately, on purpose.
pub(crate) fn tcp_probe(anchor: Ipv4Addr, cfg: &Settings) -> Result<f64, String> {
    let addr = SocketAddr::from((anchor, 443));
    let timeout = Duration::from_millis((cfg.ping_timeout_ms as u64 * 3).max(2000));
    let started = Instant::now();
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_) => Ok(started.elapsed().as_secs_f64() * 1000.0),
        Err(e) => Err(e.to_string()),
    }
}

fn report_reachability(wire: &Wire, m: &mut Measurements) -> Vec<Finding> {
    let host = match (&wire.tcp, wire.tcp_via) {
        (Some(Ok(_)), Some(via)) => via.to_string(),
        _ => format!("{ANCHOR}, {ANCHOR_ALT}"),
    };
    let Some(result) = &wire.tcp else {
        return Vec::new();
    };

    match result {
        Err(e) => {
            m.tcp_blocked = true;
            // Only a contradiction is interesting: if ICMP failed too, the
            // internet check already said so and this adds nothing.
            if m.internet.is_none() {
                return Vec::new();
            }
            vec![Finding::new(
                "tcp_blocked",
                i18n::f_tcp_blocked(),
                Severity::Critical,
                i18n::f_tcp_blocked_detail(&host, e),
            )
            .advise(i18n::f_tcp_blocked_advice())]
        }
        Ok(ms) => {
            let ms = *ms;
            m.tcp_ms = Some(ms);
            let ping = Measurements::avg(&m.internet).unwrap_or(0.0);
            let detail = i18n::f_tcp_detail(&host, ms, ping);
            // One round trip to open, so anything far above the ping to the
            // same address is something in the middle, not the distance.
            if ping > 0.0 && ms > ping * 4.0 + 100.0 {
                vec![Finding::new("tcp_slow", i18n::f_tcp_slow(ms), Severity::Warn, detail)
                    .advise(i18n::f_tcp_slow_advice())]
            } else {
                vec![Finding::new("tcp", i18n::f_tcp_ok(&host, ms), Severity::Good, detail)]
            }
        }
    }
}

/// The check that reproduces the complaint instead of sampling around it.
///
/// An idle line says nothing about a line in use. Bufferbloat — a fat queue in
/// the router or the modem that fills the moment a transfer starts and holds
/// every other packet behind it — is the ordinary reason a connection that
/// benchmarks well still drops calls, and no amount of idle pinging will ever
/// show it.
fn check_load(
    cfg: &Settings,
    progress: Option<bandwidth::Progress>,
    m: &mut Measurements,
) -> Vec<Finding> {
    // Shorter than the dedicated tab's run: enough to fill the queue and read
    // the grade, not enough to make a scan feel like a speed test.
    let res = bandwidth::run(
        ANCHOR,
        Duration::from_secs(4),
        Duration::from_secs(8),
        cfg.ping_timeout_ms,
        progress,
    );

    if !res.error.is_empty() && res.grade.is_none() {
        m.load = Some(res.clone());
        return vec![Finding::new("load", i18n::f_load_skipped(), Severity::Info, res.error)];
    }

    let bump = res.worst_bump().unwrap_or(0.0);
    let grade = res.grade_or_unknown();
    let mut detail = i18n::f_load_detail(
        res.idle_avg.unwrap_or(0.0),
        res.loaded_avg.unwrap_or(0.0),
        res.mbps.unwrap_or(0.0),
        grade.letter(),
    );
    if let Some(up) = &res.upload {
        detail.push(' ');
        detail.push_str(&match (up.loaded_avg, up.mbps) {
            (Some(loaded), Some(mbps)) => i18n::f_load_detail_up(loaded, mbps),
            _ => up.note.clone(),
        });
    }
    m.load = Some(res);

    match grade {
        Grade::D | Grade::F => {
            vec![Finding::new("load", i18n::f_load_bad(bump), Severity::Critical, detail)
                .advise(i18n::f_load_bad_advice())]
        }
        Grade::C => vec![Finding::new("load", i18n::f_load_bad(bump), Severity::Warn, detail)
            .advise(i18n::f_load_bad_advice())],
        _ => vec![Finding::new("load", i18n::f_load_ok(bump), Severity::Good, detail)
            .advise(i18n::f_load_ok_advice())],
    }
}

/// Minutes of watching the same three links, for the fault that comes and
/// goes. See [`crate::longrun`].
fn check_long(
    opts: LongOpts,
    cfg: &Settings,
    progress: Option<Progress>,
    m: &mut Measurements,
) -> Vec<Finding> {
    // Without ICMP there is nothing to watch with, and `icmp_blind` has
    // already said why.
    if m.blind.is_some() {
        return Vec::new();
    }
    let ticks: Option<longrun::Progress> = progress.map(|p| {
        Arc::new(move |done: usize, total: usize| {
            p(&i18n::step_long(done, total), 0.2 + 0.78 * done as f32 / total.max(1) as f32)
        }) as longrun::Progress
    });
    let started_at = store::now();
    let recorded = longrun::record(
        m.gateway_addr,
        m.edge.as_ref().map(|(addr, _)| *addr),
        [ANCHOR, ANCHOR_ALT],
        opts.secs,
        cfg.ping_timeout_ms,
        Arc::clone(&opts.cancel),
        ticks,
    );
    let ticks = match recorded {
        Ok(t) => t,
        Err(why) => {
            return vec![Finding::new("long_run", i18n::f_long_failed(), Severity::Info, why)]
        }
    };
    let cancelled = opts.cancel.load(std::sync::atomic::Ordering::Relaxed);
    let mut run = longrun::analyse(ticks, opts.secs, m.edge_is_local, cancelled);
    run.started_at = started_at;
    let finding = long_finding(&run);
    m.long = Some(run);
    vec![finding]
}

/// The long run, as one finding: what was watched, how long, what broke.
fn long_finding(run: &LongRun) -> Finding {
    let watched = run.ticks.len();
    let found: Vec<_> = run.significant().collect();
    let drops = found.iter().filter(|e| e.kind == Trouble::Loss).count();
    let spikes = found.len() - drops;
    let lone = run.episodes.len() - found.len();

    let Some((seg, share, _)) = run.culprit() else {
        return Finding::new(
            "long_run",
            i18n::f_long_clean(watched, run.cancelled),
            Severity::Good,
            i18n::f_long_clean_detail(lone, spikes),
        );
    };
    let severity = if drops > 0 { Severity::Critical } else { Severity::Warn };
    let bad_s: usize = found.iter().map(|e| e.len_s).sum();
    Finding::new(
        "long_run",
        i18n::f_long_found(found.len(), watched, seg.label(), run.cancelled),
        severity,
        i18n::f_long_detail(drops, spikes, bad_s, share * 100.0),
    )
    .advise(match seg {
        Segment::Lan => i18n::f_long_advice_lan(),
        Segment::Isp => i18n::f_long_advice_isp(),
        _ => i18n::f_long_advice_far(),
    })
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

/// Total time down in one scope, over the last day, that is worth calling
/// critical on its own. Five minutes off the network is a lost meeting.
const HIST_CRITICAL_DOWN_S: f64 = 300.0;

/// The span the history check reads.
const HIST_WINDOW_S: f64 = 24.0 * 3600.0;
/// How much of it has to have been watched for "no outages" to count as a
/// clean day. Short of it, the finding says how much was.
const HIST_WATCHED_ENOUGH: f64 = 0.95;

fn check_history(_net: &NetState, store: &Store, _cfg: &Settings) -> Vec<Finding> {
    let events = store.events_since(24.0 * 3600.0);
    if events.is_empty() {
        // An empty history is only good news for the part of the day that
        // was watched. Minutes after a first start it is no news at all.
        let now = store::now();
        let watched = store.observed_seconds(now - HIST_WINDOW_S, now, store::OBSERVATION_GAP_S);
        if watched >= HIST_WINDOW_S * HIST_WATCHED_ENOUGH {
            return vec![Finding::new(
                "history",
                i18n::f_hist_none(),
                Severity::Good,
                i18n::f_hist_none_detail(),
            )];
        }
        return vec![Finding::new(
            "history",
            i18n::f_hist_none_partial(&i18n::span(watched)),
            Severity::Info,
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
    scopes.sort_by_key(|(scope, v)| std::cmp::Reverse((v.len(), monitor::scope_rank(scope))));

    for (scope, items) in scopes {
        let (title, advice) = match scope.as_str() {
            "lan" => (i18n::f_hist_lan(), i18n::f_hist_lan_advice()),
            "adapter" => (i18n::f_hist_adapter(), i18n::f_hist_adapter_advice()),
            "isp" => (i18n::f_hist_isp(), i18n::f_hist_isp_advice()),
            "dns" => (i18n::f_hist_dns(), i18n::f_hist_dns_advice()),
            _ => (i18n::f_hist_other(), i18n::f_hist_other_advice()),
        };

        let durations: Vec<f64> = items.iter().filter_map(|e| e.duration_s()).collect();
        let total_down: f64 = durations.iter().sum();
        let avg = if durations.is_empty() { 0.0 } else { total_down / durations.len() as f64 };
        let times: Vec<String> = items.iter().take(6).map(|e| format_clock(e.ts_start)).collect();

        out.push(
            Finding::new(
                &format!("hist_{scope}"),
                i18n::f_hist_title(title, items.len()),
                // Counting outages alone made three two-second blips
                // CRITICAL and one four-hour blackout a WARNING, which is
                // the opposite of what the user lived through. Either a
                // pattern or a long total is enough to promote it.
                if items.len() >= 3 || total_down >= HIST_CRITICAL_DOWN_S {
                    Severity::Critical
                } else {
                    Severity::Warn
                },
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

/// Time of day to the second, local, for events inside one sitting.
pub fn format_clock_s(ts: f64) -> String {
    let secs = (ts as i64 + local_utc_offset_secs()).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
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
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
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

    fn wired(mbps: u64) -> NetState {
        NetState { medium: Medium::Ethernet, link_speed_mbps: mbps, ..Default::default() }
    }

    fn counters(packets: u64, errors: u64) -> Option<LinkCounters> {
        Some(LinkCounters { packets, errors, bytes: 0 })
    }

    #[test]
    fn a_cable_corrupting_frames_during_the_scan_is_a_warning() {
        let f = check_link(&wired(1000), counters(10_000, 0), counters(10_500, 3));
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].key.as_str(), f[0].severity), ("link_errors", Severity::Warn));
    }

    #[test]
    fn old_errors_are_history_and_a_clean_cable_says_nothing() {
        // Errors in the totals, none while the scan watched: history, not now.
        let f = check_link(&wired(1000), counters(10_000, 50), counters(10_500, 50));
        assert_eq!((f[0].key.as_str(), f[0].severity), ("link_errors", Severity::Info));
        // A handful of errors over millions of frames is a healthy cable.
        assert!(check_link(&wired(1000), counters(5_000_000, 2), counters(5_000_500, 2)).is_empty());
        // Too few frames to read a rate from.
        assert!(check_link(&wired(1000), counters(0, 0), counters(20, 1)).is_empty());
    }

    #[test]
    fn a_reset_adapter_or_unreadable_counters_claim_nothing_about_now() {
        // Counters went backwards: the adapter restarted mid-scan.
        let f = check_link(&wired(1000), counters(10_000, 90), counters(300, 0));
        assert!(f.is_empty());
        assert!(check_link(&wired(1000), None, None).is_empty());
    }

    #[test]
    fn wifi_counters_are_not_read_and_a_slow_cable_is_named() {
        let wifi = NetState { medium: Medium::Wifi, link_speed_mbps: 54, ..Default::default() };
        assert!(check_link(&wifi, counters(0, 0), counters(10_000, 500)).is_empty());
        let f = check_link(&wired(100), None, None);
        assert_eq!(f[0].key, "link_slow");
        assert!(check_link(&wired(0), None, None).is_empty(), "0 is unknown, not slow");
    }

    #[test]
    fn findings_come_back_worst_first() {
        let mut f = [
            Finding::new("a", "good", Severity::Good, ""),
            Finding::new("b", "critical", Severity::Critical, ""),
            Finding::new("c", "warn", Severity::Warn, ""),
        ];
        f.sort_by_key(|f| std::cmp::Reverse(f.severity));
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
        // Without the load test: a unit test has no business saturating the
        // machine's uplink for twelve seconds.
        let scan = scan(&net, &store, &Settings::default(), false, None, None);
        assert!(!scan.findings.is_empty());
        assert!(scan.findings.windows(2).all(|w| w[0].severity >= w[1].severity));
        // Every finding must be presentable: a title and something to show.
        for f in &scan.findings {
            assert!(!f.title.is_empty(), "finding {} has no title", f.key);
        }
        assert!(!scan.verdict.cost.is_empty());
    }

    /// Stats as the scan would have measured them, so the judging rules can be
    /// exercised without a network.
    fn stats(avg: f64, loss: f64) -> Stats {
        Stats {
            count: 10,
            loss_pct: loss,
            avg: Some(avg),
            median: Some(avg),
            min: Some(avg),
            max: Some(avg),
            jitter: Some(0.0),
        }
    }

    #[test]
    fn the_chain_carries_each_links_share_and_its_owner() {
        let m = Measurements {
            gateway: Some(stats(4.0, 0.0)),
            gateway_addr: Some(Ipv4Addr::new(192, 168, 1, 1)),
            edge: Some((Ipv4Addr::new(10, 0, 0, 1), stats(70.0, 0.0))),
            internet: Some(stats(74.0, 0.0)),
            ..Default::default()
        };
        let [lan, isp, far] = chain(&[], &m);
        assert_eq!(lan.state, LinkState::Measured(stats(4.0, 0.0)));
        assert_eq!(lan.addr, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!((lan.added_ms, isp.added_ms, far.added_ms), (Some(4.0), Some(66.0), Some(4.0)));
    }

    #[test]
    fn a_second_router_of_the_users_own_has_no_provider_share() {
        // Its milliseconds went to the LAN share; showing them again on the
        // provider's link would count them twice.
        let m = Measurements {
            gateway: Some(stats(4.0, 0.0)),
            edge: Some((Ipv4Addr::new(192, 168, 0, 1), stats(30.0, 0.0))),
            edge_is_local: true,
            internet: Some(stats(40.0, 0.0)),
            ..Default::default()
        };
        let [lan, isp, _] = chain(&[], &m);
        assert_eq!(lan.added_ms, Some(30.0));
        assert!(isp.local);
        assert_eq!(isp.added_ms, None);
    }

    #[test]
    fn a_scan_that_could_not_ping_claims_nothing_about_any_link() {
        // Even with findings that would otherwise read as silence.
        let m = Measurements { blind: Some("no ICMP handle".into()), ..Default::default() };
        let f = vec![Finding::new("internet_silent", "", Severity::Critical, "")];
        assert!(chain(&f, &m).iter().all(|l| l.state == LinkState::NotMeasured));
    }

    #[test]
    fn a_router_that_ignores_pings_is_filtered_not_silent() {
        let m = Measurements { internet: Some(stats(20.0, 0.0)), ..Default::default() };
        let mute = vec![Finding::new("gateway_mute", "", Severity::Info, "")];
        assert_eq!(chain(&mute, &m)[0].state, LinkState::Filtered);
        let dead = vec![Finding::new("gateway_silent", "", Severity::Critical, "")];
        assert_eq!(chain(&dead, &Measurements::default())[0].state, LinkState::Silent);
        // Filtered pings to the internet with TCP working are not an outage.
        let filtered = vec![Finding::new("icmp_filtered", "", Severity::Warn, "")];
        assert_eq!(chain(&filtered, &Measurements::default())[2].state, LinkState::Filtered);
    }

    fn long_run_with(dead_gw_at: &[usize]) -> LongRun {
        let ok = longrun::Tick { gw: Some(3.0), edge: Some(8.0), net: Some(15.0) };
        let mut ticks = vec![ok; 120];
        for &i in dead_gw_at {
            ticks[i] = longrun::Tick { gw: None, edge: None, net: None };
        }
        longrun::analyse(ticks, 120, false, false)
    }

    #[test]
    fn drops_seen_in_the_long_run_outrank_the_outage_history() {
        // The history blames the provider; the minutes just watched show the
        // router itself going silent. What was measured now, on every link at
        // once, is the stronger evidence.
        let m = Measurements {
            gateway: Some(stats(3.0, 0.0)),
            internet: Some(stats(15.0, 0.0)),
            long: Some(long_run_with(&[10, 11, 12, 70, 71])),
            ..Default::default()
        };
        let findings = vec![
            Finding::new("hist_isp", "drops", Severity::Critical, "").advise("call them"),
            long_finding(m.long.as_ref().unwrap()),
        ];
        let v = judge(&findings, &m, &Settings::default());
        assert_eq!(v.segment, Segment::Lan);
        assert_eq!(v.confidence, Confidence::Likely);
        assert!(!v.actions.is_empty(), "the verdict must come with a next step");
    }

    #[test]
    fn a_quiet_long_run_does_not_clear_the_history() {
        // Five clean minutes say the fault was not happening then. The
        // recorded outages still stand.
        let m = Measurements {
            gateway: Some(stats(3.0, 0.0)),
            internet: Some(stats(15.0, 0.0)),
            long: Some(long_run_with(&[40])),
            ..Default::default()
        };
        let findings = vec![Finding::new("hist_isp", "drops", Severity::Critical, "")];
        assert_eq!(judge(&findings, &m, &Settings::default()).segment, Segment::Isp);
        assert_eq!(long_finding(m.long.as_ref().unwrap()).severity, Severity::Good);
    }

    #[test]
    fn a_hard_break_now_still_outranks_the_long_run() {
        let m = Measurements { long: Some(long_run_with(&[10, 11])), ..Default::default() };
        let findings = vec![Finding::new("internet_silent", "", Severity::Critical, "")];
        assert_eq!(judge(&findings, &m, &Settings::default()).segment, Segment::Isp);
    }

    #[test]
    fn latency_is_charged_to_the_segment_that_introduced_it() {
        // 4 ms to the router, 70 to the provider's first hop, 74 to the world:
        // the provider's edge added 66 of the 74 and owns the verdict.
        let m = Measurements {
            gateway: Some(stats(4.0, 0.0)),
            edge: Some((Ipv4Addr::new(10, 0, 0, 1), stats(70.0, 0.0))),
            internet: Some(stats(74.0, 0.0)),
            ..Default::default()
        };
        let (lan, isp, far) = m.shares().unwrap();
        assert_eq!(lan, 4.0);
        assert_eq!(isp, 66.0);
        assert_eq!(far, 4.0);

        let findings = vec![Finding::new("ping", "high", Severity::Warn, "")];
        assert_eq!(judge(&findings, &m, &Settings::default()).segment, Segment::Isp);
    }

    #[test]
    fn the_same_far_end_latency_can_be_the_users_own_wifi() {
        // Identical 74 ms at the far end, but this time the router itself is
        // 68 ms away. A checklist reports the same number; the split does not.
        let m = Measurements {
            gateway: Some(stats(68.0, 0.0)),
            edge: Some((Ipv4Addr::new(10, 0, 0, 1), stats(72.0, 0.0))),
            internet: Some(stats(74.0, 0.0)),
            ..Default::default()
        };
        let findings = vec![Finding::new("ping", "high", Severity::Warn, "")];
        assert_eq!(judge(&findings, &m, &Settings::default()).segment, Segment::Lan);
    }

    #[test]
    fn a_second_router_of_your_own_is_not_the_provider() {
        // The exact numbers from the isp case above, but the hop past the
        // gateway is an RFC 1918 address — a double-NAT household. Charging
        // those 66 ms to the provider produces a confident, wrong verdict and
        // a support ticket about the user's own spare router.
        let m = Measurements {
            gateway: Some(stats(4.0, 0.0)),
            edge: Some((Ipv4Addr::new(192, 168, 222, 1), stats(70.0, 0.0))),
            edge_is_local: true,
            internet: Some(stats(74.0, 0.0)),
            ..Default::default()
        };
        let (lan, isp, far) = m.shares().unwrap();
        assert_eq!(lan, 70.0, "the whole local chain counts as LAN");
        assert_eq!(isp, 0.0);
        assert_eq!(far, 4.0);

        let findings = vec![Finding::new("ping", "high", Severity::Warn, "")];
        assert_eq!(judge(&findings, &m, &Settings::default()).segment, Segment::Lan);
    }

    #[test]
    fn carrier_grade_nat_is_the_provider_however_private_it_looks() {
        // 100.64/10 is the one range a household never numbers itself out of.
        assert!(is_cgnat(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(is_cgnat(Ipv4Addr::new(100, 127, 255, 254)));
        assert!(!is_cgnat(Ipv4Addr::new(100, 128, 0, 1)));
        assert!(!is_cgnat(Ipv4Addr::new(100, 63, 255, 255)));
        assert!(!is_cgnat(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn loss_at_your_own_second_router_is_not_the_providers_loss() {
        let m = Measurements {
            gateway: Some(stats(3.0, 0.0)),
            edge: Some((Ipv4Addr::new(192, 168, 222, 1), stats(12.0, 6.0))),
            edge_is_local: true,
            internet: Some(stats(20.0, 5.0)),
            ..Default::default()
        };
        assert_eq!(loss_origin(&m, Settings::default().loss_ok_pct), Some((Segment::Lan, 6.0)));
    }

    #[test]
    fn one_long_blackout_outranks_three_blinks() {
        // Counting outages alone made three two-second blips CRITICAL and a
        // single hour-long blackout a WARNING — the opposite of what the
        // user lived through.
        let blips = Store::open_in_memory().unwrap();
        for _ in 0..3 {
            let id = blips.open_event("outage", "lan", "blink", "{}").unwrap();
            blips.close_event(id, "{}").unwrap();
        }
        let long = Store::open_in_memory().unwrap();
        let id = long.open_event("outage", "lan", "blackout", "{}").unwrap();
        long.close_event(id, "{}").unwrap();
        // close_event stamps `now`; stretch it into a real blackout.
        long.reshape_events_for_test(0.0, 3600.0);

        let net = NetState::default();
        let cfg = Settings::default();
        let sev = |s: &Store| {
            check_history(&net, s, &cfg)
                .iter()
                .find(|f| f.key == "hist_lan")
                .map(|f| f.severity)
                .expect("a lan scope finding")
        };

        assert_eq!(sev(&blips), Severity::Critical, "a pattern still counts");
        assert_eq!(sev(&long), Severity::Critical, "and so does an hour off the network");
    }

    #[test]
    fn a_link_that_is_always_slow_is_not_reported_as_broken_every_scan() {
        // Satellite: 600 ms is what this machine has always seen, and a week
        // of samples says so. The fixed threshold from the settings file
        // (ping_bad_ms = 120) fires regardless, and used to win — so every
        // scan named the provider on a line running exactly as it always has.
        let m = Measurements {
            gateway: Some(stats(20.0, 0.0)),
            edge: Some((Ipv4Addr::new(100, 64, 0, 1), stats(300.0, 0.0))),
            internet: Some(stats(600.0, 0.0)),
            baseline: Some((610.0, 5_000)),
            ..Default::default()
        };
        let findings = vec![Finding::new("ping", "high", Severity::Warn, "")];
        assert_eq!(judge(&findings, &m, &Settings::default()).segment, Segment::Healthy);
    }

    #[test]
    fn the_same_link_getting_worse_than_its_own_history_is_reported() {
        // The other half of the rule: the baseline must still be able to
        // convict. Same 610 ms line, now answering in 1200.
        let m = Measurements {
            gateway: Some(stats(20.0, 0.0)),
            edge: Some((Ipv4Addr::new(100, 64, 0, 1), stats(300.0, 0.0))),
            internet: Some(stats(1_200.0, 0.0)),
            baseline: Some((610.0, 5_000)),
            ..Default::default()
        };
        assert_eq!(judge(&[], &m, &Settings::default()).segment, Segment::Internet);
    }

    #[test]
    fn without_a_baseline_the_settings_threshold_still_decides() {
        // A fresh install has no history, and must not go quiet because of it.
        let m = Measurements {
            gateway: Some(stats(20.0, 0.0)),
            edge: Some((Ipv4Addr::new(100, 64, 0, 1), stats(300.0, 0.0))),
            internet: Some(stats(600.0, 0.0)),
            ..Default::default()
        };
        let findings = vec![Finding::new("ping", "high", Severity::Warn, "")];
        assert_eq!(judge(&findings, &m, &Settings::default()).segment, Segment::Internet);
    }

    #[test]
    fn the_split_never_adds_up_to_more_than_was_measured() {
        // Jitter made the middle hop answer faster than the router. The three
        // shares are a partition of the 40 ms round trip, not three readings
        // subtracted from each other, so they have to total 40.
        let m = Measurements {
            gateway: Some(stats(30.0, 0.0)),
            edge: Some((Ipv4Addr::new(100, 64, 0, 1), stats(25.0, 0.0))),
            internet: Some(stats(40.0, 0.0)),
            ..Default::default()
        };
        let (lan, isp, far) = m.shares().unwrap();
        assert!((lan + isp + far - 40.0).abs() < 1e-9, "got {lan} + {isp} + {far}");

        // And the other direction: a router slower than the far end.
        let odd = Measurements {
            gateway: Some(stats(50.0, 0.0)),
            internet: Some(stats(20.0, 0.0)),
            ..Default::default()
        };
        let (lan, isp, far) = odd.shares().unwrap();
        assert!((lan + isp + far - 20.0).abs() < 1e-9, "got {lan} + {isp} + {far}");
        assert!(far >= 0.0, "no segment may contribute negative milliseconds");
    }

    #[test]
    fn the_users_own_loss_threshold_is_what_the_verdict_uses() {
        let m = Measurements {
            gateway: Some(stats(3.0, 1.5)),
            internet: Some(stats(20.0, 1.5)),
            ..Default::default()
        };
        // Default tolerance is 2%: 1.5% is noise, not a verdict.
        assert_eq!(loss_origin(&m, Settings::default().loss_ok_pct), None);
        // A user who cares about every packet says so, and is listened to.
        assert_eq!(loss_origin(&m, 0.5), Some((Segment::Lan, 1.5)));
    }

    #[test]
    fn a_loss_verdict_always_arrives_with_something_to_do() {
        // The `loss` finding is filed under Internet because that is where it
        // was measured, but the origin is the LAN. Filtering actions by
        // segment alone left this verdict with no next step at all.
        let m = Measurements {
            gateway: Some(stats(3.0, 8.0)),
            internet: Some(stats(20.0, 9.0)),
            ..Default::default()
        };
        let findings = vec![
            Finding::new("loss", "packets lost", Severity::Critical, "").advise("check cable")
        ];
        let v = judge(&findings, &m, &Settings::default());
        assert_eq!(v.segment, Segment::Lan);
        assert!(!v.actions.is_empty(), "a verdict with no action is not a diagnosis");
    }

    #[test]
    fn loss_is_blamed_on_the_first_segment_that_shows_it() {
        let lan = Measurements {
            gateway: Some(stats(3.0, 8.0)),
            internet: Some(stats(20.0, 9.0)),
            ..Default::default()
        };
        assert_eq!(loss_origin(&lan, Settings::default().loss_ok_pct), Some((Segment::Lan, 8.0)));

        // A middle hop that rate-limits its own replies is not an outage, so
        // loss there only counts when the far end loses packets too.
        let quiet_hop = Measurements {
            gateway: Some(stats(3.0, 0.0)),
            edge: Some((Ipv4Addr::new(10, 0, 0, 1), stats(12.0, 30.0))),
            internet: Some(stats(20.0, 0.0)),
            ..Default::default()
        };
        assert_eq!(loss_origin(&quiet_hop, Settings::default().loss_ok_pct), None);
    }

    #[test]
    fn a_clean_idle_line_that_collapses_under_load_is_still_a_fault() {
        let m = Measurements {
            gateway: Some(stats(2.0, 0.0)),
            edge: Some((Ipv4Addr::new(10, 0, 0, 1), stats(9.0, 0.0))),
            internet: Some(stats(12.0, 0.0)),
            load: Some(BloatResult {
                idle_avg: Some(12.0),
                loaded_avg: Some(460.0),
                bump_ms: Some(448.0),
                grade: Some(Grade::F),
                ..Default::default()
            }),
            ..Default::default()
        };
        let v = judge(&[], &m, &Settings::default());
        assert_eq!(v.segment, Segment::Uplink);
        assert_eq!(v.confidence, Confidence::Certain);
    }

    /// A chain with nothing wrong anywhere along it.
    fn clean() -> Measurements {
        Measurements {
            gateway: Some(stats(2.0, 0.0)),
            edge: Some((Ipv4Addr::new(10, 0, 0, 1), stats(9.0, 0.0))),
            internet: Some(stats(12.0, 0.0)),
            ..Default::default()
        }
    }

    #[test]
    fn a_link_local_address_is_named_as_a_dhcp_failure() {
        // The gap this closes: 169.254/16 is not `is_private`, so a card that
        // never got a lease was described as "no connection" — wrong advice
        // for a working adapter with its lights on, and the most common
        // domestic failure there is.
        let net = NetState {
            adapter_name: "Ethernet".into(),
            medium: Medium::Ethernet,
            local_ip: Some(Ipv4Addr::new(169, 254, 12, 9)),
            up: true,
            ..Default::default()
        };
        let store = Store::open_in_memory().unwrap();
        let findings = check_medium(&net, &store, &Settings::default());

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[0].title, i18n::f_apipa());
        assert!(findings[0].detail.contains("169.254.12.9"), "{}", findings[0].detail);
        assert!(!findings[0].advice.is_empty(), "a DHCP failure has a fix worth naming");
    }

    #[test]
    fn an_ordinary_private_address_is_not_mistaken_for_one() {
        let net = NetState {
            adapter_name: "Ethernet".into(),
            medium: Medium::Ethernet,
            local_ip: Some(Ipv4Addr::new(192, 168, 1, 40)),
            up: true,
            ..Default::default()
        };
        let store = Store::open_in_memory().unwrap();
        let findings = check_medium(&net, &store, &Settings::default());
        assert_ne!(findings[0].title, i18n::f_apipa());
    }

    /// Runs the wire findings and the verdict, the way `scan` does.
    fn verdict_of(wire: &Wire) -> (Vec<Finding>, Verdict) {
        let net = NetState {
            adapter_name: "WiFi".into(),
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            ..Default::default()
        };
        let store = Store::open_in_memory().unwrap();
        let mut m = Measurements::default();
        let findings = report_wire(&net, &store, &Settings::default(), wire, &mut m);
        let v = judge(&findings, &m, &Settings::default());
        (findings, v)
    }

    fn silent(n: usize) -> Stats {
        store::summarise(n, &[])
    }

    #[test]
    fn a_router_that_ignores_pings_on_a_working_line_is_not_a_dead_lan() {
        // Many routers drop ICMP addressed to themselves. The internet
        // answered in 12 ms and TCP connected, and the verdict was "LAN,
        // clear", because a silent gateway outranked everything.
        let wire = Wire {
            gateway: Some(silent(10)),
            internet: Some(stats(12.0, 0.0)),
            tcp: Some(Ok(14.0)),
            dns: (Some(9.0), String::new()),
            ..Default::default()
        };
        let (findings, v) = verdict_of(&wire);
        assert_ne!(v.segment, Segment::Lan, "{findings:?}");
        assert!(
            !findings.iter().any(|f| f.severity == Severity::Critical),
            "a working line has no critical finding: {findings:?}"
        );
    }

    #[test]
    fn one_silent_anchor_is_not_a_dead_provider() {
        // 1.1.1.1 filtered by the network, 8.8.8.8 answering, TCP fine.
        let wire = Wire {
            gateway: Some(stats(2.0, 0.0)),
            internet: Some(silent(15)),
            internet_alt: Some(stats(14.0, 0.0)),
            tcp: Some(Ok(15.0)),
            dns: (Some(9.0), String::new()),
            ..Default::default()
        };
        let (findings, v) = verdict_of(&wire);
        assert!(!matches!(v.segment, Segment::Isp | Segment::Internet), "{v:?} {findings:?}");
        assert!(
            findings.iter().any(|f| f.key == "internet" && f.detail.contains("14")),
            "the anchor that answered is the one measured: {findings:?}"
        );

        // Both anchors silent to ICMP, but TCP gets through: the internet
        // works and something filters pings. Not a certain outage.
        let wire = Wire { internet_alt: Some(silent(15)), ..wire };
        let (_, v) = verdict_of(&wire);
        assert_ne!(v.confidence, Confidence::Certain, "{v:?}");

        // All three agree: that is an outage past the router.
        let wire = Wire { tcp: Some(Err("timed out".into())), ..wire };
        let (_, v) = verdict_of(&wire);
        assert_eq!((v.segment, v.confidence), (Segment::Isp, Confidence::Certain));
    }

    #[test]
    fn no_outages_is_only_good_news_when_the_day_was_watched() {
        // Five minutes after a first start the scan said "no outages in the
        // last 24 hours", graded Good, as if it had watched the whole day.
        let store = Store::open_in_memory().unwrap();
        let t = store::now();
        let rows: Vec<_> =
            (0..300).map(|i| (t - i as f64, "cloudflare".to_string(), Some(12.0), true)).collect();
        store.add_samples(&rows).unwrap();

        let f = check_history(&NetState::default(), &store, &Settings::default());
        assert_eq!(f.len(), 1);
        assert_ne!(f[0].severity, Severity::Good, "{f:?}");
        assert!(f[0].title.contains("5 min") || f[0].detail.contains("5 min"), "{f:?}");

        // A whole watched day is good news.
        let store = Store::open_in_memory().unwrap();
        let day: Vec<_> = (0..(24 * 3600 / 30))
            .map(|i| (t - (i * 30) as f64, "cloudflare".to_string(), Some(12.0), true))
            .collect();
        store.add_samples(&day).unwrap();
        let f = check_history(&NetState::default(), &store, &Settings::default());
        assert_eq!(f[0].severity, Severity::Good, "{f:?}");
    }

    #[test]
    fn only_a_public_address_speaks_for_the_internet() {
        for local in [
            [192, 168, 1, 5],
            [10, 0, 0, 1],
            [172, 16, 0, 1],
            [169, 254, 1, 1],
            [100, 64, 0, 1],
            [100, 127, 255, 254],
            [127, 0, 0, 1],
        ] {
            assert!(!is_public(Ipv4Addr::from(local)), "{local:?}");
        }
        for public in [[1, 1, 1, 1], [8, 8, 8, 8], [100, 63, 255, 255], [100, 128, 0, 1]] {
            assert!(is_public(Ipv4Addr::from(public)), "{public:?}");
        }
    }

    #[test]
    fn the_apipa_range_is_exactly_169_254() {
        assert!(is_apipa(Ipv4Addr::new(169, 254, 0, 1)));
        assert!(is_apipa(Ipv4Addr::new(169, 254, 255, 254)));
        assert!(!is_apipa(Ipv4Addr::new(169, 253, 0, 1)));
        assert!(!is_apipa(Ipv4Addr::new(169, 255, 0, 1)));
        assert!(!is_apipa(Ipv4Addr::new(192, 168, 0, 1)));
    }

    #[test]
    fn a_scan_that_could_not_ping_names_no_segment() {
        // Found by reading: `ping_series` turned a handle it could not open
        // into ten lost packets, and the verdict was "LAN, clear" while the
        // TCP probe to the same anchor connected in 14 ms.
        let wire = Wire {
            tcp: Some(Ok(14.0)),
            dns: (Some(9.0), String::new()),
            blind: Some("access denied".into()),
            ..Default::default()
        };
        let net = NetState {
            adapter_name: "WiFi".into(),
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            ..Default::default()
        };
        let store = Store::open_in_memory().unwrap();
        let mut m = Measurements::default();
        let findings = report_wire(&net, &store, &Settings::default(), &wire, &mut m);

        assert!(!findings.iter().any(|f| f.key == "gateway_silent"), "the router was never asked");
        assert!(!findings.iter().any(|f| f.key == "internet_silent"));
        let v = judge(&findings, &m, &Settings::default());
        assert_eq!(v.segment, Segment::Unmeasured);
        assert_ne!(v.confidence, Confidence::Certain);
        assert!(!v.actions.is_empty(), "the user is told what to check");
    }

    #[test]
    fn a_healthy_chain_names_no_segment() {
        assert_eq!(judge(&[], &clean(), &Settings::default()).segment, Segment::Healthy);
    }

    #[test]
    fn a_working_wifi_link_is_not_read_as_a_missing_one() {
        // `check_medium` reports the medium whichever way it comes out, so a
        // rule keyed on the subject alone announced "connected over Wi-Fi" as
        // a dead segment. Severity is what separates the two.
        let findings = vec![Finding::new("medium", "Connected over Wi-Fi", Severity::Info, "")
            .advise("a cable is steadier")];
        assert_eq!(judge(&findings, &clean(), &Settings::default()).segment, Segment::Healthy);
    }

    #[test]
    fn a_clean_reading_does_not_overrule_last_nights_outages() {
        let findings = vec![
            // Recorded, but its scope was never established: unusable for blame.
            Finding::new("hist_other", "degraded quality", Severity::Critical, ""),
            Finding::new("hist_isp", "WAN drops", Severity::Critical, "").advise("report it"),
        ];
        let v = judge(&findings, &clean(), &Settings::default());
        assert_eq!(v.segment, Segment::Isp);
        // Thirty seconds of clean measurements cannot confirm last night.
        assert_eq!(v.confidence, Confidence::Possible);
        assert!(!v.actions.is_empty());
    }

    #[test]
    fn the_plan_never_runs_past_three_items() {
        let findings: Vec<Finding> = (0..6)
            .map(|i| {
                Finding::new("signal", format!("w{i}"), Severity::Warn, "").advise("do something")
            })
            .collect();
        assert!(actions_for(&findings, Segment::Lan).len() <= 3);
    }
}
