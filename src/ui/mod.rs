//! egui front end.
//!
//! The monitor thread owns the probing and pushes snapshots down a channel;
//! the UI drains it each frame. No widget is ever touched from a worker.

mod bloat;
mod diag;
mod history;
mod live;
mod opt;
mod report;
mod settings_tab;
mod summary;
mod update_ui;
mod widgets;

pub use widgets::*;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::bandwidth::BloatResult;
use crate::diagnose::{Finding, Measurements, Scan, Verdict};
use crate::monitor::{Monitor, Snapshot, Status};
use crate::probe::netstate::NetState;
use crate::settings::Settings;
use crate::store::Store;

/// Where a window started minimised spends its first frame. See `main`.
pub const OFF_SCREEN: [f32; 2] = [-32000.0, -32000.0];

/// Where a window that started off screen belongs: centred on the monitor, or
/// near the corner when the monitor's size is not known yet.
fn on_screen(ctx: &egui::Context) -> egui::Pos2 {
    let (monitor, outer) =
        ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect.map(|r| r.size())));
    match (monitor, outer) {
        (Some(m), Some(o)) => {
            egui::pos2(((m.x - o.x) / 2.0).max(0.0), ((m.y - o.y) / 2.0).max(0.0))
        }
        _ => egui::pos2(80.0, 80.0),
    }
}

// palette
pub const BG: egui::Color32 = egui::Color32::from_rgb(0x14, 0x16, 0x1a);
pub const BG2: egui::Color32 = egui::Color32::from_rgb(0x1c, 0x1f, 0x26);
pub const BG3: egui::Color32 = egui::Color32::from_rgb(0x24, 0x28, 0x32);
pub const FG: egui::Color32 = egui::Color32::from_rgb(0xe6, 0xe8, 0xed);
/// Secondary text. Lifted from `#8b93a3`, which cleared 4.5:1 against the
/// page but only just against `BG3` — and most of what it labels is set in
/// the two smallest steps of the scale, where "only just" is not enough.
pub const FG_DIM: egui::Color32 = egui::Color32::from_rgb(0x9a, 0xa3, 0xb4);
/// A hairline. Dark enough to read as a rule rather than as another panel.
pub const LINE: egui::Color32 = egui::Color32::from_rgb(0x2c, 0x31, 0x3c);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4d, 0xa3, 0xff);
pub const GREEN: egui::Color32 = egui::Color32::from_rgb(0x3d, 0xdc, 0x84);
pub const YELLOW: egui::Color32 = egui::Color32::from_rgb(0xff, 0xc4, 0x4d);
pub const RED: egui::Color32 = egui::Color32::from_rgb(0xff, 0x5f, 0x5f);

pub const SERIES_COLOURS: [egui::Color32; 6] = [
    egui::Color32::from_rgb(0x7b, 0xd8, 0x8f),
    ACCENT,
    egui::Color32::from_rgb(0xb0, 0x7b, 0xff),
    egui::Color32::from_rgb(0xff, 0xa9, 0x4d),
    egui::Color32::from_rgb(0x4d, 0xd0, 0xe1),
    egui::Color32::from_rgb(0xff, 0x8f, 0xab),
];

// ---------------------------------------------------------------------------
// Type scale
// ---------------------------------------------------------------------------

// Six steps at roughly 1.13 between them, which is the tight ratio a dense
// tool wants: there are a lot of text elements per screen here and a wide
// ratio turns hierarchy into noise. This replaces ten ad-hoc sizes that had
// grown to sit one point apart in places, where the difference read as a
// rendering fault rather than as a level.
//
// The floor moved up. The old scale bottomed out at 10pt for hints and units
// and did most of its work at 11pt, which is small enough to be skipped —
// and what it was labelling was the units and the caveats, the parts that
// decide whether a number means anything.

/// Units, timestamps, caveats. Never a whole sentence anyone must read.
pub const T_MICRO: f32 = 11.0;
/// Secondary labels and state text, next to the thing they qualify.
pub const T_META: f32 = 12.0;
/// The default. Anything meant to be read as prose is at least this size.
pub const T_BODY: f32 = 13.5;
/// Section headings, tab labels, the name of a row in a detail panel.
pub const T_HEAD: f32 = 15.0;
/// The title of a panel or a finding.
pub const T_TITLE: f32 = 17.0;
/// The one-line verdict in the header.
pub const T_LEAD: f32 = 19.5;
/// The single number a stat card exists for.
pub const T_METRIC: f32 = 24.0;

// Spacing. Four steps, used as a rhythm rather than picked per call site:
// a group is separated by S_XS, a block from its neighbour by S_SM, and a
// section from the next by S_MD. S_LG is for the gap above a heading, which
// is always larger than the gap below it — that asymmetry is what makes a
// heading belong to what follows rather than float between two blocks.
pub const S_XS: f32 = 4.0;
pub const S_SM: f32 = 8.0;
pub const S_MD: f32 = 12.0;
pub const S_LG: f32 = 18.0;
/// The window's left and right edge. Every top-level panel uses it, so the
/// header, the tabs and the content share one left edge.
pub const GUTTER: f32 = 14.0;

/// A measurement, in the monospaced face so its digits keep their column.
///
/// The live tables redraw every second. In a proportional face a `1` is
/// narrower than a `4`, so a latency that ticks from 14 ms to 41 ms shifts
/// the whole cell sideways, and a column of them never lines up. Reading a
/// table like that means reading every row instead of scanning the shape of
/// the column.
pub fn figure(text: impl Into<String>, size: f32, colour: egui::Color32) -> egui::RichText {
    egui::RichText::new(text).size(size).color(colour).family(egui::FontFamily::Monospace)
}

/// The status indicator, painted rather than typed.
///
/// This was a `●` glyph sized against the text around it, which left its
/// optical size and vertical position up to whichever font happened to
/// supply the character. A circle is two numbers; it should not depend on a
/// font's idea of a bullet.
pub fn status_dot(ui: &mut egui::Ui, colour: egui::Color32, radius: f32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(radius * 2.0, radius * 2.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), radius, colour);
}

pub fn status_colour(s: Status) -> egui::Color32 {
    match s {
        Status::Ok => GREEN,
        Status::Degraded | Status::DnsFail => YELLOW,
        Status::IspDown | Status::LanDown | Status::AdapterDown => RED,
    }
}

/// The header's dot and headline for a snapshot. Only a fresh reading taken
/// while sampling is a verdict: paused, stalled, not started yet or nothing
/// sent, the header says which, in grey, and not what the last reading was.
pub fn verdict_line(
    snap: &Snapshot,
    sampling: bool,
    now: f64,
    interval: std::time::Duration,
) -> (egui::Color32, &'static str) {
    use crate::monitor::Seen;
    // The same reading the tray makes of the same snapshot. The header used
    // to look only at `blind`, so a paused or stalled monitor stayed green
    // here while the tray icon went grey.
    match Seen::of(snap, sampling, now, interval) {
        Seen::Paused => (FG_DIM, crate::i18n::mon_paused()),
        Seen::Waiting => (FG_DIM, crate::i18n::mon_waiting()),
        Seen::Blind => (FG_DIM, crate::i18n::mon_blind()),
        Seen::Unrecorded => (FG_DIM, crate::i18n::mon_unrecorded()),
        Seen::Stale(_) => (FG_DIM, crate::i18n::mon_stale()),
        Seen::Verdict(status, _) => (status_colour(status), status.headline()),
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Tab {
    Live,
    Diagnose,
    Bloat,
    Optimise,
    History,
    Settings,
}

/// A message from a background job back to the UI.
pub enum Job {
    ScanProgress(String, f32),
    ScanDone(Box<Scan>),
    BloatProgress(String, f32),
    BloatDone(Box<BloatResult>),
    /// A ping of the running load test, with the speed at that moment.
    BloatLive(crate::bandwidth::Sample, Option<f64>),
    AirDone(Box<crate::probe::airscan::AirScan>),
    /// The Windows event log around one outage, keyed by that outage's row id
    /// so a slow read landing after the user moved on is discarded, not shown
    /// under the wrong entry.
    SysLog(i64, Vec<crate::probe::eventlog::SysEvent>),
    /// A report was written, to this path, or not, for this reason.
    ReportSaved(Result<String, String>),
    /// A step in the update flow, from the thread carrying it out.
    UpdateState(Box<crate::update::State>),
    /// Download progress, kept apart from `UpdateState` so the release does
    /// not have to be cloned once per percent.
    UpdateProgress(Option<f32>),
    /// One pass of reading every tweak's current state, with the generation it
    /// was started for. Four of the twenty-one shell out to `netsh` or
    /// `powercfg`, which is nearly all of what this costs, so it does not
    /// happen on the UI thread. The generation is what makes a read that
    /// started before an apply land in the bin rather than on screen. The
    /// last field is which tweaks have a "before" saved, or why the file
    /// holding them could not be read.
    TweakStates(
        u64,
        Vec<(String, String, Option<bool>)>,
        HashMap<String, crate::effect::Effect>,
        Result<HashSet<&'static str>, String>,
    ),
    /// One switch's apply or revert, run off the UI thread: `netsh` and the
    /// registry held the window still for as long as they took.
    TweakDone {
        id: &'static str,
        action: &'static str,
        result: Result<String, String>,
    },
    /// The AI's reading of a scan, with the generation of the scan it was
    /// asked about, so an answer landing after a rescan is dropped rather
    /// than shown beside numbers it never saw.
    AiDone(u64, Result<crate::ai::Explanation, String>),
}

pub struct App {
    pub store: Arc<Store>,
    pub monitor: Monitor,
    pub settings: Settings,
    /// Edited in the settings tab, applied on save.
    pub draft: Settings,
    pub draft_targets: String,

    pub tab: Tab,
    pub last: Snapshot,
    pub net: NetState,

    pub findings: Vec<Finding>,
    pub verdict: Verdict,
    pub measurements: Measurements,
    pub selected_finding: Option<usize>,
    pub scanning: bool,
    pub scan_label: String,
    pub scan_progress: f32,
    /// The steps the running scan has reported, in order, for its checklist.
    pub scan_steps: Vec<String>,
    /// When the scan on screen finished, and the one before it, so a scan
    /// after a fix can say what changed.
    pub scan_at: Option<f64>,
    pub prev_scan: Option<diag::PrevScan>,
    /// Whether the scan saturates the line to look for bufferbloat. On by
    /// default: it is the check that answers the question people actually ask.
    pub deep_scan: bool,
    /// Whether the running scan holds the monitor paused, so finishing it
    /// releases exactly the hold it took. See [`crate::monitor::Monitor::hold`].
    pub scan_held: bool,
    /// Seconds of long measurement to add to the next scan; 0 is none.
    pub long_secs: usize,
    /// Stops a running long measurement early. A fresh flag per scan, so a
    /// stop pressed during one scan cannot cut the next one short.
    pub scan_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Counts finished scans; see [`Job::AiDone`].
    pub scan_gen: u64,
    /// The AI's answer about the scan on screen, when one was asked for.
    pub ai_answer: Option<Result<crate::ai::Explanation, String>>,
    pub ai_running: bool,
    /// The connection as it was when the scan on screen started. The AI
    /// report masks this network's name, not whichever one is current.
    pub scan_net: NetState,

    pub bloat: BloatResult,
    pub bloat_running: bool,
    pub bloat_label: String,
    pub bloat_progress: f32,
    /// When the running load test started, for its countdown.
    pub bloat_started: Option<std::time::Instant>,
    /// Stops the running load test. A fresh flag per run, like `scan_cancel`.
    pub bloat_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// When the result in `bloat` was measured; `None` before the first run.
    pub bloat_at: Option<f64>,
    /// The running test's pings so far, and its latest download and upload
    /// speed, for drawing it while it runs.
    pub bloat_live: Vec<crate::bandwidth::Sample>,
    pub bloat_speed: [Option<f64>; 2],

    /// How far back the live chart looks, in seconds.
    pub chart_range_s: f64,
    /// Whether the chart draws each slice's range or its mean.
    pub chart_smooth: bool,
    /// Target keys the user has switched off in the chart legend.
    pub hidden_series: std::collections::HashSet<String>,
    /// The chart's samples, reduced and kept between frames.
    pub chart_cache: Option<live::ChartCache>,
    /// The headline cards, kept between frames for the same reason.
    ///
    /// Building them costs three queries against the store, one of them a
    /// scan of a day of outages, and every one takes the lock the monitor
    /// thread writes through. They used to be built on every frame, which is
    /// twice a second at rest and as fast as the display refreshes while the
    /// pointer is over the chart.
    pub card_cache: Option<live::CardCache>,

    pub tweak_states: Vec<(String, String, Option<bool>)>,
    /// What the line did before and after each change the log says was
    /// applied, keyed by tweak id. Read with the states, off the UI thread.
    pub tweak_effects: HashMap<String, crate::effect::Effect>,
    /// The tweaks with a "before" value saved, so the ones that can be
    /// switched back. Read with the states rather than on every frame.
    pub tweak_snapshots: HashSet<&'static str>,
    /// Why the file holding those values could not be read, when it could not.
    pub tweak_snapshots_error: Option<String>,
    /// What `tweak_effects` was read from, kept for the next refresh. Shared
    /// with the worker that does the reading.
    effect_cache: Arc<Mutex<crate::effect::Cache>>,
    pub selected_tweak: Option<usize>,
    /// The read this app has asked for most recently. Bumped by every
    /// `refresh_tweaks`; a result carrying an older number is a read that an
    /// apply overtook, and is dropped.
    tweaks_gen: u64,
    /// The read that produced what `tweak_states` currently holds. Behind
    /// `tweaks_gen` means a read is in flight and the list on screen is stale.
    tweaks_shown_gen: u64,
    /// Switches flicked and not yet confirmed by a read: tweak id to the
    /// position the switch was moved to, and whether the change has finished.
    /// The card shows that position, held, until the read after it lands; a
    /// change that fails is dropped at once and the switch goes back.
    pub tweak_pending: HashMap<&'static str, (bool, bool)>,
    /// For worker threads to wake the window when their result is in, rather
    /// than leaving it for the next half-second repaint.
    pub ctx: egui::Context,

    /// What the Wi-Fi card can hear around it, and the channel advice read
    /// off it. Empty until the first scan is asked for.
    pub air: crate::probe::airscan::AirScan,
    pub air_scanning: bool,
    /// Whether the optimise list shows the tweaks nothing here can change:
    /// not on offer on this machine, or set with nothing saved to go back
    /// to. Hidden by default: a list of dimmed switches read as a list of
    /// broken ones. Hiding them does not change the section counts.
    pub show_unavailable: bool,
    /// Row id of the outage whose cause panel is open, if any.
    pub selected_outage: Option<i64>,
    /// The open outage's context: fetched and parsed once when the selection
    /// changes, not on every frame. 33 KB of JSON per outage, and the panel
    /// used to parse it twice a frame at 60 Hz.
    pub outage_detail: Option<history::OutageDetail>,
    /// The outage list, re-read once a second rather than every frame.
    pub history_list: Option<history::ListCache>,
    /// The event log read for one outage, kept so opening an entry launches
    /// `wevtutil` once rather than on every frame it stays open.
    pub syslog: Option<(i64, Vec<crate::probe::eventlog::SysEvent>)>,
    /// The outage a read is currently running for.
    pub syslog_pending: Option<i64>,
    /// How far back the next report reaches.
    pub report_range: report::Range,
    /// A report is being written on a worker thread.
    pub report_busy: bool,
    /// Where the last report went, or why it did not, for the next frame's
    /// toast: the job arrives where there is no clock to time one.
    report_done: Option<Result<String, String>>,
    pub elevated: bool,
    pub autostart_on: bool,
    /// The overlay's settings took effect but are not on disk yet: they are
    /// written once the mouse is released, not on every frame of a drag.
    pub overlay_unsaved: bool,

    /// Where the update flow has got to. One value rather than a set of
    /// booleans, so "downloading and also up to date" cannot be represented.
    pub update: crate::update::State,
    /// Whether the top banner is still showing. Dismissing it only hides the
    /// banner; the settings tab keeps the same state on offer.
    pub update_banner: bool,

    pub toast: Option<(String, egui::Color32, f64)>,

    pub tx: mpsc::Sender<Job>,
    pub rx: mpsc::Receiver<Job>,

    /// Set when the app is meant to exit, rather than hide, on the close
    /// that follows: the tray's Quit, an update's restart, an elevated
    /// relaunch. Shared with the tray thread.
    quit: Arc<AtomicBool>,
    /// The notification-area icon. `None` if it could not be created, and
    /// then the close button closes, since nothing could bring a hidden
    /// window back. Dropped with the app, which removes the icon.
    tray: Option<crate::tray::Tray>,
    /// The ping overlay shown over a game. Dropped with the app.
    _overlay: Option<crate::overlay::Overlay>,
    /// Hide the window on the first frame: `--minimised`, or the setting.
    /// See `update`.
    start_hidden: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: Arc<Store>,
        settings: Settings,
        start_hidden: bool,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
        let monitor = Monitor::start(Arc::clone(&store), settings.clone());
        let (tx, rx) = mpsc::channel();
        let quit = Arc::new(AtomicBool::new(false));
        let tray = crate::tray::Tray::start(
            Arc::clone(&monitor.shared),
            monitor.notices.clone(),
            Arc::clone(&quit),
        );
        crate::game::start_watcher(Arc::clone(&monitor.shared), Arc::clone(&store));
        let overlay = crate::overlay::Overlay::start(Arc::clone(&monitor.shared));

        let mut app = App {
            store,
            monitor,
            draft: settings.clone(),
            draft_targets: settings.extra_targets.join("\n"),
            settings,
            tab: Tab::Live,
            last: Snapshot::default(),
            net: NetState::default(),
            findings: Vec::new(),
            verdict: Verdict::default(),
            measurements: Measurements::default(),
            selected_finding: None,
            scanning: false,
            scan_label: crate::i18n::diag_scan_hint().into(),
            scan_progress: 0.0,
            scan_steps: Vec::new(),
            scan_at: None,
            prev_scan: None,
            deep_scan: false,
            scan_held: false,
            long_secs: 0,
            scan_cancel: Arc::default(),
            scan_gen: 0,
            ai_answer: None,
            ai_running: false,
            scan_net: NetState::default(),
            bloat: BloatResult::default(),
            bloat_running: false,
            bloat_label: String::new(),
            bloat_progress: 0.0,
            bloat_started: None,
            bloat_cancel: Arc::default(),
            bloat_at: None,
            bloat_live: Vec::new(),
            bloat_speed: [None; 2],
            chart_range_s: 300.0,
            chart_smooth: false,
            hidden_series: std::collections::HashSet::new(),
            chart_cache: None,
            card_cache: None,
            tweak_states: Vec::new(),
            tweak_effects: HashMap::new(),
            tweak_snapshots: HashSet::new(),
            tweak_snapshots_error: None,
            effect_cache: Default::default(),
            tweaks_gen: 0,
            tweaks_shown_gen: 0,
            tweak_pending: HashMap::new(),
            ctx: cc.egui_ctx.clone(),
            selected_tweak: None,
            air: Default::default(),
            air_scanning: false,
            show_unavailable: false,
            selected_outage: None,
            outage_detail: None,
            history_list: None,
            syslog: None,
            syslog_pending: None,
            report_range: report::Range::Day,
            report_busy: false,
            report_done: None,
            elevated: crate::optimize::is_elevated(),
            autostart_on: crate::autostart::is_enabled(),
            overlay_unsaved: false,
            update: Default::default(),
            update_banner: false,
            toast: None,
            tx,
            rx,
            quit,
            tray,
            _overlay: overlay,
            start_hidden,
        };
        app.refresh_tweaks();
        if app.settings.check_updates {
            update_ui::start_check(&mut app, false);
        }
        app
    }

    /// Starts a read of every tweak's state on a worker thread.
    ///
    /// This used to run inline, which put 369 ms of `netsh` and `powercfg`
    /// on the UI thread: once before the first frame, so the window appeared
    /// that much later, and again on every click of the Optimise tab and
    /// after every apply.
    pub fn refresh_tweaks(&mut self) {
        let net = self.net.clone();
        let tx = self.tx.clone();
        let store = Arc::clone(&self.store);
        let cache = Arc::clone(&self.effect_cache);
        let ctx = self.ctx.clone();
        self.tweaks_gen += 1;
        let gen = self.tweaks_gen;
        std::thread::spawn(move || {
            let tweaks = crate::optimize::all();
            // Each on a thread of its own. The slow ones are separate `netsh`
            // and `powercfg` runs of 150 to 240 ms each, and one after another
            // they held every switch for three quarters of a second after
            // each click; side by side the pass takes as long as the slowest.
            let states = std::thread::scope(|s| {
                let reads: Vec<_> = tweaks
                    .iter()
                    .map(|t| {
                        let net = &net;
                        s.spawn(move || {
                            let st = t.read(net);
                            (t.id().to_string(), st.text, st.optimal)
                        })
                    })
                    .collect();
                reads
                    .into_iter()
                    .zip(&tweaks)
                    // A read that panicked is a read that could not be done,
                    // which is what `None` already says.
                    .map(|(h, t)| {
                        h.join().unwrap_or_else(|_| (t.id().to_string(), String::new(), None))
                    })
                    .collect()
            });
            let snapshots = crate::optimize::snapshotted(&tweaks, &net).map_err(|e| e.to_string());
            // Before and after each change, read here rather than per frame:
            // a day of samples on two anchors is tens of thousands of rows.
            let now = crate::store::now();
            let log = store.tweaks_between(0.0, now);
            let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
            let effects = tweaks
                .iter()
                .filter_map(|t| {
                    crate::effect::of_cached(&store, &log, t.id(), now, &mut cache)
                        .map(|e| (t.id().to_string(), e))
                })
                .collect();
            drop(cache);
            let _ = tx.send(Job::TweakStates(gen, states, effects, snapshots));
            ctx.request_repaint();
        });
    }

    /// Moves one switch: applies the tweak when `on`, reverts it otherwise,
    /// on a worker thread. The switch shows the new position straight away
    /// and cannot be flicked again until the read after the change lands.
    pub fn flip_tweak(&mut self, id: &'static str, on: bool) {
        if self.tweak_pending.contains_key(id) {
            return;
        }
        self.tweak_pending.insert(id, (on, false));
        let net = self.net.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            // The trait objects are not `Send`, so the worker builds its own.
            let result = match crate::optimize::all().into_iter().find(|t| t.id() == id) {
                Some(t) if on => crate::optimize::apply(t.as_ref(), &net),
                Some(t) => crate::optimize::revert(t.as_ref(), &net),
                None => Err(anyhow::anyhow!("unknown tweak {id}")),
            };
            let action = if on { "apply" } else { "revert" };
            let _ =
                tx.send(Job::TweakDone { id, action, result: result.map_err(|e| e.to_string()) });
            ctx.request_repaint();
        });
    }

    /// True while a read is in flight, so the tab can say so instead of
    /// showing a list that is about to change under the pointer.
    pub fn tweaks_loading(&self) -> bool {
        self.tweaks_shown_gen != self.tweaks_gen
    }

    /// Closes the app for real. The close button only hides it while there
    /// is a tray icon, so anything that means to exit says so first.
    pub fn quit(&self, ctx: &egui::Context) {
        self.quit.store(true, Ordering::Relaxed);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    pub fn toast(&mut self, text: impl Into<String>, colour: egui::Color32, now: f64) {
        self.toast = Some((text.into(), colour, now + 6.0));
    }

    fn drain_jobs(&mut self, now: f64) {
        while let Ok(job) = self.rx.try_recv() {
            match job {
                Job::ScanProgress(label, frac) => {
                    match self.scan_steps.last_mut() {
                        Some(last) if *last == label => {}
                        Some(last) if diag::same_step(last, &label) => last.clone_from(&label),
                        _ => self.scan_steps.push(label.clone()),
                    }
                    self.scan_label = label;
                    self.scan_progress = frac;
                }
                Job::ScanDone(scan) => {
                    let scan = *scan;
                    if let Some(at) = self.scan_at.filter(|_| !self.findings.is_empty()) {
                        self.prev_scan =
                            Some(diag::PrevScan::of(at, &self.verdict, &self.measurements));
                    }
                    self.scan_at = Some(crate::store::now());
                    self.findings = scan.findings;
                    self.verdict = scan.verdict;
                    self.measurements = scan.measurements;
                    self.scan_gen += 1;
                    self.ai_answer = None;
                    self.ai_running = false;
                    self.selected_finding = None;
                    self.scanning = false;
                    self.scan_label = crate::i18n::diag_scan_done().into();
                    self.scan_progress = 1.0;
                    // Only the scan's own hold. Unpausing outright used to
                    // override a pause the user had set, or one a load test
                    // still needed.
                    if std::mem::take(&mut self.scan_held) {
                        self.monitor.release();
                    }
                }
                Job::AiDone(generation, answer) => {
                    if generation == self.scan_gen {
                        self.ai_answer = Some(answer);
                        self.ai_running = false;
                    }
                }
                Job::BloatProgress(label, frac) => {
                    self.bloat_label = label;
                    self.bloat_progress = frac;
                }
                Job::BloatLive(sample, mbps) => {
                    let slot = match sample.phase {
                        crate::bandwidth::Phase::Down => Some(0),
                        crate::bandwidth::Phase::Up => Some(1),
                        crate::bandwidth::Phase::Idle => None,
                    };
                    if let (Some(i), Some(m)) = (slot, mbps) {
                        self.bloat_speed[i] = Some(m);
                    }
                    self.bloat_live.push(sample);
                }
                Job::BloatDone(res) => {
                    self.bloat_running = false;
                    self.bloat_started = None;
                    self.bloat_progress = 1.0;
                    self.monitor.release();
                    // A stopped test measured nothing worth keeping, and
                    // showing its half would replace a real result with one.
                    if res.cancelled {
                        self.toast(crate::i18n::bloat_cancelled(), FG_DIM, now);
                    } else {
                        self.bloat = *res;
                        self.bloat_at = Some(crate::store::now());
                    }
                }
                Job::AirDone(scan) => {
                    self.air = *scan;
                    self.air_scanning = false;
                    self.monitor.release();
                }
                Job::UpdateState(state) => {
                    // Reopen the banner on every transition worth announcing,
                    // including one the user dismissed earlier: they dismissed
                    // "an update is available", not "it is installed".
                    self.update_banner = matches!(
                        *state,
                        crate::update::State::Available(_)
                            | crate::update::State::Installed(_)
                            | crate::update::State::Failed(_)
                    );
                    self.update = *state;
                }
                Job::UpdateProgress(frac) => {
                    if let crate::update::State::Downloading(_, f) = &mut self.update {
                        *f = frac;
                    }
                }
                Job::TweakStates(gen, states, effects, snapshots) => {
                    // A read started before the last apply describes the
                    // machine as it was, not as it is. Showing it would put a
                    // just-applied tweak back in the "worth changing" column.
                    if gen == self.tweaks_gen {
                        self.tweaks_shown_gen = gen;
                        self.tweak_states = states;
                        self.tweak_effects = effects;
                        (self.tweak_snapshots, self.tweak_snapshots_error) = match snapshots {
                            Ok(set) => (set, None),
                            Err(e) => (HashSet::new(), Some(e)),
                        };
                        // A change that finished before this read began is
                        // in it; one still running is not, and stays held.
                        self.tweak_pending.retain(|_, (_, done)| !*done);
                    }
                }
                Job::TweakDone { id, action, result } => {
                    match result {
                        Ok(msg) => {
                            self.store.log_tweak(id, action, "", &msg);
                            self.toast(msg, GREEN, now);
                            if let Some((_, done)) = self.tweak_pending.get_mut(id) {
                                *done = true;
                            }
                        }
                        Err(e) => {
                            self.store.log_tweak(id, &format!("{action}_failed"), "", &e);
                            self.toast(e, RED, now);
                            self.tweak_pending.remove(id);
                        }
                    }
                    self.refresh_tweaks();
                }
                Job::SysLog(id, events) => {
                    if self.syslog_pending == Some(id) {
                        self.syslog_pending = None;
                        self.syslog = Some((id, events));
                    }
                }
                Job::ReportSaved(result) => {
                    self.report_busy = false;
                    self.report_done = Some(result);
                }
            }
        }
    }

    /// Takes the newest snapshot. Outages are announced by the tray, through
    /// Windows, not here: this only runs while the window gets frames, and
    /// the window is hidden most of the time.
    fn drain_snapshots(&mut self) {
        let mut newest = None;
        while let Ok(snap) = self.monitor.rx.try_recv() {
            newest = Some(snap);
        }
        let Some(snap) = newest else { return };
        self.net = snap.net.clone();
        self.last = snap;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // eframe shows the window after its first frame whatever the viewport
        // builder asked for (`post_rendering` in eframe 0.29), so starting
        // minimised never hid anything: autostart put the window on screen.
        // Our commands are applied after that show, in the same frame, so the
        // window is gone before anyone sees it. Only with an icon to bring it
        // back from.
        if std::mem::take(&mut self.start_hidden) {
            if self.tray.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            // Back from `OFF_SCREEN`, where `main` started it so the first
            // frame is not seen. Hidden, it waits here for the tray; with no
            // tray it simply appears here.
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(on_screen(ctx)));
        }

        // The close button hides to the tray; measuring goes on. Only a quit
        // the app asked for itself goes through.
        if ctx.input(|i| i.viewport().close_requested())
            && self.tray.is_some()
            && !self.quit.load(Ordering::Relaxed)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // Show, then hide. winit remembers visibility itself and only
            // calls `ShowWindow` when its flag changes, but the tray and a
            // second launch bring the window back through Win32 directly, so
            // after the first hide the flag stayed "hidden" and every later
            // hide did nothing. The window is visible here anyway; the first
            // command only brings the flag back in line with it.
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        let now = ctx.input(|i| i.time);
        quiet_tooltips_while_scrolling(ctx);
        self.drain_jobs(now);
        match self.report_done.take() {
            Some(Ok(path)) => self.toast(crate::i18n::live_report_saved(&path), GREEN, now),
            Some(Err(e)) => self.toast(crate::i18n::set_save_failed(&e), RED, now),
            None => {}
        }
        self.drain_snapshots();

        // The monitor produces a sample per second; repainting on that cadence
        // keeps the chart live without spinning the GPU.
        //
        // Not while hidden in the tray: nothing is drawn, and a booked frame
        // was what woke the event loop to find a window it could not paint
        // (see `NETDOC PATCH` in vendor/winit). A frame still runs when
        // something asks for one, a job landing or the tray bringing the
        // window back, and books the next from there. Minimised is the same:
        // restoring it is an event of its own.
        let minimised = ctx.input(|i| i.viewport().minimized) == Some(true);
        if !minimised && crate::tray::main_window_shown().unwrap_or(true) {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }

        // All three panels indent to the same GUTTER, so the header, the tab
        // labels and whatever the tab draws share one left edge. They were
        // at 16, 12 and 14 before, which is not a visible misalignment so
        // much as a permanent faint wrongness down the side of the window.
        // No separator of egui's own under the header or the tabs: the header
        // and its tabs are one block, and the only rule is the one the tabs
        // draw, which egui's line would otherwise be painted over, taking a
        // pixel off the selected tab's underline.
        egui::TopBottomPanel::top("header")
            .show_separator_line(false)
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::symmetric(GUTTER, S_MD)))
            .show(ctx, |ui| self.header(ui));

        egui::TopBottomPanel::top("tabs")
            .show_separator_line(false)
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::symmetric(GUTTER, 0.0)))
            .show(ctx, |ui| {
                // Wrapped, not a plain row: six tab names in a narrow window
                // run past the right edge, and the ones that fall off are
                // Outage history and Settings -- navigation that vanishes
                // rather than moving is navigation that looks missing.
                let rule = ui.painter().add(egui::Shape::Noop);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(S_XS, 0.0);
                    for (tab, label) in [
                        (Tab::Live, crate::i18n::tab_live()),
                        (Tab::Diagnose, crate::i18n::tab_diagnose()),
                        (Tab::Bloat, crate::i18n::tab_bloat()),
                        (Tab::Optimise, crate::i18n::tab_optimise()),
                        (Tab::History, crate::i18n::tab_history()),
                        (Tab::Settings, crate::i18n::tab_settings()),
                    ] {
                        let selected = self.tab == tab;
                        // Settings edited and not saved follow the user to
                        // the other tabs, so leaving them behind is a choice
                        // rather than an accident.
                        let label = if tab == Tab::Settings && settings_tab::is_dirty(self) {
                            format!("{label} •")
                        } else {
                            label.to_string()
                        };
                        if tab_button(ui, &label, selected).clicked() {
                            self.tab = tab;
                            if tab == Tab::Optimise {
                                self.refresh_tweaks();
                            }
                        }
                    }
                });

                // The rule that the selected tab's underline sits on. Without
                // it the tab strip and the content below are one undivided
                // field of the same colour. Its slot was taken before the
                // tabs were drawn, so the underline covers it rather than the
                // other way round.
                let rect = ui.max_rect();
                ui.painter().set(
                    rule,
                    egui::Shape::hline(
                        rect.x_range(),
                        ui.min_rect().bottom() - 0.5,
                        egui::Stroke::new(1.0_f32, LINE),
                    ),
                );
            });

        if update_ui::banner_wanted(self) {
            egui::TopBottomPanel::top("update_banner")
                .frame(
                    egui::Frame::none()
                        .fill(BG2)
                        .inner_margin(egui::Margin::symmetric(GUTTER, S_SM)),
                )
                .show(ctx, |ui| update_ui::banner(self, ui));
        }

        if let Some((text, colour, until)) = self.toast.clone() {
            // The Settings tab has a bar of its own at the bottom and says it
            // there: two strips stacked under the form read as two footers.
            if now < until && self.tab == Tab::Settings {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(until - now));
            } else if now < until {
                egui::TopBottomPanel::bottom("toast")
                    .frame(
                        egui::Frame::none()
                            .fill(BG2)
                            .inner_margin(egui::Margin::symmetric(GUTTER, S_SM)),
                    )
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            status_dot(ui, colour, 5.0);
                            ui.add_space(S_XS);
                            ui.label(egui::RichText::new(text).size(T_BODY).color(FG));
                            // Pushed to the far edge and kept quiet. The toast
                            // is there to be read; the way to get rid of it
                            // should not be the loudest thing in it.
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if button(ui, crate::i18n::btn_dismiss(), Emphasis::Ghost)
                                        .clicked()
                                    {
                                        self.toast = None;
                                    }
                                },
                            );
                        });
                    });
            } else {
                self.toast = None;
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(GUTTER)))
            .show(ctx, |ui| match self.tab {
                Tab::Live => live::show(self, ui),
                Tab::Diagnose => diag::show(self, ui),
                Tab::Bloat => bloat::show(self, ui),
                Tab::Optimise => opt::show(self, ui),
                Tab::History => history::show(self, ui),
                Tab::Settings => settings_tab::show(self, ui),
            });
    }
}

impl App {
    fn header(&mut self, ui: &mut egui::Ui) {
        let (colour, headline) = verdict_line(
            &self.last,
            !self.monitor.shared.paused(),
            crate::store::now(),
            self.settings.interval(),
        );

        // The right-hand block's width is reserved before the left block is
        // drawn, and the left block is then held to what is left.
        //
        // Laid out the other way round — a `vertical` inside a `horizontal`,
        // followed by a right-to-left layout — the facts row takes the whole
        // header width to wrap in, because nothing has told it otherwise, and
        // the elevation note is then painted straight over the end of it. Two
        // pieces of text on the same pixels is not a spacing problem that a
        // bit more padding fixes; it is two layouts each believing they own
        // the same space.
        let reserved = if self.elevated { 140.0 } else { 300.0 };

        // Held steady: this panel moving moves every tab under it.
        steady(ui, "header", |ui| self.header_row(ui, colour, headline, reserved));
    }

    fn header_row(
        &mut self,
        ui: &mut egui::Ui,
        colour: egui::Color32,
        headline: &str,
        reserved: f32,
    ) {
        ui.horizontal_top(|ui| {
            ui.add_space(2.0);
            status_dot(ui, colour, 7.0);
            ui.add_space(S_XS);

            let left_w = (ui.available_width() - reserved).max(220.0);
            ui.allocate_ui_with_layout(
                egui::vec2(left_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(left_w);
                    ui.label(egui::RichText::new(headline).size(T_LEAD).strong().color(FG));
                    ui.add_space(S_XS * 0.5);
                    let facts = self.connection_facts();
                    if facts.is_empty() {
                        ui.label(
                            egui::RichText::new(crate::i18n::hdr_no_adapter())
                                .size(T_BODY)
                                .color(FG_DIM),
                        );
                    } else {
                        connection_row(ui, &facts);
                    }
                },
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if self.elevated {
                    ui.label(
                        egui::RichText::new(crate::i18n::hdr_administrator())
                            .size(T_META)
                            .color(GREEN),
                    );
                } else {
                    // The caveat used to sit to the *left* of the button, on
                    // the same line, where it ate a third of the header's
                    // width and crowded the connection facts into the corner.
                    // Stacked under the button it reads as belonging to that
                    // button — which is what it is, a note on what the button
                    // is for — and the header gets its horizontal room back.
                    //
                    // `Align::Max` in a top-down layout is the right edge, so
                    // the button and its note share one right margin with the
                    // window.
                    ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                        // Secondary, not primary. It is a standing offer that
                        // sits in the header on every screen, and an
                        // accent-filled button that never goes away is a
                        // button nobody reads.
                        if button(ui, crate::i18n::hdr_restart_elevated(), Emphasis::Secondary)
                            .clicked()
                        {
                            match crate::autostart::relaunch_elevated() {
                                // Through the normal exit, not
                                // `process::exit`: the tray icon has to be
                                // removed and the outage closed on the way.
                                Ok(()) => self.quit(ui.ctx()),
                                Err(e) => {
                                    let now = ui.input(|i| i.time);
                                    self.toast(e.to_string(), RED, now);
                                }
                            }
                        }
                        ui.add_space(S_XS * 0.5);
                        // Dropped a step to T_MICRO. It is a caveat, and the
                        // scale reserves that step for exactly this: something
                        // worth having on screen that nobody has to read twice.
                        ui.label(
                            egui::RichText::new(crate::i18n::hdr_standard_mode())
                                .size(T_MICRO)
                                .color(FG_DIM),
                        );
                    });
                }
            });
        });
    }

    /// One fact from the connection line.
    ///
    /// The line used to be a single interpolated string: six unrelated facts
    /// welded together with `·`, every one of them the same size and the same
    /// grey. That reads as one long label rather than as six values, and the
    /// separator between the network name and the signal looked exactly like
    /// the separator between a channel and its band — so there was nothing to
    /// tell you where one fact stopped. Splitting it lets the values carry
    /// weight, the labels stay quiet, and the one value with thresholds get
    /// the colour it deserves.
    fn connection_facts(&self) -> Vec<Fact> {
        let n = &self.net;
        let mut facts = Vec::new();
        if n.adapter_name.is_empty() {
            return facts;
        }

        let dash = |s: String| if s.is_empty() { "—".to_string() } else { s };
        let gateway = n.gateway.map(|g| g.to_string()).unwrap_or_else(|| "—".into());

        match n.medium {
            crate::probe::netstate::Medium::Wifi => {
                // The network's name first. It is the one thing here the user
                // chose, and the only one they would recognise at a glance.
                facts.push(Fact::new(crate::i18n::word_network(), dash(n.ssid.clone()), FG));

                if let Some(pct) = n.signal_pct {
                    // Signal is the only fact on this line with thresholds, so
                    // it is the only one that gets a colour. Colouring more
                    // than one would mean none of them stood out.
                    // In words first: "78%" means nothing until you know
                    // what a good one is. The dBm figure is in Diagnose.
                    let (word, colour) = if pct >= 67 {
                        (crate::i18n::word_signal_good(), GREEN)
                    } else if pct >= 34 {
                        (crate::i18n::word_signal_fair(), YELLOW)
                    } else {
                        (crate::i18n::word_signal_weak(), RED)
                    };
                    let value = format!("{word} · {pct}%");
                    facts.push(Fact::new(crate::i18n::word_signal(), value, colour));
                }

                if let Some(ch) = n.channel {
                    // The band matters more than the channel number to anyone
                    // who is not already debugging, so show both.
                    let value = match n.band() {
                        Some(band) => format!("{ch} · {band}"),
                        None => ch.to_string(),
                    };
                    facts.push(Fact::new(crate::i18n::word_channel(), value, FG));
                }

                if let Some(rx) = n.rx_mbps {
                    let value = if n.phy.is_empty() {
                        format!("{rx} Mbps")
                    } else {
                        format!("{rx} Mbps · {}", n.phy)
                    };
                    facts.push(Fact::new(crate::i18n::word_link(), value, FG));
                }

                facts.push(Fact::new(crate::i18n::word_gateway(), gateway, FG));
            }
            _ => {
                facts.push(Fact::new(
                    crate::i18n::word_network(),
                    crate::i18n::word_wired().to_string(),
                    FG,
                ));
                facts.push(Fact::new(
                    crate::i18n::word_link(),
                    format!("{} Mbps", n.link_speed_mbps),
                    FG,
                ));
                facts.push(Fact::new(crate::i18n::word_gateway(), gateway, FG));
                facts.push(Fact::new(
                    "DNS",
                    dash(
                        n.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", "),
                    ),
                    FG,
                ));
            }
        }

        // The adapter's name is the longest string on the line and the one
        // least often needed, so it goes last and stays dim instead of pushing
        // the network name and the signal off to the right.
        facts.push(Fact::new(crate::i18n::word_adapter(), n.adapter_name.clone(), FG_DIM));
        facts
    }
}

/// A labelled value in the header's connection line.
struct Fact {
    /// Sits above the value, quiet and small.
    label: &'static str,
    value: String,
    colour: egui::Color32,
}

impl Fact {
    fn new(label: &'static str, value: String, colour: egui::Color32) -> Self {
        Fact { label, value, colour }
    }
}

/// Draw the connection facts as a wrapping row of columns, each a small label
/// over its value.
///
/// The facts used to run inline, label then value then a rule then the next
/// label, all within a few pixels of each other in two sizes of small text.
/// Six of those in a row read as one dense string, not as six readings. As
/// columns, the eye can run along the values alone, and the gap between
/// columns does the separating that the painted rules were straining to do.
///
/// Wrapping rather than truncating: the old single line ran off the right edge
/// of a narrow window and took the gateway with it, and a fact you cannot see
/// is worse than a second row.
fn connection_row(ui: &mut egui::Ui, facts: &[Fact]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(S_LG + S_XS, S_SM);

        for fact in facts {
            // One widget per fact, not two, so the row can wrap around a
            // column but never through it and leave a label on one line with
            // its value on the next. `extend` keeps the label from wrapping
            // the job itself into the space left on the current line.
            let mut job = egui::text::LayoutJob::default();
            append(&mut job, 0.0, fact.label, T_MICRO, FG_DIM, false);
            append(&mut job, 0.0, "\n", T_MICRO, FG_DIM, false);
            // Monospaced, like every other measurement in the app: these
            // refresh as the link changes, and proportional digits make the
            // whole row shuffle sideways when one of them does.
            append(&mut job, 0.0, &fact.value, T_BODY, fact.colour, true);
            ui.add(egui::Label::new(job).extend());
        }
    });
}

/// One tab in the strip under the header.
///
/// Drawn by hand rather than as a `selectable_label`: egui fills a selected
/// one with a tinted pill, which reads as a pressed button, and the underline
/// that was added on top of the pill floated a few pixels above the rule it
/// was meant to sit on. Here the selection is the text's colour and an accent
/// bar lying on the rule itself, so the current tab reads as the page you are
/// on, joined to the content under it.
fn tab_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    // Equal room above and below the text, so the label sits in the middle of
    // the strip instead of hugging the header or the rule.
    let pad = egui::vec2(S_MD, S_SM + 2.0);
    let underline = 2.0;
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(T_HEAD),
        FG, // Recoloured when painted.
    );
    let size = galley.size() + 2.0 * pad + egui::vec2(0.0, underline);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });

    if ui.is_rect_visible(rect) {
        let hovered = response.hovered() && !selected;
        let colour = if selected || hovered { FG } else { FG_DIM };
        let text_pos = egui::pos2(rect.left() + pad.x, rect.top() + pad.y);
        ui.painter().galley(text_pos, galley, colour);

        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + S_XS, rect.bottom() - underline),
            rect.right_bottom() - egui::vec2(S_XS, 0.0),
        );
        if selected {
            ui.painter().rect_filled(
                bar,
                egui::Rounding { nw: 1.0, ne: 1.0, sw: 0.0, se: 0.0 },
                ACCENT,
            );
        } else if hovered {
            // A hint of where the bar would go, not the bar itself.
            ui.painter().rect_filled(bar, 0.0, LINE);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(rect.shrink(1.0), 4.0, egui::Stroke::new(1.0_f32, ACCENT));
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Adds one run to a fact's layout job, in the app's own type scale.
fn append(
    job: &mut egui::text::LayoutJob,
    leading_space: f32,
    text: &str,
    size: f32,
    colour: egui::Color32,
    monospace: bool,
) {
    let family =
        if monospace { egui::FontFamily::Monospace } else { egui::FontFamily::Proportional };
    job.append(
        text,
        leading_space,
        egui::TextFormat {
            font_id: egui::FontId::new(size, family),
            color: colour,
            ..Default::default()
        },
    );
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = BG2;
    visuals.extreme_bg_color = BG3;
    visuals.faint_bg_color = BG2;
    visuals.override_text_color = Some(FG);
    visuals.widgets.inactive.bg_fill = BG3;
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x32, 0x38, 0x46);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.selection.bg_fill = ACCENT.linear_multiply(0.4);

    // Keyboard focus was the default 1px white-ish rect, which on this
    // palette is nearly invisible against BG3. The accent is the colour
    // selection already uses, so focus and selection read as one idea.
    visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(0x32, 0x38, 0x46);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    visuals.window_stroke = egui::Stroke::new(1.0_f32, LINE);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(S_SM, S_SM);
    style.spacing.button_padding = egui::vec2(S_MD, 6.0);

    // Widgets that are never given an explicit RichText — buttons, checkbox
    // labels, drag values, text fields — were taking egui's own scale, which
    // is a different scale from the one the rest of the app is drawn on. The
    // settings tab was the worst of it: hand-sized labels next to
    // default-sized controls, on every row.
    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::new(T_MICRO, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(T_BODY, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(T_BODY, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(T_TITLE, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(T_BODY, FontFamily::Monospace)),
    ]
    .into();

    // Give the scroll bar a strip of its own down the right-hand side.
    //
    // egui's floating scroll bars allocate nothing and are painted over the
    // content, so the moment one appeared it sat on the right edge of
    // whatever was underneath — on the live tab, the newest end of the chart,
    // which is the part being watched. This is the width it gets instead.
    //
    // `ScrollArea` multiplies this by how far the bar is shown, so the strip
    // costs nothing on a view that fits and is only taken while a bar is
    // actually there. The content therefore does narrow by this much as the
    // bar fades in, which is a reflow — but it is the same reflow a solid
    // scroll bar would cause, it is animated rather than instant, and the
    // alternative is the bar covering live data.
    style.spacing.scroll.floating_allocated_width = style.spacing.scroll.bar_width + 2.0;

    // Tooltips appear the instant the pointer is over the thing, everywhere.
    //
    // egui's defaults are built for a tooltip that repeats a button's label:
    // it waits for the pointer to come to rest, then waits another third of a
    // second, and skips both if another tooltip was shown in the last few
    // hundred milliseconds. That last rule is why the delay felt random
    // rather than slow — the same chip opened instantly or late depending on
    // where the pointer had been beforehand. Here a tooltip is not a repeat
    // of the label; it is where the explanation of a reading lives, and the
    // readout on the plot is a value that has to track the pointer, which
    // cannot be done at all by a tooltip that waits for the pointer to stop.
    //
    // This has to be set on the context: `Response` reads the tooltip timing
    // from `ctx.style()`, so the same fields set on a `Ui` are ignored.
    style.interaction.show_tooltips_only_when_still = false;
    style.interaction.tooltip_delay = 0.0;
    style.interaction.tooltip_grace_time = 0.0;

    ctx.set_style(style);
}

/// How long after the wheel last moved tooltips stay away.
const SCROLL_QUIET_S: f32 = 0.3;

/// Keeps tooltips from opening while the page is being scrolled.
///
/// egui hides them itself for `tooltip_delay` after a scroll, and the delay is
/// zero here (see `apply_theme`), so that rule never fired: scrolling the Live
/// tab carried card after card under a still pointer, each one opened its
/// tooltip at once, and a tooltip that lands under the pointer takes the wheel
/// from the page. The page stopped moving. Measured in the running app, the
/// pointer sat over a tooltip on a wheel frame. The delay is raised for just
/// as long as the wheel is turning, which switches that rule back on, and is
/// zero again, instant tooltips and all, once it stops.
fn quiet_tooltips_while_scrolling(ctx: &egui::Context) {
    let scrolling = ctx.input(|i| i.time_since_last_scroll()) < SCROLL_QUIET_S;
    let delay = if scrolling { SCROLL_QUIET_S } else { 0.0 };
    if ctx.style().interaction.tooltip_delay != delay {
        ctx.style_mut(|s| s.interaction.tooltip_delay = delay);
    }
}

/// Name a latency figure by the same thresholds that colour it.
///
/// The colour already says good-or-bad to anyone who knows the convention;
/// the word says it to everyone else, and it is the difference between a
/// number on a chart and a number the user can act on.
pub fn latency_verdict(ms: f64, s: &Settings) -> &'static str {
    if ms < s.ping_ok_ms {
        crate::i18n::live_scale_ok()
    } else if ms < s.ping_bad_ms {
        crate::i18n::live_scale_mid()
    } else {
        crate::i18n::live_scale_bad()
    }
}

/// Colour a latency figure by the configured thresholds.
pub fn latency_colour(ms: f64, s: &Settings) -> egui::Color32 {
    if ms < s.ping_ok_ms {
        GREEN
    } else if ms < s.ping_bad_ms {
        YELLOW
    } else {
        RED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_000_000.0;
    const SECOND: std::time::Duration = std::time::Duration::from_secs(1);

    #[test]
    fn a_sweep_that_sent_nothing_shows_no_verdict() {
        // A blind snapshot carries the default status, "Ok". The header used
        // to paint whatever `status` said.
        let blind =
            Snapshot { ts: NOW, blind: Some("no ICMP handle".into()), ..Snapshot::default() };
        let (colour, headline) = verdict_line(&blind, true, NOW, SECOND);
        assert_eq!(colour, FG_DIM);
        assert_eq!(headline, crate::i18n::mon_blind());

        let measured = Snapshot { ts: NOW, status: Status::IspDown, ..Snapshot::default() };
        assert_eq!(verdict_line(&measured, true, NOW, SECOND), (RED, Status::IspDown.headline()));
    }

    #[test]
    fn a_paused_or_stale_monitor_shows_no_verdict_in_the_window() {
        // Found on a screenshot: twelve seconds into a pause the header still
        // said "Connection healthy" in green. The tray was grey.
        let healthy = Snapshot { ts: NOW, status: Status::Ok, ..Snapshot::default() };
        let (colour, headline) = verdict_line(&healthy, false, NOW + 12.0, SECOND);
        assert_eq!(colour, FG_DIM);
        assert_eq!(headline, crate::i18n::mon_paused());

        let (colour, headline) = verdict_line(&healthy, true, NOW + 120.0, SECOND);
        assert_eq!(colour, FG_DIM, "a reading nobody refreshed");
        assert_eq!(headline, crate::i18n::mon_stale());

        let (_, headline) = verdict_line(&Snapshot::default(), true, NOW, SECOND);
        assert_eq!(headline, crate::i18n::mon_waiting());
    }

    #[test]
    fn a_steady_panel_never_gives_height_back() {
        // A header whose content is one line one second and two the next
        // moved every tab under it by that line.
        let ctx = egui::Context::default();
        let mut heights = Vec::new();
        for frame in 0..8 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                let panel = egui::TopBottomPanel::top("header").show(ctx, |ui| {
                    steady(ui, "header", |ui| {
                        ui.label("one");
                        if frame % 2 == 1 {
                            ui.label("two");
                        }
                    });
                });
                heights.push(panel.response.rect.height());
            });
        }
        let tallest = heights[1];
        assert!(heights[1..].iter().all(|h| (h - tallest).abs() < 0.5), "{heights:?}");
    }

    #[test]
    fn no_tooltip_opens_while_the_wheel_is_turning() {
        // A tooltip that opens under a still pointer during a scroll takes
        // the wheel from the page, and the page stops. Instant tooltips have
        // to wait for the wheel, and only for the wheel.
        let ctx = egui::Context::default();
        ctx.style_mut(|s| {
            s.interaction.show_tooltips_only_when_still = false;
            s.interaction.tooltip_delay = 0.0;
            s.interaction.tooltip_grace_time = 0.0;
        });
        let mut shown = Vec::new();
        for frame in 0..60 {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))];
            if (5..10).contains(&frame) {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 40.0),
                    modifiers: Default::default(),
                });
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                events,
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            };
            let mut open = false;
            let _ = ctx.run(input, |ctx| {
                quiet_tooltips_while_scrolling(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (_, resp) =
                        ui.allocate_exact_size(egui::vec2(300.0, 300.0), egui::Sense::hover());
                    resp.on_hover_ui(|ui| {
                        open = true;
                        ui.label("1.1.1.1");
                    });
                });
            });
            shown.push(open);
        }
        assert!(shown[3], "instant before any scrolling: {shown:?}");
        assert!(!shown[5..=20].iter().any(|s| *s), "hidden while scrolling: {shown:?}");
        assert!(shown[40], "back once the wheel has stopped: {shown:?}");
    }

    #[test]
    fn a_card_explains_itself_from_its_question_mark_only() {
        // Where on a clickable card the explanation opens, pointer by pointer.
        let tip_at = |pos: egui::Pos2| {
            let ctx = egui::Context::default();
            ctx.style_mut(|s| {
                s.interaction.show_tooltips_only_when_still = false;
                s.interaction.tooltip_delay = 0.0;
            });
            let mut open = false;
            for frame in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 620.0),
                    )),
                    events: vec![egui::Event::PointerMoved(pos)],
                    time: Some(frame as f64),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let r = stat_card(ui, "Jitter", "5 ms", "", FG, Some(200.0), "tip");
                        // As the Live tab does with a card that opens a tab.
                        let _ = card_clicked(ui, &r);
                    });
                    open = ctx.memory(|m| m.any_popup_open())
                        || ctx.viewport(|v| !v.this_pass.tooltips.widget_tooltips.is_empty());
                });
            }
            open
        };
        // The label row sits about 26 px down; somewhere along it is the mark.
        let on_label_row = (20..200).step_by(2).any(|x| tip_at(egui::pos2(x as f32, 26.0)));
        assert!(on_label_row, "the question mark opens the explanation");
        assert!(!tip_at(egui::pos2(60.0, 60.0)), "the figure does not");
    }
}
