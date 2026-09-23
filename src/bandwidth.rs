//! Bufferbloat test: latency while the link is saturated.
//!
//! An idle ping of 12 ms means nothing if it jumps to 300 ms the moment
//! somebody starts a download. That jump is bufferbloat — oversized buffers in
//! the router or at the ISP queueing packets instead of dropping them — and it
//! is the usual reason a game lags "even though the ping is fine".
//!
//! Both directions are loaded in turn. On an asymmetric line the upload
//! queue is usually the worse one, and it is the one a video call or a
//! backup to the cloud fills.

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
/// Takes a body of any length and throws it away.
const UP_URL: &str = "https://speed.cloudflare.com/__up";
/// Bytes per upload request. Each stream posts one after another, like the
/// download pulls one 25 MB payload after another.
const UP_CHUNK: u64 = 10_000_000;
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
            Grade::A => crate::i18n::grade_a(),
            Grade::B => crate::i18n::grade_b(),
            Grade::C => crate::i18n::grade_c(),
            Grade::D => crate::i18n::grade_d(),
            Grade::F => crate::i18n::grade_f(),
            Grade::Unknown => crate::i18n::grade_unknown(),
        }
    }

    /// How bad, for picking the worse of two directions. `Unknown` ranks
    /// below `A`: a direction that was not measured cannot make the other
    /// one look worse.
    fn rank(&self) -> u8 {
        match self {
            Grade::Unknown => 0,
            Grade::A => 1,
            Grade::B => 2,
            Grade::C => 3,
            Grade::D => 4,
            Grade::F => 5,
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
    /// How many of the `STREAMS` download threads were still pulling when the
    /// measurement ended. A grade read off half the intended load is not the
    /// grade of a saturated line, so the number has to travel with it.
    pub streams_alive: usize,
    /// Bytes pulled during the test, so the cost of running it is visible
    /// rather than implied.
    pub bytes: u64,
    /// The same measurement with the line loaded the other way. `None` when
    /// the test never got that far. The fields above are the download, and
    /// `grade` is the worse of the two directions.
    pub upload: Option<Upload>,
}

/// Latency while this machine sends as fast as it can.
#[derive(Debug, Clone, Default)]
pub struct Upload {
    pub loaded_avg: Option<f64>,
    pub loaded_max: Option<f64>,
    pub loaded_loss_pct: f64,
    pub bump_ms: Option<f64>,
    pub mbps: Option<f64>,
    /// `None` when no upload stream held, which is "not measured", not a pass.
    pub grade: Option<Grade>,
    pub bytes: u64,
    /// Why the grade is missing or not to be taken at face value.
    pub note: String,
}

impl BloatResult {
    pub fn grade_or_unknown(&self) -> Grade {
        self.grade.unwrap_or(Grade::Unknown)
    }

    /// The larger rise of the two directions: the one the grade came from.
    pub fn worst_bump(&self) -> Option<f64> {
        let up = self.upload.as_ref().and_then(|u| u.bump_ms);
        match (self.bump_ms, up) {
            (Some(d), Some(u)) => Some(d.max(u)),
            (d, u) => d.or(u),
        }
    }

    /// Everything the test moved, both ways.
    pub fn total_bytes(&self) -> u64 {
        self.bytes + self.upload.as_ref().map_or(0, |u| u.bytes)
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
///
/// Returns true if it was still pulling when it was asked to stop. A stream
/// that died early leaves the line less than saturated, and a bufferbloat
/// grade measured under partial load flatters the connection — so the caller
/// needs to know, rather than reading a confident A off a quarter of the
/// intended traffic.
fn download(stop: Arc<AtomicBool>, counter: Arc<AtomicU64>) -> bool {
    while !stop.load(Ordering::Relaxed) {
        let resp = match ureq::get(LOAD_URL).timeout(Duration::from_secs(20)).call() {
            Ok(r) => r,
            Err(_) => return false,
        };
        let mut reader = resp.into_reader();
        let mut buf = [0u8; 65536];
        loop {
            if stop.load(Ordering::Relaxed) {
                return true;
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    counter.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => return false,
            }
        }
    }
    true
}

/// Hands over zeros until the chunk is spent or the test is stopped,
/// counting what it gave. A stop ends the body early, which is a normal end
/// of a chunked upload rather than an error.
struct Pusher {
    stop: Arc<AtomicBool>,
    counter: Arc<AtomicU64>,
    left: u64,
}

impl Read for Pusher {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 || self.stop.load(Ordering::Relaxed) {
            return Ok(0);
        }
        let n = buf.len().min(self.left as usize);
        buf[..n].fill(0);
        self.left -= n as u64;
        self.counter.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Pushes bytes until told to stop. Returns true if it was still pushing
/// then, for the same reason as [`download`].
fn upload(stop: Arc<AtomicBool>, counter: Arc<AtomicU64>) -> bool {
    while !stop.load(Ordering::Relaxed) {
        let body =
            Pusher { stop: Arc::clone(&stop), counter: Arc::clone(&counter), left: UP_CHUNK };
        if ureq::post(UP_URL).timeout(Duration::from_secs(20)).send(body).is_err() {
            // A request cut short by the stop is the test ending, not a
            // stream dying.
            return stop.load(Ordering::Relaxed);
        }
    }
    true
}

/// One direction of load: the pings taken while `work` ran on every stream.
struct Loaded {
    samples: Vec<Option<f64>>,
    mbps: Option<f64>,
    alive: usize,
    bytes: u64,
}

fn under_load(
    host: Ipv4Addr,
    load: Duration,
    timeout_ms: u32,
    work: fn(Arc<AtomicBool>, Arc<AtomicU64>) -> bool,
) -> Loaded {
    let stop = Arc::new(AtomicBool::new(false));
    let counter = Arc::new(AtomicU64::new(0));
    let workers: Vec<_> = (0..STREAMS)
        .map(|_| {
            let stop = Arc::clone(&stop);
            let counter = Arc::clone(&counter);
            thread::spawn(move || work(stop, counter))
        })
        .collect();

    // Let the streams ramp up before the buffers start to matter.
    thread::sleep(Duration::from_millis(1500));
    let started = Instant::now();
    let start_bytes = counter.load(Ordering::Relaxed);

    let samples = ping_window(host, load, timeout_ms);

    let elapsed = started.elapsed().as_secs_f64();
    let moved = counter.load(Ordering::Relaxed).saturating_sub(start_bytes);
    stop.store(true, Ordering::Relaxed);
    // Each worker reports whether it was still going when it was stopped; a
    // thread that panicked counts as dead rather than as load.
    let alive = workers
        .into_iter()
        .filter_map(|w| w.join().ok())
        .filter(|still_going| *still_going)
        .count();
    let mbps = (elapsed > 0.0 && moved > 0).then(|| moved as f64 * 8.0 / elapsed / 1_000_000.0);
    Loaded { samples, mbps, alive, bytes: counter.load(Ordering::Relaxed) }
}

/// The upload half, judged against the same idle baseline as the download.
fn judge_upload(up: Loaded, idle_avg: f64) -> Upload {
    let mut res = Upload { mbps: up.mbps, bytes: up.bytes, ..Default::default() };
    if up.alive == 0 {
        res.note = crate::i18n::bloat_no_upload().into();
        return res;
    }
    let rtts: Vec<f64> = up.samples.iter().flatten().copied().collect();
    let stats = store::summarise(up.samples.len(), &rtts);
    res.loaded_loss_pct = stats.loss_pct;
    if rtts.is_empty() {
        res.grade = Some(Grade::F);
        res.note = crate::i18n::bloat_silent_under_load().into();
        return res;
    }
    res.loaded_avg = stats.avg;
    res.loaded_max = stats.max;
    let bump = stats.avg.unwrap_or(0.0) - idle_avg;
    res.bump_ms = Some(bump);
    res.grade = Some(Grade::from_bump(bump));
    if up.alive < STREAMS {
        res.note = crate::i18n::bloat_partial_load(up.alive, STREAMS);
    }
    res
}

/// Measure latency before and during saturation, downloading and then
/// uploading. `load` is how long each direction is held.
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

    say(crate::i18n::bloat_prog_idle(), 0.05);
    let idle_samples = ping_window(host, idle, timeout_ms);
    let idle_rtts: Vec<f64> = idle_samples.iter().flatten().copied().collect();
    if idle_rtts.is_empty() {
        res.error = crate::i18n::bloat_no_reply(&host.to_string());
        return res;
    }
    let idle_stats = store::summarise(idle_samples.len(), &idle_rtts);
    res.idle_avg = idle_stats.avg;
    res.idle_max = idle_stats.max;

    say(crate::i18n::bloat_prog_load(), 0.3);
    let down = under_load(host, load, timeout_ms, download);
    res.streams_alive = down.alive;
    res.bytes = down.bytes;
    res.mbps = down.mbps;
    let loaded_samples = down.samples;

    // No load means no test. Reporting a grade here would present the absence
    // of a measurement as a pass — and "silent under load" below would blame
    // the line for going quiet under traffic that never arrived.
    if res.streams_alive == 0 {
        res.grade = Some(Grade::Unknown);
        res.error = crate::i18n::bloat_no_load_str().into();
        return res;
    }

    let loaded_rtts: Vec<f64> = loaded_samples.iter().flatten().copied().collect();
    let loaded_stats = store::summarise(loaded_samples.len(), &loaded_rtts);
    res.loaded_loss_pct = loaded_stats.loss_pct;

    if loaded_rtts.is_empty() {
        res.grade = Some(Grade::F);
        res.error = crate::i18n::bloat_silent_under_load().into();
        return res;
    }

    res.loaded_avg = loaded_stats.avg;
    res.loaded_max = loaded_stats.max;
    res.bump_ms = Some(loaded_stats.avg.unwrap_or(0.0) - idle_stats.avg.unwrap_or(0.0));
    res.grade = Some(Grade::from_bump(res.bump_ms.unwrap_or(0.0)));
    if res.streams_alive < STREAMS {
        res.error = crate::i18n::bloat_partial_load(res.streams_alive, STREAMS);
    }

    say(crate::i18n::bloat_prog_upload(), 0.65);
    let up =
        judge_upload(under_load(host, load, timeout_ms, upload), idle_stats.avg.unwrap_or(0.0));
    if let Some(g) = up.grade.filter(|g| g.rank() > res.grade_or_unknown().rank()) {
        res.grade = Some(g);
    }
    res.upload = Some(up);

    say(crate::i18n::bloat_prog_done(), 1.0);
    res
}

/// What to actually do about the result.
pub fn advice(res: &BloatResult) -> String {
    match res.grade_or_unknown() {
        Grade::Unknown => {
            if res.error.is_empty() {
                crate::i18n::bloat_advice_run().into()
            } else {
                res.error.clone()
            }
        }
        Grade::A | Grade::B => {
            let mut text = crate::i18n::bloat_advice_ok().to_string();
            // A good grade is only as good as the load behind it: a server
            // that tops out at 100 Mbps leaves a gigabit line idle and its
            // queue empty. The app cannot know the plan, the user does.
            if let Some(down) = res.mbps {
                let up = res.upload.as_ref().and_then(|u| u.mbps);
                text.push_str("\n\n");
                text.push_str(&crate::i18n::bloat_saturation_caveat(down, up));
            }
            text
        }
        _ => {
            let mut lines = vec![
                crate::i18n::bloat_advice_intro(res.worst_bump().unwrap_or(0.0)),
                String::new(),
                crate::i18n::bloat_advice_header().into(),
                crate::i18n::bloat_advice_1().into(),
                crate::i18n::bloat_advice_2().into(),
                crate::i18n::bloat_advice_3().into(),
            ];
            if let Some(mbps) = res.mbps {
                lines.push(String::new());
                lines.push(crate::i18n::bloat_advice_throughput(mbps));
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
        let _guard = crate::i18n::test_lock();
        let res = BloatResult { grade: Some(Grade::A), ..Default::default() };
        let text = advice(&res);
        assert_eq!(text, crate::i18n::bloat_advice_ok());
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
    fn the_worse_direction_sets_the_grade_and_an_unmeasured_one_does_not() {
        assert!(Grade::F.rank() > Grade::A.rank());
        assert!(Grade::Unknown.rank() < Grade::A.rank());
        let idle = 20.0;
        let loaded = |rtt: f64, alive| Loaded {
            samples: vec![Some(rtt); 40],
            mbps: Some(20.0),
            alive,
            bytes: 1,
        };
        let up = judge_upload(loaded(320.0, STREAMS), idle);
        assert_eq!(up.grade, Some(Grade::D));
        assert_eq!(up.bump_ms, Some(300.0));
        // No stream held: not measured, and not a pass either.
        let none = judge_upload(loaded(20.0, 0), idle);
        assert_eq!(none.grade, None);
        assert!(!none.note.is_empty());

        let res = BloatResult { bump_ms: Some(10.0), upload: Some(up), ..Default::default() };
        assert_eq!(res.worst_bump(), Some(300.0));
    }

    #[test]
    fn a_good_grade_carries_the_load_it_was_measured_at() {
        let _guard = crate::i18n::test_lock();
        let res = BloatResult {
            grade: Some(Grade::A),
            mbps: Some(94.0),
            upload: Some(Upload { mbps: Some(11.0), ..Default::default() }),
            ..Default::default()
        };
        let text = advice(&res);
        assert!(text.contains("94") && text.contains("11"), "{text}");
    }

    #[test]
    fn an_unrun_test_says_so_rather_than_claiming_success() {
        let res = BloatResult::default();
        assert_eq!(res.grade_or_unknown(), Grade::Unknown);
        let _guard = crate::i18n::test_lock();
        assert_eq!(advice(&res), crate::i18n::bloat_advice_run());
    }
}
