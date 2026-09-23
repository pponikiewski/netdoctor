//! Background monitor.
//!
//! Pings the router and several internet endpoints in parallel, once per
//! interval, and turns the *pattern* of failures into a verdict about where
//! the connection broke. That distinction is the whole point of the tool: a
//! dropout looks identical from inside Windows whether the Wi-Fi card fell
//! asleep or the ISP went down, and the two are fixed in completely different
//! ways.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::i18n;
use crate::probe::icmp::{PingError, PingResult, Pinger};
use crate::probe::netstate::{self, Medium, NetState};
use crate::probe::path::{self, PathReading};
use crate::settings::{Scope, Settings, DNS_TEST_HOST};
use crate::store::{self, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Ok,
    Degraded,
    DnsFail,
    IspDown,
    LanDown,
    AdapterDown,
}

impl Status {
    pub fn headline(&self) -> &'static str {
        match self {
            Status::Ok => i18n::mon_ok(),
            Status::Degraded => i18n::mon_degraded(),
            Status::DnsFail => i18n::mon_dns_fail(),
            Status::IspDown => i18n::mon_isp_down(),
            Status::LanDown => i18n::mon_lan_down(),
            Status::AdapterDown => i18n::mon_adapter_down(),
        }
    }

    pub fn scope(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Degraded => "internet",
            Status::DnsFail => "dns",
            Status::IspDown => "isp",
            Status::LanDown => "lan",
            Status::AdapterDown => "adapter",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Degraded => "degraded",
            Status::DnsFail => "dns_fail",
            Status::IspDown => "isp_down",
            Status::LanDown => "lan_down",
            Status::AdapterDown => "adapter_down",
        }
    }
}

/// Severity order among scopes, used only to break ties. A dropped adapter is
/// a more specific and more actionable claim than "quality was poor", so when
/// two scopes occur equally often the stronger claim is the one worth naming.
pub fn scope_rank(scope: &str) -> u8 {
    match scope {
        "adapter" => 4,
        "lan" => 3,
        "isp" => 2,
        "dns" => 1,
        _ => 0,
    }
}

/// The scope to name when summarising several outages: the most frequent, and
/// on a tie the more severe.
///
/// The tie-break is not cosmetic. Counting into a `HashMap` and taking the
/// maximum leaves the winner of a tie down to iteration order, which is seeded
/// per map — so a summary rebuilt every frame picked a different scope every
/// frame and the headline visibly flickered between two verdicts. Ordering has
/// to be total, not merely "usually stable".
pub fn dominant_scope<'a, I>(scopes: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for s in scopes {
        *counts.entry(s).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(scope, n)| (*n, scope_rank(scope), *scope))
        .map(|(scope, _)| scope)
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub ok: bool,
    pub rtt_ms: Option<f64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub ts: f64,
    pub status: Status,
    pub note: String,
    pub results: HashMap<String, Sample>,
    pub net: NetState,
    pub dns_ms: Option<f64>,
    pub dns_error: String,
    pub roamed: bool,
    /// When the current unbroken stretch of watching began, or `None` before
    /// the first sweep of this run has placed itself in the history. See
    /// [`Store::observing_since`].
    pub observed_from: Option<f64>,
    /// Why this sweep measured nothing, when it did not: no ICMP handle could
    /// be opened, so no probe was sent. `status` is then not a verdict, and
    /// whatever shows one has to show this instead.
    pub blind: Option<String>,
    /// Why this sweep's samples did not reach the database, when they did
    /// not. Loss and jitter are read back from there, so a healthy verdict
    /// judged without them is not one: see [`Seen::Unrecorded`].
    pub unrecorded: Option<String>,
    /// The loss and jitter the verdict was judged on. See [`LineQuality`].
    pub quality: LineQuality,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            ts: 0.0,
            status: Status::Ok,
            note: String::new(),
            results: HashMap::new(),
            net: NetState::default(),
            dns_ms: None,
            dns_error: String::new(),
            roamed: false,
            observed_from: None,
            blind: None,
            unrecorded: None,
            quality: LineQuality::default(),
        }
    }
}

/// What the last snapshot can honestly be shown as. The window's header and
/// the tray both read it, so neither can call a line healthy that the other
/// shows as unmeasured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seen<'a> {
    /// The user or a job paused sampling.
    Paused,
    /// No sweep yet.
    Waiting,
    /// The last sweep sent nothing: see [`Snapshot::blind`].
    Blind,
    /// The line answered, but its samples could not be saved: see
    /// [`Snapshot::unrecorded`]. Only a healthy verdict turns into this. A
    /// failure is read off the pings themselves and stands without the
    /// database.
    Unrecorded,
    /// Sampling is on, but the newest sweep is this many seconds old: the
    /// sweep thread has stopped or is stuck. Its verdict is history.
    Stale(f64),
    Verdict(Status, &'a str),
}

/// How many intervals a reading may age before it stops being the latest
/// word. A sweep can run late by its own timeout and an adapter read, so one
/// or two missed beats is ordinary; five is not.
const STALE_AFTER_INTERVALS: u32 = 5;
/// And never less than this, so a short interval does not grey the icon on a
/// single slow sweep.
const STALE_FLOOR: Duration = Duration::from_secs(10);

impl<'a> Seen<'a> {
    /// `sampling` is false while the user or a job holds the monitor. `now`
    /// is the wall clock the snapshot's `ts` was taken on, and `interval` the
    /// sweep interval it should be refreshed at.
    pub fn of(last: &'a Snapshot, sampling: bool, now: f64, interval: Duration) -> Self {
        let age = now - last.ts;
        let limit = (interval * STALE_AFTER_INTERVALS).max(STALE_FLOOR).as_secs_f64();
        if !sampling {
            Seen::Paused
        } else if last.ts <= 0.0 {
            Seen::Waiting
        } else if age > limit {
            Seen::Stale(age)
        } else if last.blind.is_some() {
            Seen::Blind
        } else if last.unrecorded.is_some() && last.status == Status::Ok {
            Seen::Unrecorded
        } else {
            Seen::Verdict(last.status, &last.note)
        }
    }
}

/// Ring buffer of (timestamp, rtt) per target, for the live chart.
pub type Series = Vec<(f64, Option<f64>)>;

// ---------------------------------------------------------------------------
// Telling a real latency event apart from one target's noise
// ---------------------------------------------------------------------------

/// A latency spike is only evidence about the connection if it hits more than
/// one target at once.
///
/// Every target is reached over the same first hops — the same Wi-Fi link, the
/// same router, the same uplink. A delay introduced anywhere on that shared
/// stretch has to show up on all of them in the same sweep. So a sweep where
/// one target jumps and the others are untouched cannot have been caused
/// there: what was measured is that single responder taking its time. Routers
/// and anycast nodes answer ICMP from the control plane at the lowest
/// priority, and traffic that merely passes through them never waits that
/// long.
///
/// On a real link the overwhelming majority of visible spikes are of the
/// single-target kind, and plotting them identically to the correlated ones
/// makes a healthy connection look ragged.
#[derive(Debug, Clone, Default)]
pub struct Spikes {
    /// Sweeps where at least two targets jumped together, with how many did.
    pub correlated: Vec<(f64, usize)>,
    /// Spikes that hit exactly one target. Noise from that responder.
    pub single: usize,
}

/// How far above its own normal a sample has to sit to count as a spike.
///
/// Relative, because a 20 ms jump means nothing on a 90 ms satellite link and
/// is an event on a 4 ms one; with a floor, because on a very fast link the
/// relative test alone fires on ordinary scheduling noise.
fn spike_threshold(median: f64) -> f64 {
    (median * 0.8).max(8.0)
}

fn median_of(series: &Series) -> Option<f64> {
    let mut v: Vec<f64> = series.iter().filter_map(|(_, r)| *r).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(v[v.len() / 2])
}

/// Group the spikes across every plotted series by the sweep they happened in.
///
/// Samples from one sweep carry slightly different timestamps — they are taken
/// one after another — so they are bucketed by whole second, which is the
/// probe interval and comfortably wider than a sweep.
pub fn find_spikes(series: &[Series]) -> Spikes {
    let mut hits: BTreeMap<i64, (f64, usize)> = BTreeMap::new();

    for s in series {
        let Some(median) = median_of(s) else {
            continue;
        };
        let threshold = spike_threshold(median);
        for (ts, rtt) in s {
            if let Some(v) = rtt {
                if v - median > threshold {
                    let slot = hits.entry(ts.round() as i64).or_insert((*ts, 0));
                    slot.1 += 1;
                }
            }
        }
    }

    let mut out = Spikes::default();
    for (_, (ts, n)) in hits {
        if n >= 2 {
            out.correlated.push((ts, n));
        } else {
            out.single += 1;
        }
    }
    out
}

/// How many sweeps of lead-up are kept for the next outage. At the default one
/// sweep per second this is three minutes, which is long enough to show a
/// signal sliding away or a roam to another access point, and short enough
/// that the JSON stays a few kilobytes.
const LEAD_SWEEPS: usize = 180;

/// One sweep, reduced to the fields that explain a later failure. Kept
/// deliberately narrow: this is written into every outage row, so it has to
/// stay cheap to store and cheap to read back.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LeadSample {
    pub ts: f64,
    pub status: String,
    pub up: bool,
    pub bssid: String,
    pub signal_pct: Option<u32>,
    pub rssi_dbm: Option<i32>,
    pub channel: Option<u32>,
    pub rx_mbps: Option<u32>,
    pub gateway_ms: Option<f64>,
    pub internet_ms: Option<f64>,
    pub internet_ok: bool,
}

/// Best available internet round-trip for a sweep: the fastest target that is
/// not the router, so one slow endpoint does not look like an outage.
fn internet_rtt(results: &HashMap<String, Sample>) -> (Option<f64>, bool) {
    let mut best: Option<f64> = None;
    let mut any_ok = false;
    for (key, s) in results {
        if key == "gateway" {
            continue;
        }
        if s.ok {
            any_ok = true;
            if let Some(rtt) = s.rtt_ms {
                best = Some(best.map_or(rtt, |b: f64| b.min(rtt)));
            }
        }
    }
    (best, any_ok)
}

fn lead_sample(snap: &Snapshot) -> LeadSample {
    let (internet_ms, internet_ok) = internet_rtt(&snap.results);
    LeadSample {
        ts: snap.ts,
        status: snap.status.key().to_string(),
        up: snap.net.up,
        bssid: snap.net.bssid.clone(),
        signal_pct: snap.net.signal_pct,
        rssi_dbm: snap.net.rssi_dbm,
        channel: snap.net.channel,
        rx_mbps: snap.net.rx_mbps,
        gateway_ms: snap.results.get("gateway").and_then(|s| s.rtt_ms),
        internet_ms,
        internet_ok,
    }
}

/// The evidence written into an outage row. `lead_up` is the part that was
/// missing: a single snapshot says what the connection looked like once it had
/// already broken, which is rarely the thing that broke it.
///
/// `path` is the per-hop table as the path thread last left it. It goes in
/// because the hop table on screen is the current one: a report written days
/// later has to show the path as it was when the outage happened, which is
/// the part of the evidence a provider cannot wave away.
fn context_json(
    snap: &Snapshot,
    lead: &VecDeque<LeadSample>,
    roamed_recently: bool,
    path: Option<&PathReading>,
) -> serde_json::Value {
    let net = &snap.net;
    serde_json::json!({
        "adapter": net.adapter_name,
        "adapter_desc": net.adapter_desc,
        "medium": net.medium.label(),
        "up": net.up,
        "ssid": net.ssid,
        "bssid": net.bssid,
        "security": net.security,
        "signal_pct": net.signal_pct,
        "rssi_dbm": net.rssi_dbm,
        "channel": net.channel,
        "band": net.band(),
        "phy": net.phy,
        "rx_mbps": net.rx_mbps,
        "tx_mbps": net.tx_mbps,
        "link_speed_mbps": net.link_speed_mbps,
        "local_ip": net.local_ip.map(|i| i.to_string()),
        "gateway": net.gateway.map(|g| g.to_string()),
        "dns": net.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        "dns_is_router_only": net.dns_is_router_only(),
        "dns_is_own_resolver": net.dns_is_own_resolver(),
        "dns_ms": snap.dns_ms,
        "dns_error": snap.dns_error,
        "roamed": roamed_recently,
        "lead_up": lead.iter().collect::<Vec<_>>(),
        "path": path.filter(|p| !p.hops.is_empty()),
    })
}

/// Takes a lock, ignoring poisoning.
///
/// A poisoned mutex means some other thread panicked while holding it, not
/// that the value is unusable: everything behind these locks is a snapshot
/// the next sweep overwrites anyway. `unwrap` here would turn one panic into
/// a panic in every thread that touches the same lock afterwards, and the
/// monitor is the part that has to keep running.
fn held<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct Shared {
    pub last: Mutex<Snapshot>,
    /// The pause the user asked for with the Live tab's button.
    user_paused: AtomicBool,
    /// Pauses held by the app's own jobs: a load test, a deep scan, an air
    /// scan. Counted rather than flagged, because two of them can overlap and
    /// the first to finish used to unpause the monitor under the second, and
    /// under a pause the user had set themselves.
    holds: AtomicU32,
    pub settings: Mutex<Settings>,
    /// The per-hop picture, maintained by its own thread — see
    /// [`crate::probe::path`] and `run_path`.
    pub path: Mutex<PathReading>,
    /// The current default gateway, published for the path thread. It is read
    /// on the sweep cadence and changes when the machine moves between
    /// networks, which is exactly when the path has to be walked again.
    pub gateway: Mutex<Option<Ipv4Addr>>,
    /// The latest DNS test, kept by `run_dns`.
    dns: Mutex<DnsReading>,
    /// The current adapter's resolvers, published for `run_dns` beside the
    /// gateway.
    dns_servers: Mutex<Vec<Ipv4Addr>>,
    /// The user's extra targets that are hostnames, resolved by `run_hosts`.
    hosts: Mutex<HashMap<String, Ipv4Addr>>,
    /// Since when no public address has answered a ping, published by the
    /// sweep. `run_tcp` only works while this is set.
    icmp_dark_since: Mutex<Option<Instant>>,
    /// The latest connection attempt `run_tcp` made.
    tcp: Mutex<Option<TcpAttempt>>,
    /// A game from [`crate::game::GAMES`] is running. Kept by the game
    /// watcher; the sweep reads it to tighten its cadence.
    pub gaming: AtomicBool,
    /// Which game, for the tray menu.
    pub game: Mutex<Option<&'static str>>,
    /// The outcome of the last game-mode prepare or restore, waiting for the
    /// tray to show it.
    pub game_msg: Mutex<Option<String>>,
}

/// How long a DNS test may run before the sweep calls it a failure. A healthy
/// resolver answers in milliseconds; the Windows client's own retries run to
/// about twelve seconds, and the sweep cannot wait that out.
const DNS_STALL: Duration = Duration::from_secs(5);
/// How often the DNS test runs. The same ten seconds the sweep used to count.
const DNS_EVERY: Duration = Duration::from_secs(10);
/// How soon a failed test is repeated. Short, because until it is repeated
/// the failure stands, and the outage it opens is only as precise as this.
const DNS_RETRY: Duration = Duration::from_secs(2);
/// Failed tests in a row before DNS counts as down. One lost UDP reply is
/// not an outage.
const DNS_FAILS_TO_REPORT: u32 = 2;
/// How often the user's hostnames are looked up again when they all resolved.
const HOSTS_EVERY: Duration = Duration::from_secs(300);
/// And when one of them did not.
const HOSTS_RETRY: Duration = Duration::from_secs(30);

/// How often `run_tcp` tries while pings are unanswered.
const TCP_EVERY: Duration = Duration::from_secs(2);
/// How old a successful connection may be and still vouch for the internet.
/// A little over two attempts: one slow handshake must not flip the verdict,
/// and an outage on a network that filters pings must still show within
/// seconds.
const TCP_FRESH: Duration = Duration::from_secs(6);

/// One connection attempt to a public anchor on port 443.
#[derive(Debug, Clone, Copy)]
struct TcpAttempt {
    /// When the attempt started.
    at: Instant,
    ok: bool,
}

impl TcpAttempt {
    /// Whether this attempt shows the internet reachable although no public
    /// address answers pings: it succeeded, it began after the pings stopped
    /// answering, and it is recent. A success from before the silence says
    /// nothing about the silence.
    fn vouches(&self, dark_since: Option<Instant>, now: Instant) -> bool {
        let Some(dark) = dark_since else { return false };
        self.ok && self.at >= dark && now.saturating_duration_since(self.at) <= TCP_FRESH
    }
}

/// The DNS tests, as `run_dns` last left them.
///
/// A single failed test used to be the answer until the next one, ten seconds
/// later: ten bad sweeps, a recorded outage and a notification out of one
/// lost reply, and every DNS outage in the history rounded up to ten seconds.
/// A failure now has to repeat, and is repeated after [`DNS_RETRY`].
#[derive(Debug, Clone, Default)]
struct DnsReading {
    /// The last successful test's time.
    ms: Option<f64>,
    /// The last failed test's reason, while tests are failing.
    error: String,
    /// Failed tests in a row.
    fails: u32,
    /// Set while a lookup is running, so one that hangs can be reported as a
    /// failure instead of leaving the last good answer on screen.
    started: Option<Instant>,
}

impl DnsReading {
    /// A test finished: `error` is empty when it succeeded.
    fn finish(&mut self, ms: Option<f64>, error: String) {
        self.started = None;
        if error.is_empty() {
            self.ms = ms;
            self.error.clear();
            self.fails = 0;
        } else {
            self.error = error;
            self.fails += 1;
        }
    }

    /// How long until the next test.
    fn next_check(&self) -> Duration {
        if self.fails > 0 {
            DNS_RETRY
        } else {
            DNS_EVERY
        }
    }

    /// What the sweep reports. A failure once it has repeated, or once the
    /// test running now has been silent for longer than a working resolver
    /// takes: that is several seconds of failing already, not one reading.
    fn current(&self, now: Instant) -> (Option<f64>, String) {
        match self.started {
            Some(t) if now.saturating_duration_since(t) > DNS_STALL => {
                (None, i18n::dns_no_answer(DNS_STALL.as_secs()))
            }
            _ if self.fails >= DNS_FAILS_TO_REPORT => (None, self.error.clone()),
            _ => (self.ms, String::new()),
        }
    }
}

pub struct Monitor {
    pub shared: Arc<Shared>,
    pub rx: Receiver<Snapshot>,
    /// Outages worth a Windows notification. Read by [`crate::tray`].
    pub notices: Receiver<Notice>,
    stop: Arc<AtomicBool>,
    /// The sweep, joined on drop: it closes an open outage on the way out.
    /// The other threads are not, see `start`.
    handle: Option<thread::JoinHandle<()>>,
}

impl Monitor {
    pub fn start(store: Arc<Store>, settings: Settings) -> Monitor {
        let store_for_router = Arc::clone(&store);
        let shared = Arc::new(Shared::new(settings));
        let stop = Arc::new(AtomicBool::new(false));
        // Bounded so a stalled UI cannot grow the queue without limit; the
        // newest snapshot matters, older ones can be dropped.
        let (tx, rx) = bounded::<Snapshot>(64);
        let (notice_tx, notices) = bounded::<Notice>(16);

        let handle = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("netdoctor-monitor".into())
                .spawn(move || run_loop(shared, store, tx, notice_tx, stop))
                // ponytail: a thread spawn failing means the OS is out of
                // resources and nothing this app does next will work. Make
                // `new` fallible if it ever needs to degrade instead of die.
                .expect("spawn monitor thread")
        };

        // Four more threads, none of them joined on drop. They touch nothing
        // but `Shared`, so there is nothing for them to finish, and waiting
        // for them could take long enough to matter: the process has to be
        // gone before an update's new copy can start (see
        // `single::acquire_within`).
        //
        // The path walk is a dozen sequential probes, and a hop that never
        // answers costs a full timeout, so in the sweep it would stall the one
        // measurement that has to keep its cadence; at shutdown it could be
        // most of a traceroute from noticing the stop.
        //
        // Name lookups block for as long as the resolver makes them, which is
        // longest exactly when DNS is what broke. So they run here, and the
        // sweep only ever reads their last result.
        for (name, job) in [
            ("netdoctor-path", run_path as fn(Arc<Shared>, Arc<AtomicBool>)),
            ("netdoctor-dns", run_dns),
            ("netdoctor-hosts", run_hosts),
            ("netdoctor-tcp", run_tcp),
        ] {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name(name.into())
                .spawn(move || job(shared, stop))
                // ponytail: as above.
                .expect("spawn worker thread");
        }

        {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            let store = Arc::clone(&store_for_router);
            thread::Builder::new()
                .name("netdoctor-router".into())
                .spawn(move || run_router(shared, store, stop))
                // ponytail: as above.
                .expect("spawn router thread");
        }

        Monitor { shared, rx, notices, stop, handle: Some(handle) }
    }

    /// The user's own pause. Independent of any job's hold.
    pub fn set_paused(&self, paused: bool) {
        self.shared.user_paused.store(paused, Ordering::Relaxed);
    }

    /// Whether the user paused it. A job's hold does not show here, so the
    /// Live tab's button keeps meaning what the user last pressed.
    pub fn is_paused(&self) -> bool {
        self.shared.user_paused.load(Ordering::Relaxed)
    }

    /// Stops sampling for the length of a job. Every `hold` needs exactly one
    /// `release`.
    pub fn hold(&self) {
        self.shared.hold();
    }

    pub fn release(&self) {
        self.shared.release();
    }

    pub fn update_settings(&self, s: Settings) {
        *held(&self.shared.settings) = s;
    }

    /// The per-hop table and its verdict, as the path thread last left them.
    pub fn path(&self) -> PathReading {
        held(&self.shared.path).clone()
    }
}

impl Shared {
    pub(crate) fn new(settings: Settings) -> Shared {
        Shared {
            last: Mutex::new(Snapshot::default()),
            user_paused: AtomicBool::new(false),
            holds: AtomicU32::new(0),
            settings: Mutex::new(settings),
            path: Mutex::new(PathReading::default()),
            gateway: Mutex::new(None),
            dns: Mutex::new(DnsReading::default()),
            dns_servers: Mutex::new(Vec::new()),
            hosts: Mutex::new(HashMap::new()),
            icmp_dark_since: Mutex::new(None),
            tcp: Mutex::new(None),
            gaming: AtomicBool::new(false),
            game: Mutex::new(None),
            game_msg: Mutex::new(None),
        }
    }

    /// Whether the monitor is sampling right now: neither the user nor a job
    /// has it paused.
    pub fn paused(&self) -> bool {
        self.user_paused.load(Ordering::Relaxed) || self.holds.load(Ordering::Relaxed) > 0
    }

    fn hold(&self) {
        self.holds.fetch_add(1, Ordering::Relaxed);
    }

    fn release(&self) {
        // Saturating: a stray release must not wrap round to a permanent hold.
        let _ = self.holds.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1));
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Sleeps in slices so a stop request is noticed promptly. `false` once
/// stopped.
fn nap(stop: &AtomicBool, total: Duration) -> bool {
    let mut left = total;
    while left > Duration::ZERO {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let step = left.min(Duration::from_millis(200));
        thread::sleep(step);
        left = left.saturating_sub(step);
    }
    !stop.load(Ordering::Relaxed)
}

/// Times a lookup of the test host every [`DNS_EVERY`].
fn run_dns(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let mut wait = DNS_EVERY;
        if !shared.paused() {
            let servers = held(&shared.dns_servers).clone();
            held(&shared.dns).started = Some(Instant::now());
            let (ms, error) = netstate::resolvers_answer(&servers, DNS_TEST_HOST);
            let mut reading = held(&shared.dns);
            reading.finish(ms, error);
            wait = reading.next_check();
        }
        if !nap(&stop, wait) {
            return;
        }
    }
}

/// Tries a TCP connection to the public anchors while no public address
/// answers pings, and does nothing otherwise.
///
/// Pings are all the sweep sends, and a network that filters them outbound
/// (hotels, some offices and mobile operators) looked like the provider being
/// down for as long as the app ran: a recorded outage, a notification. A
/// handshake on 443 is traffic nobody filters without breaking the web, so
/// when it gets through the internet is there, whatever the pings say. It is
/// only tried while pings are unanswered: an idle line costs nothing.
fn run_tcp(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let dark = held(&shared.icmp_dark_since).is_some();
        let mut wait = Duration::from_secs(1);
        if dark && !shared.paused() {
            let settings = held(&shared.settings).clone();
            let at = Instant::now();
            let ok = [crate::diagnose::ANCHOR, crate::diagnose::ANCHOR_ALT]
                .iter()
                .any(|a| crate::diagnose::tcp_probe(*a, &settings).is_ok());
            *held(&shared.tcp) = Some(TcpAttempt { at, ok });
            wait = TCP_EVERY;
        }
        if !nap(&stop, wait) {
            return;
        }
    }
}

/// Keeps the user's hostname targets resolved, off the sweep.
///
/// A failed lookup keeps the address it had: a host the user asked to watch
/// should not drop out of the sweep for the length of a DNS outage, which is
/// what happened when the sweep resolved them itself.
fn run_hosts(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let mut last_list: Option<Vec<String>> = None;
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let list = held(&shared.settings).extra_targets.clone();
        if last_list.as_ref() != Some(&list) || Instant::now() >= next {
            let old = held(&shared.hosts).clone();
            let mut fresh = HashMap::new();
            let mut missing = false;
            for raw in &list {
                let text = raw.trim();
                if text.is_empty() || text.parse::<Ipv4Addr>().is_ok() {
                    continue;
                }
                match crate::settings::resolve_target(text).or_else(|| old.get(text).copied()) {
                    Some(addr) => {
                        fresh.insert(text.to_string(), addr);
                    }
                    None => missing = true,
                }
            }
            *held(&shared.hosts) = fresh;
            last_list = Some(list);
            next = Instant::now() + if missing { HOSTS_RETRY } else { HOSTS_EVERY };
        }
        if !nap(&stop, Duration::from_secs(1)) {
            return;
        }
    }
}

/// A target with its host resolved against the current network state.
struct Resolved {
    key: String,
    host: Ipv4Addr,
    scope: Scope,
}

/// `hosts` is `run_hosts`'s cache. Nothing here touches DNS.
fn resolve_targets(
    settings: &Settings,
    net: &NetState,
    hosts: &HashMap<String, Ipv4Addr>,
) -> Vec<Resolved> {
    let mut out = Vec::new();
    let cached = |text: &str| text.parse().ok().or_else(|| hosts.get(text).copied());
    for t in settings.targets_with(cached) {
        let host = match t.key.as_str() {
            "gateway" => net.gateway,
            "dns_isp" => {
                // Probing the router twice tells us nothing new, so when the
                // only resolver *is* the router we skip this target.
                net.dns_servers.iter().find(|d| Some(**d) != net.gateway).copied()
            }
            _ => t.host,
        };
        if let Some(host) = host {
            out.push(Resolved { key: t.key, host, scope: t.scope });
        }
    }
    out
}

/// Where the path is walked to. The same anchor the diagnostic scan uses, so
/// the two never disagree about which route was measured.
const PATH_ANCHOR: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);

/// How often the path is walked again. Routes change, but not every minute,
/// and a walk costs a dozen probes.
const PATH_REFRESH_S: f64 = 300.0;

/// Seconds between per-hop probes. Slower than the sweep on purpose: this
/// measurement is about where a fault sits, not about catching the instant it
/// starts, and a dozen extra pings a second is traffic a router may start
/// rate-limiting — which would show up as loss the path module then has to
/// explain away.
const HOP_INTERVAL_S: u64 = 5;

/// How often the router is asked about its WAN connection. Often enough that
/// an outage of a minute has a reading inside it, rarely enough to be no load
/// on the router at all.
const ROUTER_INTERVAL: Duration = Duration::from_secs(30);

/// How long to wait before looking for the router's UPnP service again after
/// it was not found: most routers that do not answer never will.
const ROUTER_REDISCOVER: Duration = Duration::from_secs(600);

/// Records what the router says about its own internet connection, for the
/// outage analysis to read afterwards (see [`crate::cause`]). Nothing here
/// feeds the live verdict: that stays on what this machine measured.
fn run_router(shared: Arc<Shared>, store: Arc<Store>, stop: Arc<AtomicBool>) {
    use crate::probe::igd;
    let mut found: Option<(Ipv4Addr, igd::Igd)> = None;
    let mut last_look: Option<(Ipv4Addr, Instant)> = None;

    while !stop.load(Ordering::Relaxed) {
        let gateway = *held(&shared.gateway);
        if !shared.paused() {
            if let Some(gw) = gateway {
                if found.as_ref().is_some_and(|(g, _)| *g != gw) {
                    found = None;
                }
                let due =
                    last_look.is_none_or(|(g, at)| g != gw || at.elapsed() >= ROUTER_REDISCOVER);
                if found.is_none() && due {
                    last_look = Some((gw, Instant::now()));
                    found = igd::discover(gw).map(|i| (gw, i));
                }
                if let Some((_, service)) = &found {
                    match igd::read(service) {
                        Some(reading) => {
                            let _ = store.add_router_reading(&reading);
                        }
                        // The control URL can move when the router restarts;
                        // look again on the next round rather than in ten
                        // minutes.
                        None => {
                            found = None;
                            last_look = None;
                        }
                    }
                }
            }
        }
        let until = Instant::now() + ROUTER_INTERVAL;
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Keeps the per-hop picture current, independently of the sweep.
///
/// Walking the path and probing its hops are both slow and both bursty. Run
/// inside the sweep they would drag the interval around; run here they cost
/// the sweep nothing and the hop figures simply lag a few seconds behind,
/// which is what they measure anyway.
fn run_path(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let Ok(pinger) = Pinger::new() else {
        return;
    };
    let mut tracker = path::Tracker::default();
    let mut last_gateway: Option<Ipv4Addr> = None;

    while !stop.load(Ordering::Relaxed) {
        if shared.paused() {
            thread::sleep(Duration::from_millis(200));
            continue;
        }

        let timeout = held(&shared.settings).ping_timeout_ms;
        let gateway = *held(&shared.gateway);
        let now = store::now();

        // A new gateway means a different network, and every hop behind it
        // belongs to the old one. Keeping the old path would report a route
        // this machine is no longer on.
        let moved = gateway != last_gateway;
        if moved || tracker.path_age(now) > PATH_REFRESH_S {
            last_gateway = gateway;
            let walked = path::discover(PATH_ANCHOR, gateway, timeout);
            if moved || !walked.hops.is_empty() {
                tracker.set_path(walked);
            }
        }

        if !tracker.is_empty() {
            tracker.probe(&pinger, timeout);
            *held(&shared.path) = tracker.reading();
        } else {
            *held(&shared.path) = PathReading::default();
        }

        // Sleep in slices so stopping the app does not wait out the interval.
        for _ in 0..HOP_INTERVAL_S * 5 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

/// One ICMP handle per target, pinged at the same time.
///
/// Sequential probing cost one timeout per silent target, so a sweep that
/// normally takes 40 ms took 3.6 s the moment everything stopped answering:
/// the sampling cadence quietly dropped from the configured second to nearly
/// four, exactly when the samples matter most, and the chart then read its own
/// slow sweeps as "the app was not recording".
///
/// Each thread takes its own `Pinger` by `&mut`, which is what keeps this
/// sound: `Pinger` is `Send` but deliberately not `Sync`, so a handle is moved
/// to one thread rather than shared between them, and `IcmpSendEcho` never
/// sees two calls on the same handle.
#[derive(Default)]
struct PingerPool {
    handles: Vec<Pinger>,
}

impl PingerPool {
    /// Pings every target at once and returns the results in the same order.
    ///
    /// A handle that cannot be opened leaves its target to be pinged on a
    /// neighbour's handle afterwards, sequentially: fewer handles is slower,
    /// not wrong.
    ///
    /// `Err` when not one handle could be opened. Nothing was sent, so there
    /// is no result to report for any target: an empty list here used to be
    /// read as "the router is gone" and recorded as an outage.
    fn sweep(
        &mut self,
        targets: &[Resolved],
        timeout_ms: u32,
    ) -> Result<Vec<PingResult>, PingError> {
        self.sweep_opening(targets, timeout_ms, Pinger::new)
    }

    /// `sweep`, with the way a handle is opened passed in, so a test can make
    /// it fail.
    fn sweep_opening(
        &mut self,
        targets: &[Resolved],
        timeout_ms: u32,
        mut open: impl FnMut() -> Result<Pinger, PingError>,
    ) -> Result<Vec<PingResult>, PingError> {
        let mut refused = None;
        while self.handles.len() < targets.len() {
            match open() {
                Ok(p) => self.handles.push(p),
                Err(e) => {
                    refused = Some(e);
                    break;
                }
            }
        }
        if self.handles.is_empty() {
            return match refused {
                Some(e) => Err(e),
                // No targets, so no handle was asked for.
                None => Ok(Vec::new()),
            };
        }

        let mut out: Vec<Option<PingResult>> = (0..targets.len()).map(|_| None).collect();
        // One chunk per available handle. With a handle each — the normal
        // case — every chunk is a single target and the whole sweep costs one
        // timeout rather than one per target.
        let per_handle = targets.len().div_ceil(self.handles.len());

        thread::scope(|s| {
            let mut workers = Vec::new();
            for (handle, chunk) in self.handles.iter_mut().zip(targets.chunks(per_handle)) {
                workers.push(s.spawn(move || {
                    chunk.iter().map(|t| handle.ping(t.host, timeout_ms)).collect::<Vec<_>>()
                }));
            }
            for (i, worker) in workers.into_iter().enumerate() {
                let Ok(results) = worker.join() else { continue };
                for (j, r) in results.into_iter().enumerate() {
                    out[i * per_handle + j] = Some(r);
                }
            }
        });

        Ok(out.into_iter().map(|r| r.unwrap_or_else(PingResult::timeout)).collect())
    }
}

/// Outage bookkeeping, kept apart from the probes and the database so the
/// rules about holes in the watching can be tested.
///
/// Two kinds of hole. One wider than [`store::OBSERVATION_GAP_S`] is a
/// machine that slept or a long pause: nothing is known about what happened
/// in it, so an open outage ends at the last sweep before it. A shorter one is
/// a load test or an air scan holding the monitor for a few seconds: an outage
/// that is still there on the far side is the same outage, and splitting it
/// in two would double-count it. The seconds nobody watched are recorded with
/// it, so its length is never read as fully observed.
#[derive(Debug, Default)]
struct Outages {
    open: bool,
    fail_streak: u32,
    /// Seconds inside the open outage that were bridged over, not watched.
    unwatched_s: f64,
    /// The last sweep before the current pause, while it lasts.
    paused_after: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Nothing,
    Open,
    /// Close the open outage, ending at `at`.
    Close {
        at: f64,
        unwatched_s: f64,
    },
}

impl Outages {
    /// The monitor is paused. `last_sweep` is the newest sweep it ran.
    fn pause(&mut self, last_sweep: Option<f64>) {
        if self.paused_after.is_none() {
            self.paused_after = last_sweep;
        }
    }

    /// A hole too wide to bridge ended at `last_seen`.
    fn hole(&mut self, last_seen: f64) -> Step {
        self.fail_streak = 0;
        self.paused_after = None;
        self.finish(last_seen)
    }

    /// One sweep at `ts`, good or bad.
    fn sweep(&mut self, ts: f64, bad: bool, open_after: u32) -> Step {
        let resumed_from = self.paused_after.take();
        self.fail_streak = if bad { self.fail_streak + 1 } else { 0 };

        if self.open {
            if bad {
                if let Some(from) = resumed_from {
                    self.unwatched_s += (ts - from).max(0.0);
                }
                return Step::Nothing;
            }
            // Recovered. After a pause, all that is known is that it was
            // still down at the last sweep before it.
            return self.finish(resumed_from.unwrap_or(ts));
        }
        if bad && self.fail_streak >= open_after {
            self.open = true;
            self.unwatched_s = 0.0;
            return Step::Open;
        }
        Step::Nothing
    }

    /// The database refused the outage it was asked to open.
    fn abandon(&mut self) {
        self.open = false;
    }

    fn finish(&mut self, at: f64) -> Step {
        if !std::mem::take(&mut self.open) {
            return Step::Nothing;
        }
        Step::Close { at, unwatched_s: std::mem::take(&mut self.unwatched_s) }
    }
}

/// What the open outage is recorded as: the gravest state it has reached, by
/// [`scope_rank`].
///
/// The row used to keep the state it opened with, so an outage that began as
/// "slow" and became the provider going down stayed "degraded" in the history
/// and was never counted towards the provider's pattern.
///
/// A graver state has to hold for as many sweeps as opening an outage takes.
/// It used to take one: a router that dropped a single ping to itself during
/// a two-hour provider outage relabelled the whole outage "lan", took it out
/// of the provider's pattern and put the household's kit in the report.
#[derive(Debug, Default)]
struct Recorded {
    status: Option<Status>,
    /// Graver-than-recorded sweeps in a row.
    graver_streak: u32,
}

impl Recorded {
    fn open(&mut self, status: Status) {
        self.status = Some(status);
        self.graver_streak = 0;
    }

    /// The state to write over the open outage's, once `status` and the
    /// sweeps before it have been graver than what it is recorded as for
    /// `needed` sweeps in a row. Never a milder one.
    fn escalate(&mut self, status: Status, needed: u32) -> Option<Status> {
        let recorded = self.status?;
        if scope_rank(status.scope()) <= scope_rank(recorded.scope()) {
            self.graver_streak = 0;
            return None;
        }
        self.graver_streak += 1;
        if self.graver_streak < needed {
            return None;
        }
        self.graver_streak = 0;
        self.status = Some(status);
        Some(status)
    }

    fn close(&mut self) {
        self.status = None;
        self.graver_streak = 0;
    }
}

/// What the tray tells the user through Windows. Only outages that reach the
/// history, and only hard ones: the user asked to hear "it is down" and "it is
/// back", and "it is slow" is what the icon's colour is for.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    Down {
        status: Status,
        note: String,
    },
    /// The outage that was announced as down is over, after `secs`.
    Up {
        secs: f64,
    },
}

/// Decides the notices, apart from the sweep, so the rules can be tested.
#[derive(Debug, Default)]
struct Announcer {
    /// When the open outage opened, as the history records it.
    opened_at: Option<f64>,
    /// Whether "down" was said for it, which is what earns it an "up".
    told: bool,
}

impl Announcer {
    /// After a sweep's [`Step`]. `open` is whether an outage is open once the
    /// step has been applied.
    fn after_sweep(
        &mut self,
        step: Step,
        open: bool,
        status: Status,
        note: &str,
        ts: f64,
        notify: bool,
    ) -> Option<Notice> {
        match step {
            Step::Open => self.opened_at = Some(ts),
            Step::Close { at, .. } => {
                let opened = self.opened_at.take();
                return std::mem::take(&mut self.told)
                    .then(|| Notice::Up { secs: opened.map_or(0.0, |o| (at - o).max(0.0)) });
            }
            Step::Nothing => {}
        }
        // Said once per outage, on the first sweep of it that is a hard
        // failure: an outage can open as "degraded" and turn into "ISP down".
        let hard = !matches!(status, Status::Ok | Status::Degraded);
        if open && hard && notify && !self.told {
            self.told = true;
            return Some(Notice::Down { status, note: note.to_string() });
        }
        None
    }

    /// The outage ended in a hole nobody watched.
    fn hole(&mut self) {
        *self = Announcer::default();
    }
}

fn run_loop(
    shared: Arc<Shared>,
    store: Arc<Store>,
    tx: Sender<Snapshot>,
    notices: Sender<Notice>,
    stop: Arc<AtomicBool>,
) {
    let mut pool = PingerPool::default();

    let mut net = netstate::read();
    let mut last_bssid = net.bssid.clone();
    let mut next_state_refresh = Instant::now();
    let mut sweep: u64 = 0;

    let mut outages = Outages::default();
    let mut announcer = Announcer::default();
    let mut recorded = Recorded::default();
    // The last sweep that was a hard failure. See the quality window below.
    let mut last_hard: Option<f64> = None;
    let mut open_event: Option<i64> = None;
    let mut lead: VecDeque<LeadSample> = VecDeque::with_capacity(LEAD_SWEEPS);
    let mut last_roam_ts: Option<f64> = None;

    // Continuity of observation. The first sweep asks the database whether it
    // is resuming a stretch or starting one; after that the loop is the only
    // writer of samples, so it can spot its own gaps -- a pause, a sleeping
    // machine -- without going back to SQLite every second.
    let mut observed_from = 0.0_f64;
    let mut prev_sweep_ts: Option<f64> = None;

    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        let settings = held(&shared.settings).clone();
        let interval = sweep_interval(&settings, shared.gaming.load(Ordering::Relaxed));

        if shared.paused() {
            // Nobody is watching from here on. What that means for an open
            // outage is decided when watching resumes: see `Outages`.
            outages.pause(prev_sweep_ts);
            thread::sleep(Duration::from_millis(200));
            continue;
        }

        // A hole this wide is a machine that slept, or a pause too long to
        // bridge. Whatever was open ended, as far as anyone can tell, at the
        // last sweep before it: an outage that happened to span a night used
        // to come back as eight hours of "ISP down". And the sweeps from
        // before the hole are not the lead-up to anything after it, so they
        // go too, as does the last access point, because a laptop carried to
        // another network while asleep has not roamed.
        if let Some(prev) = prev_sweep_ts {
            if store::now() - prev > store::OBSERVATION_GAP_S {
                // Nobody saw it come back, so there is no "restored" to say.
                announcer.hole();
                recorded.close();
                if let (Step::Close { at, .. }, Some(id)) = (outages.hole(prev), open_event.take())
                {
                    let _ = store.close_event_at(id, at, r#"{"closed_by":"gap"}"#);
                }
                lead.clear();
                last_bssid.clear();
                last_roam_ts = None;
                next_state_refresh = Instant::now();
            }
        }

        // Adapter state changes slowly and costs more to read than the pings,
        // so it is refreshed on its own slower cadence.
        if Instant::now() >= next_state_refresh {
            let fresh = netstate::read();
            // Named adapter or gateway: either is a real reading. Only a
            // wholly empty one — the enumeration itself failed — is worth
            // keeping the previous state for.
            //
            // This guard used to collapse to "has a gateway", because
            // `read_adapters` returned nothing at all without one. The state
            // therefore froze for the length of every outage, showing the
            // SSID, signal and channel from before it broke.
            if fresh.gateway.is_some() || !fresh.adapter_name.is_empty() {
                net = fresh;
            }
            next_state_refresh = Instant::now() + Duration::from_secs(5);
            *held(&shared.gateway) = net.gateway;
            *held(&shared.dns_servers) = net.dns_servers.clone();
        }

        // Read, never run: the lookup lives on `run_dns`.
        let (dns_ms, dns_error) = held(&shared.dns).current(Instant::now());

        let hosts = held(&shared.hosts).clone();
        let targets = resolve_targets(&settings, &net, &hosts);
        let ts = store::now();
        let mut results = HashMap::new();
        let mut rows = Vec::new();

        // All at once, one handle each: a sweep costs one timeout rather than
        // one per target, so the cadence holds during an outage instead of
        // stretching to four seconds exactly when the samples matter.
        let swept = match pool.sweep(&targets, settings.sweep_timeout_ms()) {
            Ok(swept) => swept,
            Err(e) => {
                // Nothing was sent, so there is no verdict and no sample. To
                // the outage bookkeeping this is a pause, and in the history
                // a gap: what happened meanwhile is not known.
                outages.pause(prev_sweep_ts);
                let snap = Snapshot {
                    ts,
                    net: net.clone(),
                    dns_ms,
                    dns_error,
                    blind: Some(i18n::mon_blind_detail(&e.describe())),
                    ..Snapshot::default()
                };
                *held(&shared.last) = snap.clone();
                let _ = tx.try_send(snap);
                pace(started, interval, &stop);
                continue;
            }
        };
        for (t, r) in targets.iter().zip(swept) {
            let sample = Sample {
                ok: r.ok(),
                rtt_ms: r.rtt_ms,
                error: r.error.as_ref().map(|e| e.describe()),
            };
            rows.push((ts, t.key.clone(), sample.rtt_ms, sample.ok));
            results.insert(t.key.clone(), sample);
        }

        // Continuity, before this sweep's rows land: on the first sweep the
        // question is whether the history runs right up to now, and after
        // that the loop can see its own gaps.
        match prev_sweep_ts {
            Some(prev) if ts - prev <= store::OBSERVATION_GAP_S => {}
            Some(_) => observed_from = ts,
            None => {
                let resumed = store
                    .last_sample_ts()
                    .filter(|last| ts - last <= store::OBSERVATION_GAP_S)
                    .and_then(|_| store.observing_since(store::OBSERVATION_GAP_S));
                observed_from = resumed.unwrap_or(ts);
            }
        }
        prev_sweep_ts = Some(ts);

        // Straight to the database. The sweep used to also keep a ring of
        // recent points per target for the chart to read; the chart now asks
        // the database for whatever window it is showing, which is the only
        // way a window longer than the ring -- or one that predates this
        // process -- can be drawn at all. Two copies of the same samples,
        // one of them capped at an arbitrary count, was one too many.
        let unrecorded =
            store.add_samples(&rows).err().map(|e| i18n::mon_unrecorded_detail(&e.to_string()));

        let roamed = !net.bssid.is_empty() && !last_bssid.is_empty() && net.bssid != last_bssid;
        if !net.bssid.is_empty() {
            last_bssid = net.bssid.clone();
        }
        if roamed {
            last_roam_ts = Some(ts);
        }

        // Loss and jitter are read from after the last hard outage, never
        // across it. Over a plain 60 s window the outage's own lost pings
        // kept the line "degraded" for up to a minute after it came back,
        // which held the outage open that long: every recorded outage ran a
        // minute long, and "restored" arrived a minute late.
        let quality_window =
            last_hard.map_or(QUALITY_WINDOW_S, |h| (ts - h - 0.5).clamp(0.0, QUALITY_WINDOW_S));
        // Whether pings to the public anchors are all going unanswered, and
        // since when: `run_tcp` works only while they are.
        let heard = answered_publicly(&results, &targets);
        let dark_since = {
            let mut dark = held(&shared.icmp_dark_since);
            if heard {
                *dark = None;
            } else if dark.is_none() {
                *dark = Some(Instant::now());
            }
            *dark
        };
        let tcp = *held(&shared.tcp);
        let tcp_vouches = !heard && tcp.is_some_and(|t| t.vouches(dark_since, Instant::now()));
        let (status, note) = past_filtered_pings(
            classify(&results, &targets, &net, &dns_error, &settings, &store, quality_window),
            tcp_vouches,
        );
        if !matches!(status, Status::Ok | Status::Degraded) {
            last_hard = Some(ts);
        }

        let snap = Snapshot {
            ts,
            status,
            note,
            results,
            net: net.clone(),
            dns_ms,
            dns_error: dns_error.clone(),
            roamed,
            observed_from: Some(observed_from),
            blind: None,
            unrecorded,
            // The same read `classify` judged on, repeated for the cards: two
            // small queries, and the figures cannot drift from the verdict.
            quality: line_quality(&targets, &store, quality_window),
        };

        // The lead-up is recorded on every sweep, good ones included: by the
        // time an outage is confirmed the interesting sweeps are already past.
        lead.push_back(lead_sample(&snap));
        while lead.len() > LEAD_SWEEPS {
            lead.pop_front();
        }

        // Event bookkeeping: only open after a few consecutive bad sweeps so a
        // single dropped packet does not fill the log with noise.
        let bad = status != Status::Ok;

        // A roam counts as "recent" for a minute either way; the disconnect it
        // causes usually lands a few sweeps after the BSSID actually changes.
        let roamed_recently = last_roam_ts.is_some_and(|t| ts - t <= 60.0);

        let step = outages.sweep(ts, bad, settings.outage_after_fails);
        match step {
            Step::Open => {
                let path = held(&shared.path).clone();
                let context = context_json(&snap, &lead, roamed_recently, Some(&path)).to_string();
                open_event =
                    store.open_event(status.key(), status.scope(), &snap.note, &context).ok();
                if open_event.is_none() {
                    outages.abandon();
                } else {
                    recorded.open(status);
                }
            }
            Step::Nothing => {
                if let (Some(graver), Some(id)) =
                    (recorded.escalate(status, settings.outage_after_fails), open_event)
                {
                    let _ = store.set_event_kind(id, graver.key(), graver.scope(), &snap.note);
                }
            }
            Step::Close { at, unwatched_s, .. } => {
                recorded.close();
                if let Some(id) = open_event.take() {
                    // An empty lead-up: what matters at recovery is the state
                    // the connection came back into, not another copy of the
                    // history.
                    let mut end = context_json(&snap, &VecDeque::new(), roamed_recently, None);
                    if unwatched_s > 0.0 {
                        end["unwatched_s"] = serde_json::json!(unwatched_s);
                    }
                    let _ = store.close_event_at(id, at, &end.to_string());
                }
            }
        }
        let heard = announcer.after_sweep(
            step,
            outages.open,
            status,
            &snap.note,
            ts,
            settings.notify_on_outage,
        );
        if let Some(notice) = heard {
            // A full queue means nobody is reading it; the tray is the only
            // listener and drains it every second.
            let _ = notices.try_send(notice);
        }

        *held(&shared.last) = snap.clone();
        // A full channel means the UI is behind; dropping is correct here.
        let _ = tx.try_send(snap);

        sweep += 1;
        if sweep % 600 == 0 {
            let _ = store.prune(settings.keep_days);
        }

        pace(started, interval, &stop);
    }

    // Shutting down while an outage is open: close it at the last sweep that
    // saw it (the app may have sat paused for a while first), and say nothing
    // about a recovery that never happened.
    if let Some(id) = open_event {
        let at = prev_sweep_ts.unwrap_or_else(store::now);
        let _ = store.close_event_at(id, at, "");
    }
}

/// How often a sweep runs while a game is being played. A spike has to be
/// caught while it is happening, and a second between readings is long
/// enough to miss the one that cost the round.
const GAME_INTERVAL: Duration = Duration::from_millis(500);

/// The sweep interval: the user's, or [`GAME_INTERVAL`] while a game runs
/// if the user's is slower.
fn sweep_interval(settings: &Settings, gaming: bool) -> Duration {
    let interval = settings.interval();
    if gaming {
        interval.min(GAME_INTERVAL)
    } else {
        interval
    }
}

/// Sleeps out what is left of a sweep's interval, waking early enough to
/// notice a stop request promptly.
fn pace(started: Instant, interval: Duration, stop: &AtomicBool) {
    let _ = nap(stop, interval.saturating_sub(started.elapsed()));
}

/// How far back the line's loss and jitter are judged from, at most.
const QUALITY_WINDOW_S: f64 = 60.0;

/// Fewer sweeps than this in the window, and loss and jitter are not judged
/// at all. Right after an outage the window is short, and one lost reply out
/// of three would read as 33% loss.
const MIN_QUALITY_SAMPLES: usize = 10;

/// The blame logic. Everything else in the app exists to support this.
///
/// `quality_window_s` is how far back loss and jitter are
/// read from: [`QUALITY_WINDOW_S`], or less right after a hard outage (see
/// `run_loop`).
fn classify(
    results: &HashMap<String, Sample>,
    targets: &[Resolved],
    net: &NetState,
    dns_error: &str,
    settings: &Settings,
    store: &Store,
    quality_window_s: f64,
) -> (Status, String) {
    // Only a public address can vouch for the internet: see
    // [`crate::diagnose::is_public`]. A resolver the user runs at home, or a
    // host they added on their own network, used to answer for it and hide
    // an outage at the provider.
    let internet_ok = answered_publicly(results, targets);

    let gw = results.get("gateway").map(|s| s.ok);

    if internet_ok {
        if !dns_error.is_empty() {
            return (Status::DnsFail, i18n::mon_dns_detail(dns_error));
        }
        return quality_verdict(results, targets, settings, store, quality_window_s);
    }

    // Nothing on the internet answered. Who is still there?
    match gw {
        Some(true) => (
            Status::IspDown,
            i18n::mon_isp_detail(&net.gateway.map(|g| g.to_string()).unwrap_or_else(|| "?".into())),
        ),
        Some(false) => {
            if net.medium == Medium::Wifi
                && netstate::wifi_associated(&net.adapter_guid) == Some(false)
            {
                (Status::AdapterDown, i18n::mon_wifi_deassociated().into())
            } else {
                (Status::LanDown, i18n::mon_nothing_responded().into())
            }
        }
        None => (Status::AdapterDown, i18n::mon_no_gateway().into()),
    }
}

/// Whether any public internet or provider target answered this sweep.
fn answered_publicly(results: &HashMap<String, Sample>, targets: &[Resolved]) -> bool {
    targets
        .iter()
        .filter(|t| matches!(t.scope, Scope::Internet | Scope::Isp))
        .filter(|t| crate::diagnose::is_public(t.host))
        .any(|t| results.get(&t.key).map(|s| s.ok).unwrap_or(false))
}

/// A verdict of "nothing out there answers" that a TCP connection just
/// contradicted: the pings are filtered, the internet is not down. Reported
/// as working, with a note that says why the latency figures are missing,
/// rather than as an outage nobody had.
fn past_filtered_pings(verdict: (Status, String), tcp_vouches: bool) -> (Status, String) {
    match verdict.0 {
        Status::IspDown | Status::LanDown | Status::AdapterDown if tcp_vouches => {
            (Status::Ok, i18n::mon_icmp_filtered().into())
        }
        _ => verdict,
    }
}

/// The link is up; decide whether it is actually usable.
///
/// Every public internet target is read, and each measure is taken from the
/// target that shows the least of it. Loss, jitter and delay on the line
/// itself show on all of them; on one alone they belong to that responder,
/// or to a filter in front of it. Reading 1.1.1.1 only turned a network that
/// drops pings to it into a permanent "degraded".
fn quality_verdict(
    results: &HashMap<String, Sample>,
    targets: &[Resolved],
    settings: &Settings,
    store: &Store,
    window_s: f64,
) -> (Status, String) {
    let fastest = public_internet(targets)
        .filter_map(|t| results.get(&t.key).and_then(|s| s.rtt_ms))
        .fold(f64::NAN, f64::min);

    let q = line_quality(targets, store, window_s);
    if let Some(loss) = q.loss_pct.filter(|l| *l > settings.loss_ok_pct) {
        return (Status::Degraded, i18n::mon_loss_detail(loss));
    }
    // The same limit the Diagnose tab warns at, and the one the setting names.
    // This used to be twice it, so 20 ms read "healthy" here and a warning there.
    if let Some(jitter) = q.jitter_ms.filter(|j| *j > settings.jitter_ok_ms) {
        return (Status::Degraded, i18n::mon_jitter_detail(jitter));
    }
    if fastest > settings.ping_bad_ms {
        return (Status::Degraded, i18n::mon_ping_detail(fastest));
    }
    (Status::Ok, String::new())
}

fn public_internet(targets: &[Resolved]) -> impl Iterator<Item = &Resolved> {
    targets.iter().filter(|t| t.scope == Scope::Internet && crate::diagnose::is_public(t.host))
}

/// The line's loss and jitter as the verdict judges them: the lowest of each
/// over the public internet targets with enough readings in the window.
/// Published in the snapshot, so the Live tab's cards show the very figures
/// the headline was decided on. They used to read 1.1.1.1 alone over five
/// minutes, and put a red "2.8% loss" beside a green "Connection healthy".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LineQuality {
    /// `None` when no target had enough readings to judge.
    pub loss_pct: Option<f64>,
    pub jitter_ms: Option<f64>,
    /// The window they were read over, in seconds.
    pub window_s: f64,
}

fn line_quality(targets: &[Resolved], store: &Store, window_s: f64) -> LineQuality {
    let judged: Vec<store::Stats> = public_internet(targets)
        .map(|t| store.stats(&t.key, window_s))
        .filter(|s| s.count >= MIN_QUALITY_SAMPLES)
        .collect();
    let lowest = |v: Vec<f64>| v.into_iter().reduce(f64::min);
    LineQuality {
        loss_pct: lowest(judged.iter().map(|s| s.loss_pct).collect()),
        jitter_ms: lowest(judged.iter().filter_map(|s| s.jitter).collect()),
        window_s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four addresses reserved for documentation (RFC 5737). Nothing answers
    /// at them, so every ping runs the full timeout — which is the shape of a
    /// real outage, when the router and all three internet targets are gone
    /// at once.
    const DEAD: [Ipv4Addr; 4] = [
        Ipv4Addr::new(192, 0, 2, 1),
        Ipv4Addr::new(192, 0, 2, 2),
        Ipv4Addr::new(198, 51, 100, 1),
        Ipv4Addr::new(203, 0, 113, 1),
    ];

    fn dead_targets() -> Vec<Resolved> {
        DEAD.iter()
            .enumerate()
            .map(|(i, host)| Resolved {
                key: format!("dead{i}"),
                host: *host,
                scope: Scope::Internet,
            })
            .collect()
    }

    #[test]
    fn a_sweep_costs_one_timeout_and_not_one_per_target() {
        // The bug this guards: four silent targets pinged in turn cost four
        // timeouts, so the sampling cadence fell from the configured second
        // to nearly four during an outage — and `mark_recording_gaps` then
        // read those slow sweeps as the app not recording at all.
        //
        // The threshold is not "one timeout", because a ping to an
        // unreachable address does not come back inside a short one. Measured
        // on this machine, four dead targets at a 150 ms timeout:
        //
        //     sequential 2.00 s   parallel 0.50 s
        //
        // The same 4x holds at 1000 ms (3.99 s against 1.01 s), and below
        // roughly 500 ms per probe the timeout stops buying anything at all.
        // So 1.2 s sits an octave clear of the parallel result and well under
        // the serialised one, and the test costs half a second rather than
        // four.
        let targets = dead_targets();
        let timeout = 150;

        let mut pool = PingerPool::default();
        let at = std::time::Instant::now();
        let results = pool.sweep(&targets, timeout).expect("an ICMP handle");
        let elapsed = at.elapsed();

        assert_eq!(results.len(), targets.len(), "every target gets a result");
        assert!(results.iter().all(|r| !r.ok()), "nothing answers at a reserved address");
        assert!(
            elapsed < Duration::from_millis(1_200),
            "four dead targets took {elapsed:?}; the sweep is serialised again"
        );
    }

    #[test]
    fn each_result_belongs_to_the_target_that_produced_it() {
        // The pool hands chunks to threads and stitches the answers back
        // together by index. Getting that wrong would file the router's
        // latency under the name of an internet target, which is the kind of
        // mistake that makes the whole verdict wrong rather than merely late.
        let mut targets = dead_targets();
        targets.insert(
            2,
            Resolved {
                key: "loopback".into(),
                host: Ipv4Addr::new(127, 0, 0, 1),
                scope: Scope::Lan,
            },
        );

        let mut pool = PingerPool::default();
        let results = pool.sweep(&targets, 300).expect("an ICMP handle");

        assert_eq!(results.len(), 5);
        for (t, r) in targets.iter().zip(&results) {
            match t.key.as_str() {
                "loopback" => assert!(r.ok(), "this machine answers itself"),
                _ => assert!(!r.ok(), "{} is a reserved address and must not reply", t.key),
            }
        }
    }

    #[test]
    fn a_sweep_that_could_not_open_icmp_is_not_an_outage() {
        // The bug: with no handle the pool returned an empty list, `classify`
        // found no router result, and "adapter down" went into the history
        // and out as a notification. Nothing had been measured at all.
        let mut pool = PingerPool::default();
        let swept =
            pool.sweep_opening(&targets(), 100, || Err(PingError::Api("access denied".into())));
        if let Ok(results) = swept {
            let results: HashMap<String, Sample> = targets()
                .iter()
                .zip(results)
                .map(|(t, r)| (t.key.clone(), sample(r.ok(), r.rtt_ms)))
                .collect();
            let store = Store::open_in_memory().unwrap();
            let (status, _) = classify(
                &results,
                &targets(),
                &wifi_state(),
                "",
                &Settings::default(),
                &store,
                QUALITY_WINDOW_S,
            );
            panic!("a sweep that measured nothing was judged as {status:?}");
        }
    }

    /// What a sweep costs when nothing answers. Ignored: it takes seconds and
    /// measures the machine.
    #[test]
    #[ignore = "measures a sweep against unreachable targets"]
    fn sweep_cost_with_dead_targets() {
        let targets = dead_targets();
        let timeout = 1000;

        let pinger = Pinger::new().expect("an ICMP handle");
        let at = std::time::Instant::now();
        for t in &targets {
            let _ = pinger.ping(t.host, timeout);
        }
        println!("sequential, one handle: {:?}", at.elapsed());

        let mut pool = PingerPool::default();
        let at = std::time::Instant::now();
        let _ = pool.sweep(&targets, timeout);
        println!("parallel, one handle each: {:?}", at.elapsed());

        // Both paths at each timeout. The gap between them is what the
        // non-ignored test asserts against, and the floor visible here is why
        // that test cannot simply compare against the timeout.
        for t_ms in [150u32, 300, 500, 1000] {
            let one = Pinger::new().expect("an ICMP handle");
            let at = std::time::Instant::now();
            for t in &targets {
                let _ = one.ping(t.host, t_ms);
            }
            let seq = at.elapsed();

            let mut pool = PingerPool::default();
            let at = std::time::Instant::now();
            let _ = pool.sweep(&targets, t_ms);
            println!("timeout {t_ms:>4} ms -> sequential {seq:?}, parallel {:?}", at.elapsed());
        }

        // What the monitor actually runs with, against the interval it has to
        // fit inside.
        let cfg = Settings::default();
        let mut pool = PingerPool::default();
        let at = std::time::Instant::now();
        let _ = pool.sweep(&targets, cfg.sweep_timeout_ms());
        println!(
            "as configured: sweep_timeout {} ms -> {:?}, interval {:?}",
            cfg.sweep_timeout_ms(),
            at.elapsed(),
            cfg.interval()
        );
    }

    fn sample(ok: bool, rtt: Option<f64>) -> Sample {
        Sample { ok, rtt_ms: rtt, error: None }
    }

    fn targets() -> Vec<Resolved> {
        vec![
            Resolved {
                key: "gateway".into(),
                host: Ipv4Addr::new(192, 168, 50, 1),
                scope: Scope::Lan,
            },
            Resolved {
                key: "cloudflare".into(),
                host: Ipv4Addr::new(1, 1, 1, 1),
                scope: Scope::Internet,
            },
        ]
    }

    fn wifi_state() -> NetState {
        NetState {
            gateway: Some(Ipv4Addr::new(192, 168, 50, 1)),
            medium: Medium::Ethernet, // avoids touching the WLAN API in tests
            ..Default::default()
        }
    }

    /// Three bad sweeps a second apart from `t`, which opens an outage at the
    /// default threshold.
    fn opened_at(t: f64) -> Outages {
        let mut o = Outages::default();
        assert_eq!(o.sweep(t, true, 3), Step::Nothing);
        assert_eq!(o.sweep(t + 1.0, true, 3), Step::Nothing);
        assert_eq!(o.sweep(t + 2.0, true, 3), Step::Open);
        o
    }

    #[test]
    fn an_unbroken_outage_closes_when_a_sweep_sees_it_recover() {
        let mut o = opened_at(0.0);
        assert_eq!(o.sweep(3.0, true, 3), Step::Nothing);
        assert_eq!(o.sweep(4.0, false, 3), Step::Close { at: 4.0, unwatched_s: 0.0 });
        assert_eq!(o.sweep(5.0, false, 3), Step::Nothing, "closed once, not twice");
    }

    #[test]
    fn an_outage_still_there_after_a_short_pause_is_the_same_outage() {
        // A load test holds the monitor for twenty seconds in the middle of
        // an outage. Closing on pause split this into two outages.
        let mut o = opened_at(0.0);
        o.pause(Some(2.0));
        o.pause(Some(2.0)); // every paused iteration calls it
        assert_eq!(o.sweep(22.0, true, 3), Step::Nothing, "still open, not reopened");
        assert_eq!(
            o.sweep(23.0, false, 3),
            Step::Close { at: 23.0, unwatched_s: 20.0 },
            "the bridged seconds travel with it"
        );
    }

    #[test]
    fn a_recovery_during_a_pause_ends_at_the_last_sweep_that_saw_it_down() {
        // Nobody saw when it came back, only that it was down at 2 s and up
        // at 22 s. Ending it at 22 would bill the pause to the outage.
        let mut o = opened_at(0.0);
        o.pause(Some(2.0));
        assert_eq!(o.sweep(22.0, false, 3), Step::Close { at: 2.0, unwatched_s: 0.0 });
    }

    #[test]
    fn a_hole_too_wide_to_bridge_ends_the_outage_before_it() {
        let mut o = opened_at(0.0);
        o.pause(Some(2.0));
        assert_eq!(o.hole(2.0), Step::Close { at: 2.0, unwatched_s: 0.0 });
        // And the streak starts again: one bad sweep after waking is not an
        // outage.
        assert_eq!(o.sweep(8000.0, true, 3), Step::Nothing);
        assert_eq!(o.hole(8000.0), Step::Nothing, "nothing open, nothing to close");
    }

    /// Runs sweeps through `Outages` and `Announcer` together, the way
    /// `run_loop` does, and collects what would be said.
    fn heard(sweeps: &[(f64, Status)], notify: bool) -> Vec<Notice> {
        let (mut o, mut a) = (Outages::default(), Announcer::default());
        sweeps
            .iter()
            .filter_map(|(ts, status)| {
                let step = o.sweep(*ts, *status != Status::Ok, 3);
                a.after_sweep(step, o.open, *status, "note", *ts, notify)
            })
            .collect()
    }

    #[test]
    fn a_hard_outage_is_announced_once_when_it_reaches_the_history_and_again_when_it_ends() {
        use Status::{IspDown, Ok};
        let said = heard(
            &[(0.0, IspDown), (1.0, IspDown), (2.0, IspDown), (3.0, IspDown), (32.0, Ok)],
            true,
        );
        assert_eq!(
            said,
            vec![
                Notice::Down { status: IspDown, note: "note".into() },
                Notice::Up { secs: 30.0 },
            ],
            "nothing for the first two sweeps, one down at the third, one up with the length the history records"
        );
    }

    #[test]
    fn a_blip_shorter_than_the_threshold_says_nothing() {
        use Status::{LanDown, Ok};
        assert!(heard(&[(0.0, LanDown), (1.0, LanDown), (2.0, Ok)], true).is_empty());
    }

    #[test]
    fn a_slow_line_is_the_icon_s_business_not_a_notification() {
        use Status::{Degraded, Ok};
        let said = heard(&[(0.0, Degraded), (1.0, Degraded), (2.0, Degraded), (3.0, Ok)], true);
        assert!(said.is_empty(), "degraded opens an outage in the history, but is not 'down'");
    }

    #[test]
    fn an_outage_that_turns_hard_is_announced_when_it_does() {
        use Status::{Degraded, IspDown, Ok};
        let said = heard(
            &[(0.0, Degraded), (1.0, Degraded), (2.0, Degraded), (3.0, IspDown), (10.0, Ok)],
            true,
        );
        assert_eq!(
            said,
            vec![Notice::Down { status: IspDown, note: "note".into() }, Notice::Up { secs: 8.0 },],
            "the length runs from when the outage opened, as in the history"
        );
    }

    #[test]
    fn the_setting_silences_both_ends() {
        use Status::{LanDown, Ok};
        let s = [(0.0, LanDown), (1.0, LanDown), (2.0, LanDown), (9.0, Ok)];
        assert!(heard(&s, false).is_empty());
    }

    #[test]
    fn an_outage_lost_in_a_hole_is_never_called_restored() {
        let (mut o, mut a) = (Outages::default(), Announcer::default());
        for ts in [0.0, 1.0] {
            let step = o.sweep(ts, true, 3);
            assert_eq!(a.after_sweep(step, o.open, Status::LanDown, "", ts, true), None);
        }
        let step = o.sweep(2.0, true, 3);
        assert!(matches!(
            a.after_sweep(step, o.open, Status::LanDown, "", 2.0, true),
            Some(Notice::Down { .. })
        ));
        // The machine slept; the sweep after waking is fine.
        a.hole();
        let _ = o.hole(2.0);
        let step = o.sweep(9000.0, false, 3);
        assert_eq!(a.after_sweep(step, o.open, Status::Ok, "", 9000.0, true), None);
    }

    #[test]
    fn an_outage_is_recorded_as_the_worst_it_became() {
        // Opened as "slow", turned into the provider going down. The history
        // kept "degraded", and `isp_pattern` never counted it.
        let store = Store::open_in_memory().unwrap();
        let (mut o, mut rec) = (Outages::default(), Recorded::default());
        let mut id = None;
        let seq = [
            (0.0, Status::Degraded),
            (1.0, Status::Degraded),
            (2.0, Status::Degraded),
            (3.0, Status::IspDown),
            (4.0, Status::Degraded),
            (5.0, Status::IspDown),
            (6.0, Status::IspDown),
            (7.0, Status::IspDown),
        ];
        for (ts, status) in seq {
            match o.sweep(ts, true, 3) {
                Step::Open => {
                    id = store.open_event(status.key(), status.scope(), "n", "{}").ok();
                    rec.open(status);
                }
                _ => {
                    if let (Some(graver), Some(id)) = (rec.escalate(status, 3), id) {
                        store.set_event_kind(id, graver.key(), graver.scope(), "n").unwrap();
                    }
                }
            }
        }
        let events = store.events_since(3600.0);
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].kind.as_str(), events[0].scope.as_str()), ("isp_down", "isp"));

        // Never downgraded: a better sweep inside the outage changes nothing.
        assert_eq!(rec.escalate(Status::DnsFail, 1), None);
        assert_eq!(rec.escalate(Status::LanDown, 1), Some(Status::LanDown));
        assert_eq!(rec.escalate(Status::IspDown, 1), None);
        rec.close();
        assert_eq!(rec.escalate(Status::AdapterDown, 1), None, "nothing open, nothing to rename");
    }

    #[test]
    fn a_tcp_connection_after_the_pings_went_quiet_overrules_an_outage() {
        let dark = Instant::now();
        let later = dark + Duration::from_secs(1);
        let ok = TcpAttempt { at: later, ok: true };
        assert!(ok.vouches(Some(dark), later));
        // Stale: the network may have gone down since.
        assert!(!ok.vouches(Some(dark), later + TCP_FRESH + Duration::from_secs(1)));
        // From before the silence: says nothing about it.
        let before = TcpAttempt { at: dark, ok: true };
        assert!(!before.vouches(Some(later), later));
        // Failed, or pings answering and nothing to vouch for.
        assert!(!TcpAttempt { at: later, ok: false }.vouches(Some(dark), later));
        assert!(!ok.vouches(None, later));

        let isp = (Status::IspDown, "n".to_string());
        assert_eq!(past_filtered_pings(isp.clone(), true).0, Status::Ok);
        assert_eq!(past_filtered_pings(isp, false).0, Status::IspDown);
        // A verdict reached with pings answering is not the TCP probe's to change.
        let dns = (Status::DnsFail, "n".to_string());
        assert_eq!(past_filtered_pings(dns, true).0, Status::DnsFail);
    }

    #[test]
    fn a_game_tightens_the_cadence_and_never_slows_it() {
        let every = |ms| Settings { probe_interval_ms: ms, ..Settings::default() };
        assert_eq!(sweep_interval(&every(1000), true), GAME_INTERVAL);
        assert_eq!(sweep_interval(&every(1000), false), Duration::from_millis(1000));
        // Already faster than the game cadence: left alone.
        assert_eq!(sweep_interval(&every(300), true), Duration::from_millis(300));
    }

    #[test]
    fn one_lost_router_ping_does_not_relabel_a_provider_outage() {
        let mut rec = Recorded::default();
        rec.open(Status::IspDown);
        // The router misses one ping, twice, with the provider still down in
        // between: noise, and the outage stays the provider's.
        for status in [Status::LanDown, Status::IspDown, Status::LanDown, Status::IspDown] {
            assert_eq!(rec.escalate(status, 3), None);
        }
        // Held for three sweeps, it is the household's link after all.
        assert_eq!(rec.escalate(Status::LanDown, 3), None);
        assert_eq!(rec.escalate(Status::AdapterDown, 3), None);
        assert_eq!(rec.escalate(Status::LanDown, 3), Some(Status::LanDown));
    }

    #[test]
    fn an_outage_keeps_the_path_as_it_was_when_it_began() {
        // The report used to print the hop table as it is now, under outages
        // from days ago.
        use crate::probe::path::{HopReading, Owner};
        let path = PathReading {
            hops: vec![HopReading {
                ttl: 1,
                addr: Ipv4Addr::new(192, 168, 1, 1),
                owner: Owner::Gateway,
                loss_pct: 0.0,
                avg_ms: Some(2.0),
                samples: 10,
                silent: false,
            }],
            blame: None,
        };
        let snap = Snapshot::default();
        let ctx = context_json(&snap, &VecDeque::new(), false, Some(&path));
        let back: PathReading = serde_json::from_value(ctx["path"].clone()).unwrap();
        assert_eq!(back.hops.len(), 1);
        assert_eq!(back.hops[0].addr, Ipv4Addr::new(192, 168, 1, 1));

        // A path not walked yet is not stored as an empty table, which
        // would read as "no hops".
        let none = context_json(&snap, &VecDeque::new(), false, Some(&PathReading::default()));
        assert!(none["path"].is_null());
    }

    #[test]
    fn a_pause_with_nothing_open_changes_nothing() {
        let mut o = Outages::default();
        o.pause(Some(5.0));
        assert_eq!(o.sweep(10.0, false, 3), Step::Nothing);
    }

    #[test]
    fn one_failed_lookup_is_not_a_dns_outage_and_a_failure_is_rechecked_soon() {
        // One lost UDP reply used to stand as the answer for the next ten
        // seconds: ten bad sweeps, a recorded outage and a notification.
        let t0 = Instant::now();
        let mut r = DnsReading::default();
        r.finish(Some(4.0), String::new());
        r.finish(None, "timed out".into());
        assert_eq!(r.current(t0).1, "", "one failure is not reported");
        assert!(r.next_check() < DNS_EVERY, "and it is looked at again soon");
        r.finish(None, "timed out".into());
        assert_eq!(r.current(t0).1, "timed out", "two in a row are");
        assert!(r.next_check() <= Duration::from_secs(2), "so its end is not rounded to 10 s");
        r.finish(Some(5.0), String::new());
        assert_eq!(r.current(t0), (Some(5.0), String::new()));
        assert_eq!(r.next_check(), DNS_EVERY);
    }

    #[test]
    fn a_hung_dns_lookup_is_a_failure_not_the_last_good_answer() {
        let t0 = Instant::now();
        let reading = DnsReading { ms: Some(4.0), started: Some(t0), ..DnsReading::default() };
        // Still inside the allowance: the previous answer stands.
        assert_eq!(reading.current(t0 + Duration::from_secs(1)).0, Some(4.0));
        // Past it: reported as a failure, with a reason.
        let (ms, err) = reading.current(t0 + DNS_STALL + Duration::from_secs(1));
        assert_eq!(ms, None);
        assert!(!err.is_empty());
        // A finished lookup reports what it found, however old.
        let done = DnsReading { started: None, ..reading };
        assert_eq!(done.current(t0 + Duration::from_secs(60)).0, Some(4.0));
    }

    #[test]
    fn user_hostnames_come_from_the_cache_and_never_from_a_lookup() {
        let s = Settings {
            extra_targets: vec![
                "watched.example".into(),
                "9.9.9.9".into(),
                "unresolved.example".into(),
            ],
            ..Settings::default()
        };
        let net = NetState { gateway: Some(Ipv4Addr::new(192, 168, 1, 1)), ..Default::default() };
        let mut hosts = HashMap::new();
        hosts.insert("watched.example".to_string(), Ipv4Addr::new(203, 0, 113, 7));

        let resolved = resolve_targets(&s, &net, &hosts);
        let addrs: Vec<Ipv4Addr> = resolved.iter().map(|t| t.host).collect();
        assert!(addrs.contains(&Ipv4Addr::new(203, 0, 113, 7)), "cached hostname used");
        assert!(addrs.contains(&Ipv4Addr::new(9, 9, 9, 9)), "a literal needs no cache");
        // Router, 1.1.1.1 and 8.8.8.8 (no separate ISP resolver here), plus
        // the two above. The uncached name is left out, not looked up.
        assert_eq!(resolved.len(), 5);
    }

    #[test]
    fn a_finished_job_does_not_unpause_under_another_or_the_user() {
        let s = Shared::new(Settings::default());
        s.hold(); // load test
        s.hold(); // air scan
        s.release();
        assert!(s.paused(), "the load test is still running");
        s.release();
        assert!(!s.paused());

        s.user_paused.store(true, Ordering::Relaxed);
        s.hold();
        s.release();
        assert!(s.paused(), "the user's pause outlives the job's");

        s.user_paused.store(false, Ordering::Relaxed);
        s.release(); // stray
        assert!(!s.paused(), "a stray release must not wrap into a hold");
    }

    #[test]
    fn everything_up_is_ok() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(true, Some(12.0)));
        let (status, _) = classify(
            &r,
            &targets(),
            &wifi_state(),
            "",
            &Settings::default(),
            &store,
            QUALITY_WINDOW_S,
        );
        assert_eq!(status, Status::Ok);
    }

    #[test]
    fn router_up_internet_down_blames_the_isp() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(false, None));
        let (status, note) = classify(
            &r,
            &targets(),
            &wifi_state(),
            "",
            &Settings::default(),
            &store,
            QUALITY_WINDOW_S,
        );
        assert_eq!(status, Status::IspDown);
        assert!(note.contains("Router"));
    }

    #[test]
    fn a_local_box_that_answers_does_not_prove_the_internet_works() {
        // A Pi-hole or a NAS added as a target, or a resolver that is really
        // the provider's CGNAT box: each answered, and the verdict was "Ok"
        // with every public target silent.
        let resolved = |key: &str, a: [u8; 4], scope| Resolved {
            key: key.into(),
            host: Ipv4Addr::from(a),
            scope,
        };
        let targets = vec![
            resolved("gateway", [192, 168, 1, 1], Scope::Lan),
            resolved("dns_isp", [100, 64, 0, 1], Scope::Isp),
            resolved("cloudflare", [1, 1, 1, 1], Scope::Internet),
            resolved("google", [8, 8, 8, 8], Scope::Internet),
            resolved("custom0", [192, 168, 1, 5], Scope::Internet),
            resolved("custom1", [169, 254, 3, 3], Scope::Internet),
            resolved("custom2", [10, 0, 0, 9], Scope::Internet),
        ];
        let mut r = HashMap::new();
        for t in &targets {
            let public = matches!(t.key.as_str(), "cloudflare" | "google");
            r.insert(t.key.clone(), sample(!public, (!public).then_some(3.0)));
        }
        let store = Store::open_in_memory().unwrap();
        let s = Settings::default();
        let (status, _) = classify(&r, &targets, &wifi_state(), "", &s, &store, QUALITY_WINDOW_S);
        assert_eq!(status, Status::IspDown);

        // And a public address the user added does count.
        r.insert("custom0".into(), sample(false, None));
        let mut public = targets;
        public.push(resolved("custom3", [9, 9, 9, 9], Scope::Internet));
        r.insert("custom3".into(), sample(true, Some(20.0)));
        let (status, _) = classify(&r, &public, &wifi_state(), "", &s, &store, QUALITY_WINDOW_S);
        assert_ne!(status, Status::IspDown);
    }

    #[test]
    fn nothing_answering_blames_the_local_link() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(false, None));
        r.insert("cloudflare".into(), sample(false, None));
        let (status, _) = classify(
            &r,
            &targets(),
            &wifi_state(),
            "",
            &Settings::default(),
            &store,
            QUALITY_WINDOW_S,
        );
        assert_eq!(status, Status::LanDown);
    }

    #[test]
    fn missing_gateway_means_the_adapter_is_down() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("cloudflare".into(), sample(false, None));
        let net = NetState { gateway: None, ..Default::default() };
        let (status, _) =
            classify(&r, &targets(), &net, "", &Settings::default(), &store, QUALITY_WINDOW_S);
        assert_eq!(status, Status::AdapterDown);
    }

    #[test]
    fn reachable_ip_with_broken_names_is_a_dns_fault() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(true, Some(12.0)));
        let (status, _) = classify(
            &r,
            &targets(),
            &wifi_state(),
            "getaddrinfo failed",
            &Settings::default(),
            &store,
            QUALITY_WINDOW_S,
        );
        assert_eq!(status, Status::DnsFail);
    }

    #[test]
    fn packet_loss_downgrades_a_working_link_to_degraded() {
        let store = Store::open_in_memory().unwrap();
        let t = store::now();
        let mut rows = Vec::new();
        for i in 0..10 {
            // 40% loss in the last minute
            let ok = i % 5 >= 2;
            rows.push((t - i as f64, "cloudflare".to_string(), ok.then_some(12.0), ok));
        }
        store.add_samples(&rows).unwrap();

        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(true, Some(12.0)));
        let (status, note) = classify(
            &r,
            &targets(),
            &wifi_state(),
            "",
            &Settings::default(),
            &store,
            QUALITY_WINDOW_S,
        );
        assert_eq!(status, Status::Degraded);
        assert!(note.contains(if crate::i18n::current() == crate::i18n::Lang::Pl {
            "utraconych pakietów"
        } else {
            "packet loss"
        }));
    }

    fn line_up() -> HashMap<String, Sample> {
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(true, Some(12.0)));
        r
    }

    #[test]
    fn the_outage_s_own_lost_pings_do_not_keep_the_line_degraded() {
        // Found live: Wi-Fi cut for ten seconds, back for fifteen, and the
        // line was still "degraded" on the loss the outage itself caused.
        let store = Store::open_in_memory().unwrap();
        let t = store::now();
        let mut rows = Vec::new();
        for i in 21..=30 {
            rows.push((t - i as f64, "cloudflare".to_string(), None, false));
        }
        for i in 1..=15 {
            rows.push((t - i as f64, "cloudflare".to_string(), Some(12.0), true));
        }
        store.add_samples(&rows).unwrap();

        let s = Settings::default();
        let across = classify(&line_up(), &targets(), &wifi_state(), "", &s, &store, 60.0).0;
        assert_eq!(across, Status::Degraded, "the old window reads the outage as loss");
        let after = classify(&line_up(), &targets(), &wifi_state(), "", &s, &store, 16.0).0;
        assert_eq!(after, Status::Ok, "from the outage's end on, nothing was lost");
    }

    #[test]
    fn one_filtered_target_is_not_a_lossy_line() {
        // A network that drops pings to 1.1.1.1 while 8.8.8.8 answers: loss
        // was read off 1.1.1.1 alone, and the line stayed "degraded" for as
        // long as the filter did.
        let store = Store::open_in_memory().unwrap();
        let t = store::now();
        let mut rows = Vec::new();
        for i in 1..=30 {
            rows.push((t - i as f64, "cloudflare".to_string(), None, false));
            rows.push((t - i as f64, "google".to_string(), Some(14.0), true));
        }
        store.add_samples(&rows).unwrap();

        let targets = vec![
            targets().remove(0),
            Resolved {
                key: "cloudflare".into(),
                host: Ipv4Addr::new(1, 1, 1, 1),
                scope: Scope::Internet,
            },
            Resolved {
                key: "google".into(),
                host: Ipv4Addr::new(8, 8, 8, 8),
                scope: Scope::Internet,
            },
        ];
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(false, None));
        r.insert("google".into(), sample(true, Some(14.0)));
        let s = Settings::default();
        let (status, note) =
            classify(&r, &targets, &wifi_state(), "", &s, &store, QUALITY_WINDOW_S);
        assert_eq!(status, Status::Ok, "{note}");

        // Loss on every public target is loss on the line.
        let mut lossy = Vec::new();
        for i in 1..=30 {
            let ok = i % 3 != 0;
            lossy.push((t - i as f64 - 0.5, "google".to_string(), ok.then_some(14.0), ok));
        }
        let store2 = Store::open_in_memory().unwrap();
        store2
            .add_samples(&rows.iter().filter(|r| r.1 == "cloudflare").cloned().collect::<Vec<_>>())
            .unwrap();
        store2.add_samples(&lossy).unwrap();
        let (status, _) = classify(&r, &targets, &wifi_state(), "", &s, &store2, QUALITY_WINDOW_S);
        assert_eq!(status, Status::Degraded);
    }

    #[test]
    fn jitter_past_the_setting_is_unstable_here_as_in_diagnose() {
        // 20 ms of jitter on both targets: past the 15 ms `jitter_ok_ms`
        // names and Diagnose warns at, under the 30 ms this used to wait for.
        let store = Store::open_in_memory().unwrap();
        let t = store::now();
        let mut rows = Vec::new();
        for i in 1..=30 {
            let rtt = if i % 2 == 0 { 10.0 } else { 30.0 };
            rows.push((t - i as f64, "cloudflare".to_string(), Some(rtt), true));
            rows.push((t - i as f64, "google".to_string(), Some(rtt), true));
        }
        store.add_samples(&rows).unwrap();
        let targets = vec![
            targets().remove(0),
            Resolved {
                key: "cloudflare".into(),
                host: Ipv4Addr::new(1, 1, 1, 1),
                scope: Scope::Internet,
            },
            Resolved {
                key: "google".into(),
                host: Ipv4Addr::new(8, 8, 8, 8),
                scope: Scope::Internet,
            },
        ];
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(true, Some(10.0)));
        r.insert("google".into(), sample(true, Some(10.0)));
        let s = Settings::default();
        let (status, note) =
            classify(&r, &targets, &wifi_state(), "", &s, &store, QUALITY_WINDOW_S);
        assert_eq!(status, Status::Degraded, "{note}");
    }

    #[test]
    fn the_cards_get_the_figures_the_headline_was_judged_on() {
        // 1.1.1.1 losing a third of its replies, 8.8.8.8 none: the line is
        // fine, and the loss the cards show must be the 0% the verdict used,
        // not 1.1.1.1's 33%.
        let store = Store::open_in_memory().unwrap();
        let t = store::now();
        let mut rows = Vec::new();
        for i in 1..=30 {
            let ok = i % 3 != 0;
            let jumpy = if i % 2 == 0 { 12.0 } else { 40.0 };
            rows.push((t - i as f64, "cloudflare".to_string(), ok.then_some(jumpy), ok));
            rows.push((t - i as f64, "google".to_string(), Some(14.0 + (i % 2) as f64), true));
        }
        store.add_samples(&rows).unwrap();
        let targets = vec![
            Resolved {
                key: "cloudflare".into(),
                host: Ipv4Addr::new(1, 1, 1, 1),
                scope: Scope::Internet,
            },
            Resolved {
                key: "google".into(),
                host: Ipv4Addr::new(8, 8, 8, 8),
                scope: Scope::Internet,
            },
        ];
        let q = line_quality(&targets, &store, QUALITY_WINDOW_S);
        assert_eq!(q.loss_pct, Some(0.0));
        assert_eq!(q.jitter_ms, Some(1.0), "the steadier target's jitter");
        assert_eq!(q.window_s, QUALITY_WINDOW_S);

        let empty = Store::open_in_memory().unwrap();
        let none = line_quality(&targets, &empty, QUALITY_WINDOW_S);
        assert_eq!((none.loss_pct, none.jitter_ms), (None, None), "too few readings is no figure");
    }

    #[test]
    fn too_few_readings_are_not_judged_for_loss() {
        let store = Store::open_in_memory().unwrap();
        let t = store::now();
        let rows = vec![
            (t - 3.0, "cloudflare".to_string(), Some(12.0), true),
            (t - 2.0, "cloudflare".to_string(), None, false),
            (t - 1.0, "cloudflare".to_string(), Some(12.0), true),
        ];
        store.add_samples(&rows).unwrap();
        let s = Settings::default();
        let status = classify(&line_up(), &targets(), &wifi_state(), "", &s, &store, 5.0).0;
        assert_eq!(status, Status::Ok, "one lost reply in three is not 33% loss");
    }

    #[test]
    fn isp_resolver_probe_is_skipped_when_it_is_the_router() {
        let net = NetState {
            gateway: Some(Ipv4Addr::new(192, 168, 50, 1)),
            dns_servers: vec![Ipv4Addr::new(192, 168, 50, 1)],
            ..Default::default()
        };
        let keys: Vec<String> = resolve_targets(&Settings::default(), &net, &HashMap::new())
            .iter()
            .map(|t| t.key.clone())
            .collect();
        assert!(keys.contains(&"gateway".to_string()));
        assert!(!keys.contains(&"dns_isp".to_string()));
    }

    #[test]
    fn a_separate_resolver_does_get_probed() {
        let net = NetState {
            gateway: Some(Ipv4Addr::new(192, 168, 50, 1)),
            dns_servers: vec![Ipv4Addr::new(192, 168, 50, 1), Ipv4Addr::new(1, 1, 1, 1)],
            ..Default::default()
        };
        let keys: Vec<String> = resolve_targets(&Settings::default(), &net, &HashMap::new())
            .iter()
            .map(|t| t.key.clone())
            .collect();
        assert!(keys.contains(&"dns_isp".to_string()));
    }

    #[test]
    fn the_most_frequent_scope_wins() {
        let scopes = ["internet", "isp", "isp", "internet", "isp"];
        assert_eq!(dominant_scope(scopes), Some("isp"));
    }

    #[test]
    fn a_tie_is_broken_by_severity_not_by_luck() {
        // Three of each, which is exactly the case that used to flicker.
        let scopes = ["internet", "isp", "internet", "isp", "internet", "isp"];
        assert_eq!(dominant_scope(scopes), Some("isp"));
        assert_eq!(dominant_scope(["internet", "lan"]), Some("lan"));
        assert_eq!(dominant_scope(["lan", "adapter"]), Some("adapter"));
        assert_eq!(dominant_scope(["dns", "isp"]), Some("isp"));
    }

    #[test]
    fn the_same_outages_always_give_the_same_answer() {
        // The headline is rebuilt every frame from a freshly built map. If the
        // result depended on iteration order it would change between frames,
        // which is what the user saw as flickering text. Insertion order must
        // not matter either.
        let a = ["internet", "isp", "dns", "isp", "internet", "dns"];
        let b = ["dns", "internet", "isp", "dns", "isp", "internet"];
        let first = dominant_scope(a);
        assert_eq!(first, dominant_scope(b));
        for _ in 0..200 {
            assert_eq!(dominant_scope(a), first);
        }
    }

    #[test]
    fn no_outages_name_no_scope() {
        assert_eq!(dominant_scope([]), None);
    }

    /// A flat series at `base` ms, with `spikes` mapping a sweep index to the
    /// value measured in it.
    fn flat(base: f64, len: usize, spikes: &[(usize, f64)]) -> Series {
        (0..len)
            .map(|i| {
                let v = spikes.iter().find(|(at, _)| *at == i).map(|(_, v)| *v).unwrap_or(base);
                (i as f64, Some(v))
            })
            .collect()
    }

    #[test]
    fn a_spike_on_one_target_alone_is_not_evidence() {
        // Exactly the commonest shape in the recorded data: one responder
        // takes 90 ms to answer while the others are untouched in the same
        // sweep. It cannot have happened on the shared path.
        let s = vec![flat(12.0, 40, &[(10, 90.0)]), flat(12.0, 40, &[]), flat(16.0, 40, &[])];
        let out = find_spikes(&s);
        assert!(out.correlated.is_empty());
        assert_eq!(out.single, 1);
    }

    #[test]
    fn a_spike_on_every_target_at_once_is() {
        let s = vec![
            flat(12.0, 40, &[(10, 90.0)]),
            flat(12.0, 40, &[(10, 75.0)]),
            flat(16.0, 40, &[(10, 88.0)]),
        ];
        let out = find_spikes(&s);
        assert_eq!(out.single, 0);
        assert_eq!(out.correlated.len(), 1);
        assert_eq!(out.correlated[0].1, 3, "all three series counted");
    }

    #[test]
    fn samples_from_one_sweep_group_despite_differing_timestamps() {
        // The targets are probed one after another, so a sweep's samples never
        // share an exact timestamp.
        let a: Series = vec![(10.02, Some(90.0)), (11.0, Some(12.0))];
        let b: Series = vec![(10.31, Some(80.0)), (11.3, Some(12.0))];
        let mut a_full = flat(12.0, 10, &[]);
        let mut b_full = flat(12.0, 10, &[]);
        a_full.extend(a);
        b_full.extend(b);
        let out = find_spikes(&[a_full, b_full]);
        assert_eq!(out.correlated.len(), 1);
        assert_eq!(out.single, 0);
    }

    #[test]
    fn the_bar_scales_with_what_the_line_normally_does() {
        // 30 ms is an event on a 4 ms link.
        let fast = find_spikes(&[flat(4.0, 40, &[(5, 30.0)]), flat(4.0, 40, &[(5, 30.0)])]);
        assert_eq!(fast.correlated.len(), 1);

        // The same 30 ms is ordinary on a link that normally sits at 90.
        let slow = find_spikes(&[flat(90.0, 40, &[(5, 120.0)]), flat(90.0, 40, &[(5, 120.0)])]);
        assert!(slow.correlated.is_empty());
        assert_eq!(slow.single, 0);
    }

    #[test]
    fn lost_packets_are_not_spikes() {
        let s: Series =
            (0..20).map(|i| (i as f64, if i == 5 { None } else { Some(12.0) })).collect();
        let out = find_spikes(&[s.clone(), s]);
        assert!(out.correlated.is_empty());
        assert_eq!(out.single, 0);
    }
}
