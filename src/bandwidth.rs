//! Bufferbloat test: latency while the link is saturated.
//!
//! An idle ping of 12 ms means nothing if it jumps to 300 ms the moment
//! somebody starts a download. That jump is bufferbloat — oversized buffers in
//! the router or at the ISP queueing packets instead of dropping them — and it
//! is the usual reason a game lags "even though the ping is fine".

use std::io::Read;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::probe::icmp::Pinger;
use crate::store;

/// Cloudflare's speed-test endpoint serves an arbitrary-length payload.
const LOAD_URL: &str = "https://speed.cloudflare.com/__down?bytes=25000000";
const STREAMS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    A,
    B,
    C,
    D,
    F,
    Unknown,
}

impl Grade {
    pub fn letter(&self) -> &'static str {
        match self {
            Grade::A => "A",
            Grade::B => "B",
            Grade::C => "C",
            Grade::D => "D",
            Grade::F => "F",
            Grade::Unknown => "?",
        }
    }

    pub fn verdict(&self) -> &'static str {
        match self {
            Grade::A => "Excellent — the link does not bloat under load.",
            Grade::B => "Good — a mild rise, unnoticeable in games.",
            Grade::C => "Fair — latency climbs noticeably while downloading.",
            Grade::D => "Poor — games will stutter during any download.",
            Grade::F => "Very poor — textbook bufferbloat.",
            Grade::Unknown => "Not measured.",
        }
    }

    fn from_bump(bump_ms: f64) -> Grade {
        match bump_ms {
            b if b < 25.0 => Grade::A,
            b if b < 60.0 => Grade::B,
            b if b < 150.0 => Grade::C,
            b if b < 400.0 => Grade::D,
            _ => Grade::F,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BloatResult {
    pub idle_avg: Option<f64>,
    pub idle_max: Option<f64>,
    pub loaded_avg: Option<f64>,
    pub loaded_max: Option<f64>,
    pub loaded_loss_pct: f64,
    pub bump_ms: Option<f64>,
    pub mbps: Option<f64>,
    pub grade: Option<Grade>,
    pub error: String,
}

impl BloatResult {
    pub fn grade_or_unknown(&self) -> Grade {
        self.grade.unwrap_or(Grade::Unknown)
    }
}

pub type Progress = Arc<dyn Fn(&str, f32) + Send + Sync>;

fn ping_window(host: Ipv4Addr, duration: Duration, timeout_ms: u32) -> Vec<Option<f64>> {
    let Ok(pinger) = Pinger::new() else {
        return Vec::new();
    };
    let deadline = Instant::now() + duration;
    let mut out = Vec::new();
    while Instant::now() < deadline {
        out.push(pinger.ping(host, timeout_ms).rtt_ms);
        // Without a gap the probes themselves become the load.
        thread::sleep(Duration::from_millis(50));
    }
    out
}

/// Pulls bytes until told to stop, counting what arrived.
fn download(stop: Arc<AtomicBool>, counter: Arc<AtomicU64>) {
    while !stop.load(Ordering::Relaxed) {
        let resp = match ureq::get(LOAD_URL).timeout(Duration::from_secs(20)).call() {
            Ok(r) => r,
            // One stream failing is fine as long as the others carry load.
            Err(_) => return,
        };
        let mut reader = resp.into_reader();
        let mut buf = [0u8; 65536];
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    counter.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => return,
            }
        }
    }
}

/// Measure latency before and during saturation.
pub fn run(
    host: Ipv4Addr,
    idle: Duration,
    load: Duration,
    timeout_ms: u32,
    progress: Option<Progress>,
) -> BloatResult {
    let mut res = BloatResult::default();
    let say = |text: &str, frac: f32| {
        if let Some(p) = &progress {
            p(text, frac);
        }
    };

    say("Measuring idle latency…", 0.05);
    let idle_samples = ping_window(host, idle, timeout_ms);
    let idle_rtts: Vec<f64> = idle_samples.iter().flatten().copied().collect();
    if idle_rtts.is_empty() {
        res.error = format!("No reply from {host} — test aborted.");
        return res;
    }
    let idle_stats = store::summarise(idle_samples.len(), &idle_rtts);
    res.idle_avg = idle_stats.avg;
    res.idle_max = idle_stats.max;

    say("Saturating the link and measuring latency under load…", 0.4);
    let stop = Arc::new(AtomicBool::new(false));
    let counter = Arc::new(AtomicU64::new(0));
    let workers: Vec<_> = (0..STREAMS)
        .map(|_| {
            let stop = Arc::clone(&stop);
            let counter = Arc::clone(&counter);
            thread::spawn(move || download(stop, counter))
        })
        .collect();

    // Let the streams ramp up before the buffers start to matter.
    thread::sleep(Duration::from_millis(1500));
    let started = Instant::now();
    let start_bytes = counter.load(Ordering::Relaxed);

    let loaded_samples = ping_window(host, load, timeout_ms);

    let elapsed = started.elapsed().as_secs_f64();
    let moved = counter.load(Ordering::Relaxed).saturating_sub(start_bytes);
    stop.store(true, Ordering::Relaxed);
    for w in workers {
        let _ = w.join();
    }

    if elapsed > 0.0 && moved > 0 {
        res.mbps = Some(moved as f64 * 8.0 / elapsed / 1_000_000.0);
    }

    let loaded_rtts: Vec<f64> = loaded_samples.iter().flatten().copied().collect();
    let loaded_stats = store::summarise(loaded_samples.len(), &loaded_rtts);
    res.loaded_loss_pct = loaded_stats.loss_pct;

    if loaded_rtts.is_empty() {
        res.grade = Some(Grade::F);
        res.error =
            "Under load the host stopped replying entirely — that is itself the result.".into();
        return res;
    }

    res.loaded_avg = loaded_stats.avg;
    res.loaded_max = loaded_stats.max;
    res.bump_ms = Some(loaded_stats.avg.unwrap_or(0.0) - idle_stats.avg.unwrap_or(0.0));
    res.grade = Some(Grade::from_bump(res.bump_ms.unwrap_or(0.0)));

    say("Done", 1.0);
    res
}

/// What to actually do about the result.
pub fn advice(res: &BloatResult) -> String {
    match res.grade_or_unknown() {
        Grade::Unknown => {
            if res.error.is_empty() {
                "Run the test to get a result.".into()
            } else {
                res.error.clone()
            }
        }
        Grade::A | Grade::B => "Nothing to do — the link copes with load. If you still get lag, \
             the cause is elsewhere (Wi-Fi, driver, or the route to that particular server)."
            .into(),
        _ => {
            let mut lines = vec![
                format!(
                    "Latency rises by {:.0} ms when the link is busy, which means packets are \
                     queueing — either in your router or at the ISP.",
                    res.bump_ms.unwrap_or(0.0)
                ),
                String::new(),
                "What helps, most effective first:".into(),
                "1. Enable SQM / Smart Queue / QoS on the router (look for \"cake\" or \
                 \"fq_codel\") and cap it at about 90% of the real line speed."
                    .into(),
                "2. If the router has no such option, that is the best possible reason to replace \
                 it. No Windows setting can fix this."
                    .into(),
                "3. As a stopgap, throttle whatever saturates the link (Steam, torrents, updates) \
                 to about 80% of capacity."
                    .into(),
            ];
            if let Some(mbps) = res.mbps {
                lines.push(String::new());
                lines.push(format!(
                    "Throughput measured during the test: {mbps:.0} Mbps — use that to set the cap."
                ));
            }
            lines.join("\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_follow_the_latency_bump() {
        assert_eq!(Grade::from_bump(10.0), Grade::A);
        assert_eq!(Grade::from_bump(40.0), Grade::B);
        assert_eq!(Grade::from_bump(100.0), Grade::C);
        assert_eq!(Grade::from_bump(300.0), Grade::D);
        assert_eq!(Grade::from_bump(900.0), Grade::F);
    }

    #[test]
    fn good_grades_do_not_produce_a_wall_of_advice() {
        let res = BloatResult { grade: Some(Grade::A), ..Default::default() };
        let text = advice(&res);
        assert!(text.contains("Nothing to do"));
        assert!(!text.contains("SQM"));
    }

    #[test]
    fn bad_grades_name_the_actual_fix() {
        let res = BloatResult {
            grade: Some(Grade::F),
            bump_ms: Some(500.0),
            mbps: Some(78.0),
            ..Default::default()
        };
        let text = advice(&res);
        assert!(text.contains("SQM"));
        assert!(text.contains("78"), "throughput should feed the cap suggestion");
    }

    #[test]
    fn an_unrun_test_says_so_rather_than_claiming_success() {
        let res = BloatResult::default();
        assert_eq!(res.grade_or_unknown(), Grade::Unknown);
        assert!(advice(&res).contains("Run the test"));
    }
}
