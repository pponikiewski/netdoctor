//! Background monitor.
//!
//! Pings the router and several internet endpoints in parallel, once per
//! interval, and turns the *pattern* of failures into a verdict about where
//! the connection broke. That distinction is the whole point of the tool: a
//! dropout looks identical from inside Windows whether the Wi-Fi card fell
//! asleep or the ISP went down, and the two are fixed in completely different
//! ways.

use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::i18n;
use crate::probe::icmp::{PingResult, Pinger};
use crate::probe::netstate::{self, Medium, NetState};
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
        }
    }
}

/// Ring buffer of (timestamp, rtt) per target, for the live chart.
pub type Series = Vec<(f64, Option<f64>)>;

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

pub struct Shared {
    pub history: Mutex<HashMap<String, Series>>,
    pub last: Mutex<Snapshot>,
    pub paused: AtomicBool,
    pub settings: Mutex<Settings>,
}

pub struct Monitor {
    pub shared: Arc<Shared>,
    pub rx: Receiver<Snapshot>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Monitor {
    pub fn start(store: Arc<Store>, settings: Settings) -> Monitor {
        let shared = Arc::new(Shared {
            history: Mutex::new(HashMap::new()),
            last: Mutex::new(Snapshot::default()),
            paused: AtomicBool::new(false),
            settings: Mutex::new(settings),
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
                .expect("spawn monitor thread")
        };

        Monitor { shared, rx, stop, handle: Some(handle) }
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::Relaxed)
    }

    pub fn update_settings(&self, s: Settings) {
        *self.shared.settings.lock().unwrap() = s;
        // Targets may have gone away; drop series that no longer exist.
        let keep: Vec<String> = self
            .shared
            .settings
            .lock()
            .unwrap()
            .targets()
            .iter()
            .map(|t| t.key.clone())
            .collect();
        self.shared.history.lock().unwrap().retain(|k, _| keep.contains(k));
    }

    pub fn series(&self, key: &str) -> Series {
        self.shared
            .history
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .unwrap_or_default()
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
                net.dns_servers
                    .iter()
                    .find(|d| Some(**d) != net.gateway)
                    .copied()
            }
            _ => t.host,
        };
        if let Some(host) = host {
            out.push(Resolved { key: t.key, host, scope: t.scope });
        }
    }
    out
}

fn run_loop(
    shared: Arc<Shared>,
    store: Arc<Store>,
    tx: Sender<Snapshot>,
    stop: Arc<AtomicBool>,
) {
    let pinger = match Pinger::new() {
        Ok(p) => p,
        Err(_) => return,
    };

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

    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        let settings = shared.settings.lock().unwrap().clone();

        if shared.paused.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(200));
            continue;
        }

        // Adapter state changes slowly and costs more to read than the pings,
        // so it is refreshed on its own slower cadence.
        if Instant::now() >= next_state_refresh {
            let fresh = netstate::read();
            if fresh.gateway.is_some() || !fresh.adapter_name.is_empty() {
                net = fresh;
            }
            next_state_refresh = Instant::now() + Duration::from_secs(5);
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

        // Sequential on one handle: with four targets and sub-20 ms replies
        // the whole sweep costs well under a tenth of the interval, and a
        // shared handle avoids spawning threads every second.
        for t in &targets {
            let r: PingResult = pinger.ping(t.host, settings.ping_timeout_ms);
            let sample = Sample {
                ok: r.ok(),
                rtt_ms: r.rtt_ms,
                error: r.error.as_ref().map(|e| e.describe()),
            };
            rows.push((ts, t.key.clone(), sample.rtt_ms, sample.ok));
            results.insert(t.key.clone(), sample);
        }

        {
            let mut hist = shared.history.lock().unwrap();
            for t in &targets {
                let entry = hist.entry(t.key.clone()).or_default();
                let rtt = results.get(&t.key).and_then(|s| s.rtt_ms);
                entry.push((ts, rtt));
                let limit = settings.history_points.max(30);
                if entry.len() > limit {
                    let excess = entry.len() - limit;
                    entry.drain(0..excess);
                }
            }
        }

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
            open_event = store
                .open_event(status.key(), status.scope(), &snap.note, &context)
                .ok();
        } else if !bad {
            if let Some(id) = open_event.take() {
                // An empty lead-up: what matters at recovery is the state the
                // connection came back into, not another copy of the history.
                let end = context_json(&snap, &VecDeque::new(), roamed_recently);
                let _ = store.close_event(id, &end);
            }
        }

        *shared.last.lock().unwrap() = snap.clone();
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
            return (
                Status::DnsFail,
                i18n::mon_dns_detail(dns_error),
            );
        }
        return quality_verdict(results, settings, store);
    }

    // Nothing on the internet answered. Who is still there?
    match gw {
        Some(true) => (
            Status::IspDown,
            i18n::mon_isp_detail(
                &net.gateway.map(|g| g.to_string()).unwrap_or_else(|| "?".into()),
            ),
        ),
        Some(false) => {
            if net.medium == Medium::Wifi && !netstate::wifi_associated() {
                (
                    Status::AdapterDown,
                    i18n::mon_wifi_deassociated().into(),
                )
            } else {
                (Status::LanDown, i18n::mon_nothing_responded().into())
            }
        }
        None => (
            Status::AdapterDown,
            i18n::mon_no_gateway().into(),
        ),
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
        return (
            Status::Degraded,
            i18n::mon_loss_detail(stats.loss_pct),
        );
    }
    if let Some(j) = stats.jitter {
        if j > settings.jitter_ok_ms * 2.0 {
            return (
                Status::Degraded,
                i18n::mon_jitter_detail(j),
            );
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
        let (status, _) =
            classify(&r, &targets(), &wifi_state(), "", &Settings::default(), &store);
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
        let (status, _) =
            classify(&r, &targets(), &wifi_state(), "", &Settings::default(), &store);
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
}
