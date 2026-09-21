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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::i18n;
use crate::probe::icmp::{PingResult, Pinger};
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
fn context_json(snap: &Snapshot, lead: &VecDeque<LeadSample>, roamed_recently: bool) -> String {
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
        "dns_ms": snap.dns_ms,
        "dns_error": snap.dns_error,
        "roamed": roamed_recently,
        "lead_up": lead.iter().collect::<Vec<_>>(),
    })
    .to_string()
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
    pub paused: AtomicBool,
    pub settings: Mutex<Settings>,
    /// The per-hop picture, maintained by its own thread — see
    /// [`crate::probe::path`] and `run_path`.
    pub path: Mutex<PathReading>,
    /// The current default gateway, published for the path thread. It is read
    /// on the sweep cadence and changes when the machine moves between
    /// networks, which is exactly when the path has to be walked again.
    pub gateway: Mutex<Option<Ipv4Addr>>,
}

pub struct Monitor {
    pub shared: Arc<Shared>,
    pub rx: Receiver<Snapshot>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    path_handle: Option<thread::JoinHandle<()>>,
}

impl Monitor {
    pub fn start(store: Arc<Store>, settings: Settings) -> Monitor {
        let shared = Arc::new(Shared {
            last: Mutex::new(Snapshot::default()),
            paused: AtomicBool::new(false),
            settings: Mutex::new(settings),
            path: Mutex::new(PathReading::default()),
            gateway: Mutex::new(None),
        });
        let stop = Arc::new(AtomicBool::new(false));
        // Bounded so a stalled UI cannot grow the queue without limit; the
        // newest snapshot matters, older ones can be dropped.
        let (tx, rx) = bounded::<Snapshot>(64);

        let handle = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("netdoctor-monitor".into())
                .spawn(move || run_loop(shared, store, tx, stop))
                // ponytail: a thread spawn failing means the OS is out of
                // resources and nothing this app does next will work. Make
                // `new` fallible if it ever needs to degrade instead of die.
                .expect("spawn monitor thread")
        };

        // The path lives on its own thread rather than inside the sweep. A
        // walk is a dozen sequential probes and a hop that never answers
        // costs a full timeout, so folding it into the sweep would stall the
        // one measurement that has to keep its cadence to mean anything.
        let path_handle = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("netdoctor-path".into())
                .spawn(move || run_path(shared, stop))
                // ponytail: as above — unrecoverable, so it is not dressed up
                // as a recoverable error.
                .expect("spawn path thread")
        };

        Monitor { shared, rx, stop, handle: Some(handle), path_handle: Some(path_handle) }
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::Relaxed)
    }

    pub fn update_settings(&self, s: Settings) {
        *held(&self.shared.settings) = s;
    }

    /// The per-hop table and its verdict, as the path thread last left them.
    pub fn path(&self) -> PathReading {
        held(&self.shared.path).clone()
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in [self.handle.take(), self.path_handle.take()].into_iter().flatten() {
            let _ = h.join();
        }
    }
}

/// A target with its host resolved against the current network state.
struct Resolved {
    key: String,
    host: Ipv4Addr,
    scope: Scope,
}

fn resolve_targets(settings: &Settings, net: &NetState) -> Vec<Resolved> {
    let mut out = Vec::new();
    for t in settings.targets() {
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
        if shared.paused.load(Ordering::Relaxed) {
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
    fn sweep(&mut self, targets: &[Resolved], timeout_ms: u32) -> Vec<PingResult> {
        while self.handles.len() < targets.len() {
            match Pinger::new() {
                Ok(p) => self.handles.push(p),
                Err(_) => break,
            }
        }
        if self.handles.is_empty() {
            return Vec::new();
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

        out.into_iter().map(|r| r.unwrap_or_else(PingResult::timeout)).collect()
    }
}

fn run_loop(shared: Arc<Shared>, store: Arc<Store>, tx: Sender<Snapshot>, stop: Arc<AtomicBool>) {
    let mut pool = PingerPool::default();

    let mut net = netstate::read();
    let mut last_bssid = net.bssid.clone();
    let mut next_state_refresh = Instant::now();
    let mut sweep: u64 = 0;

    let mut fail_streak: u32 = 0;
    let mut open_event: Option<i64> = None;
    let mut lead: VecDeque<LeadSample> = VecDeque::with_capacity(LEAD_SWEEPS);
    let mut last_roam_ts: Option<f64> = None;

    let mut dns_ms: Option<f64> = None;
    let mut dns_error = String::new();

    // Continuity of observation. The first sweep asks the database whether it
    // is resuming a stretch or starting one; after that the loop is the only
    // writer of samples, so it can spot its own gaps -- a pause, a sleeping
    // machine -- without going back to SQLite every second.
    let mut observed_from = 0.0_f64;
    let mut prev_sweep_ts: Option<f64> = None;

    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        let settings = held(&shared.settings).clone();

        if shared.paused.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(200));
            continue;
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
        }

        if sweep % 10 == 0 {
            let (ms, err) = netstate::dns_lookup_ms(DNS_TEST_HOST);
            dns_ms = ms;
            dns_error = err;
        }

        let targets = resolve_targets(&settings, &net);
        let ts = store::now();
        let mut results = HashMap::new();
        let mut rows = Vec::new();

        // All at once, one handle each: a sweep costs one timeout rather than
        // one per target, so the cadence holds during an outage instead of
        // stretching to four seconds exactly when the samples matter.
        for (t, r) in targets.iter().zip(pool.sweep(&targets, settings.sweep_timeout_ms())) {
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
        let _ = store.add_samples(&rows);

        let roamed = !net.bssid.is_empty() && !last_bssid.is_empty() && net.bssid != last_bssid;
        if !net.bssid.is_empty() {
            last_bssid = net.bssid.clone();
        }
        if roamed {
            last_roam_ts = Some(ts);
        }

        let (status, note) = classify(&results, &targets, &net, &dns_error, &settings, &store);

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
        if bad {
            fail_streak += 1;
        } else {
            fail_streak = 0;
        }

        // A roam counts as "recent" for a minute either way; the disconnect it
        // causes usually lands a few sweeps after the BSSID actually changes.
        let roamed_recently = last_roam_ts.is_some_and(|t| ts - t <= 60.0);

        if bad && fail_streak >= settings.outage_after_fails && open_event.is_none() {
            let context = context_json(&snap, &lead, roamed_recently);
            open_event = store.open_event(status.key(), status.scope(), &snap.note, &context).ok();
        } else if !bad {
            if let Some(id) = open_event.take() {
                // An empty lead-up: what matters at recovery is the state the
                // connection came back into, not another copy of the history.
                let end = context_json(&snap, &VecDeque::new(), roamed_recently);
                let _ = store.close_event(id, &end);
            }
        }

        *held(&shared.last) = snap.clone();
        // A full channel means the UI is behind; dropping is correct here.
        let _ = tx.try_send(snap);

        sweep += 1;
        if sweep % 600 == 0 {
            let _ = store.prune(settings.keep_days);
        }

        let elapsed = started.elapsed();
        let interval = settings.interval();
        if interval > elapsed {
            // Wake early enough to notice a stop request promptly.
            let mut remaining = interval - elapsed;
            while remaining > Duration::ZERO && !stop.load(Ordering::Relaxed) {
                let step = remaining.min(Duration::from_millis(200));
                thread::sleep(step);
                remaining = remaining.saturating_sub(step);
            }
        }
    }

    // Shutting down while an outage is open: close it, but say nothing about a
    // recovery that never happened.
    if let Some(id) = open_event {
        let _ = store.close_event(id, "");
    }
}

/// The blame logic. Everything else in the app exists to support this.
fn classify(
    results: &HashMap<String, Sample>,
    targets: &[Resolved],
    net: &NetState,
    dns_error: &str,
    settings: &Settings,
    store: &Store,
) -> (Status, String) {
    let internet_ok = targets
        .iter()
        .filter(|t| matches!(t.scope, Scope::Internet | Scope::Isp))
        .any(|t| results.get(&t.key).map(|s| s.ok).unwrap_or(false));

    let gw = results.get("gateway").map(|s| s.ok);

    if internet_ok {
        if !dns_error.is_empty() {
            return (Status::DnsFail, i18n::mon_dns_detail(dns_error));
        }
        return quality_verdict(results, settings, store);
    }

    // Nothing on the internet answered. Who is still there?
    match gw {
        Some(true) => (
            Status::IspDown,
            i18n::mon_isp_detail(&net.gateway.map(|g| g.to_string()).unwrap_or_else(|| "?".into())),
        ),
        Some(false) => {
            if net.medium == Medium::Wifi && !netstate::wifi_associated() {
                (Status::AdapterDown, i18n::mon_wifi_deassociated().into())
            } else {
                (Status::LanDown, i18n::mon_nothing_responded().into())
            }
        }
        None => (Status::AdapterDown, i18n::mon_no_gateway().into()),
    }
}

/// The link is up; decide whether it is actually usable.
fn quality_verdict(
    results: &HashMap<String, Sample>,
    settings: &Settings,
    store: &Store,
) -> (Status, String) {
    let worst = results
        .iter()
        .filter(|(k, _)| k.as_str() == "cloudflare" || k.as_str() == "google")
        .filter_map(|(_, s)| s.rtt_ms)
        .fold(f64::NAN, f64::max);

    let stats = store.stats("cloudflare", 60.0);
    if stats.loss_pct > settings.loss_ok_pct {
        return (Status::Degraded, i18n::mon_loss_detail(stats.loss_pct));
    }
    if let Some(j) = stats.jitter {
        if j > settings.jitter_ok_ms * 2.0 {
            return (Status::Degraded, i18n::mon_jitter_detail(j));
        }
    }
    if worst.is_finite() && worst > settings.ping_bad_ms {
        return (Status::Degraded, i18n::mon_ping_detail(worst));
    }
    (Status::Ok, String::new())
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
        let results = pool.sweep(&targets, timeout);
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
        let results = pool.sweep(&targets, 300);

        assert_eq!(results.len(), 5);
        for (t, r) in targets.iter().zip(&results) {
            match t.key.as_str() {
                "loopback" => assert!(r.ok(), "this machine answers itself"),
                _ => assert!(!r.ok(), "{} is a reserved address and must not reply", t.key),
            }
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

    #[test]
    fn everything_up_is_ok() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(true, Some(12.0)));
        let (status, _) = classify(&r, &targets(), &wifi_state(), "", &Settings::default(), &store);
        assert_eq!(status, Status::Ok);
    }

    #[test]
    fn router_up_internet_down_blames_the_isp() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(true, Some(2.0)));
        r.insert("cloudflare".into(), sample(false, None));
        let (status, note) =
            classify(&r, &targets(), &wifi_state(), "", &Settings::default(), &store);
        assert_eq!(status, Status::IspDown);
        assert!(note.contains("Router"));
    }

    #[test]
    fn nothing_answering_blames_the_local_link() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("gateway".into(), sample(false, None));
        r.insert("cloudflare".into(), sample(false, None));
        let (status, _) = classify(&r, &targets(), &wifi_state(), "", &Settings::default(), &store);
        assert_eq!(status, Status::LanDown);
    }

    #[test]
    fn missing_gateway_means_the_adapter_is_down() {
        let store = Store::open_in_memory().unwrap();
        let mut r = HashMap::new();
        r.insert("cloudflare".into(), sample(false, None));
        let net = NetState { gateway: None, ..Default::default() };
        let (status, _) = classify(&r, &targets(), &net, "", &Settings::default(), &store);
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
        let (status, note) =
            classify(&r, &targets(), &wifi_state(), "", &Settings::default(), &store);
        assert_eq!(status, Status::Degraded);
        assert!(note.contains(if crate::i18n::current() == crate::i18n::Lang::Pl {
            "utraconych pakietów"
        } else {
            "packet loss"
        }));
    }

    #[test]
    fn isp_resolver_probe_is_skipped_when_it_is_the_router() {
        let net = NetState {
            gateway: Some(Ipv4Addr::new(192, 168, 50, 1)),
            dns_servers: vec![Ipv4Addr::new(192, 168, 50, 1)],
            ..Default::default()
        };
        let keys: Vec<String> =
            resolve_targets(&Settings::default(), &net).iter().map(|t| t.key.clone()).collect();
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
        let keys: Vec<String> =
            resolve_targets(&Settings::default(), &net).iter().map(|t| t.key.clone()).collect();
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
