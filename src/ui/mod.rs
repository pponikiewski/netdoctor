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
mod update_ui;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use eframe::egui;

use crate::bandwidth::BloatResult;
use crate::diagnose::{Finding, Scan, Verdict};
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
    Traceroute(Vec<String>),
    AirDone(Box<crate::probe::airscan::AirScan>),
    /// The Windows event log around one outage, keyed by that outage's row id
    /// so a slow read landing after the user moved on is discarded, not shown
    /// under the wrong entry.
    SysLog(i64, Vec<crate::probe::eventlog::SysEvent>),
    /// A step in the update flow, from the thread carrying it out.
    UpdateState(Box<crate::update::State>),
    /// Download progress, kept apart from `UpdateState` so the release does
    /// not have to be cloned once per percent.
    UpdateProgress(Option<f32>),
    /// One pass of reading every tweak's current state, with the generation it
    /// was started for. Four of the twenty-one shell out to `netsh` or
    /// `powercfg`, which is 369 ms of the 369 ms this costs, so it does not
    /// happen on the UI thread. The generation is what makes a read that
    /// started before an apply land in the bin rather than on screen.
    TweakStates(u64, Vec<(String, String, Option<bool>)>),
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
    pub selected_finding: Option<usize>,
    pub scanning: bool,
    pub scan_label: String,
    pub scan_progress: f32,
    /// Whether the scan saturates the line to look for bufferbloat. On by
    /// default: it is the check that answers the question people actually ask.
    pub deep_scan: bool,
    /// Whether the running scan holds the monitor paused, so finishing it
    /// releases exactly the hold it took. See [`crate::monitor::Monitor::hold`].
    pub scan_held: bool,

    pub bloat: BloatResult,
    pub bloat_running: bool,
    pub bloat_label: String,
    pub bloat_progress: f32,

    pub trace: Vec<String>,
    pub tracing: bool,

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
    pub selected_tweak: Option<usize>,
    /// The read this app has asked for most recently. Bumped by every
    /// `refresh_tweaks`; a result carrying an older number is a read that an
    /// apply overtook, and is dropped.
    tweaks_gen: u64,
    /// The read that produced what `tweak_states` currently holds. Behind
    /// `tweaks_gen` means a read is in flight and the list on screen is stale.
    tweaks_shown_gen: u64,

    /// What the Wi-Fi card can hear around it, and the channel advice read
    /// off it. Empty until the first scan is asked for.
    pub air: crate::probe::airscan::AirScan,
    pub air_scanning: bool,
    /// Whether the optimise list shows the tweaks that cannot be applied
    /// on this machine. They are shown by default, because "this one is
    /// not on offer here" is an answer; hiding them is for once that has
    /// been read.
    pub show_unavailable: bool,
    /// Row id of the outage whose cause panel is open, if any.
    pub selected_outage: Option<i64>,
    /// The open outage's context: fetched and parsed once when the selection
    /// changes, not on every frame. 33 KB of JSON per outage, and the panel
    /// used to parse it twice a frame at 60 Hz.
    pub outage_detail: Option<history::OutageDetail>,
    /// The event log read for one outage, kept so opening an entry launches
    /// `wevtutil` once rather than on every frame it stays open.
    pub syslog: Option<(i64, Vec<crate::probe::eventlog::SysEvent>)>,
    /// The outage a read is currently running for.
    pub syslog_pending: Option<i64>,
    pub elevated: bool,
    pub autostart_on: bool,

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
            selected_finding: None,
            scanning: false,
            scan_label: crate::i18n::diag_scan_hint().into(),
            scan_progress: 0.0,
            deep_scan: true,
            scan_held: false,
            bloat: BloatResult::default(),
            bloat_running: false,
            bloat_label: String::new(),
            bloat_progress: 0.0,
            trace: Vec::new(),
            tracing: false,
            chart_range_s: 300.0,
            chart_smooth: false,
            hidden_series: std::collections::HashSet::new(),
            chart_cache: None,
            card_cache: None,
            tweak_states: Vec::new(),
            tweaks_gen: 0,
            tweaks_shown_gen: 0,
            selected_tweak: None,
            air: Default::default(),
            air_scanning: false,
            show_unavailable: true,
            selected_outage: None,
            outage_detail: None,
            syslog: None,
            syslog_pending: None,
            elevated: crate::optimize::is_elevated(),
            autostart_on: crate::autostart::is_enabled(),
            update: Default::default(),
            update_banner: false,
            toast: None,
            tx,
            rx,
            quit,
            tray,
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
        self.tweaks_gen += 1;
        let gen = self.tweaks_gen;
        std::thread::spawn(move || {
            let states = crate::optimize::all()
                .iter()
                .map(|t| {
                    let s = t.read(&net);
                    (t.id().to_string(), s.text, s.optimal)
                })
                .collect();
            let _ = tx.send(Job::TweakStates(gen, states));
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

    fn drain_jobs(&mut self) {
        while let Ok(job) = self.rx.try_recv() {
            match job {
                Job::ScanProgress(label, frac) => {
                    self.scan_label = label;
                    self.scan_progress = frac;
                }
                Job::ScanDone(scan) => {
                    let scan = *scan;
                    self.findings = scan.findings;
                    self.verdict = scan.verdict;
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
                Job::BloatProgress(label, frac) => {
                    self.bloat_label = label;
                    self.bloat_progress = frac;
                }
                Job::BloatDone(res) => {
                    self.bloat = *res;
                    self.bloat_running = false;
                    self.bloat_label = crate::i18n::bloat_test_done().into();
                    self.bloat_progress = 1.0;
                    self.monitor.release();
                }
                Job::AirDone(scan) => {
                    self.air = *scan;
                    self.air_scanning = false;
                    self.monitor.release();
                }
                Job::Traceroute(lines) => {
                    self.trace = lines;
                    self.tracing = false;
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
                Job::TweakStates(gen, states) => {
                    // A read started before the last apply describes the
                    // machine as it was, not as it is. Showing it would put a
                    // just-applied tweak back in the "worth changing" column.
                    if gen == self.tweaks_gen {
                        self.tweaks_shown_gen = gen;
                        self.tweak_states = states;
                    }
                }
                Job::SysLog(id, events) => {
                    if self.syslog_pending == Some(id) {
                        self.syslog_pending = None;
                        self.syslog = Some((id, events));
                    }
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
        self.drain_jobs();
        self.drain_snapshots();

        // The monitor produces a sample per second; repainting on that cadence
        // keeps the chart live without spinning the GPU.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        // All three panels indent to the same GUTTER, so the header, the tab
        // labels and whatever the tab draws share one left edge. They were
        // at 16, 12 and 14 before, which is not a visible misalignment so
        // much as a permanent faint wrongness down the side of the window.
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::symmetric(GUTTER, S_MD)))
            .show(ctx, |ui| self.header(ui));

        egui::TopBottomPanel::top("tabs")
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::symmetric(GUTTER, 0.0)))
            .show(ctx, |ui| {
                // Wrapped, not a plain row: six tab names in a narrow window
                // run past the right edge, and the ones that fall off are
                // Outage history and Settings -- navigation that vanishes
                // rather than moving is navigation that looks missing.
                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (Tab::Live, crate::i18n::tab_live()),
                        (Tab::Diagnose, crate::i18n::tab_diagnose()),
                        (Tab::Bloat, crate::i18n::tab_bloat()),
                        (Tab::Optimise, crate::i18n::tab_optimise()),
                        (Tab::History, crate::i18n::tab_history()),
                        (Tab::Settings, crate::i18n::tab_settings()),
                    ] {
                        let selected = self.tab == tab;
                        // Weight and colour carry the selection, and an
                        // underline anchors it to the rule below. The tinted
                        // pill egui gives a selected `selectable_label` reads
                        // as a pressed button, which is the wrong promise for
                        // something that is already the current view.
                        let text = egui::RichText::new(label).size(T_HEAD).color(if selected {
                            FG
                        } else {
                            FG_DIM
                        });
                        let text = if selected { text.strong() } else { text };

                        let response = ui.selectable_label(selected, text);
                        if selected {
                            let r = response.rect;
                            ui.painter().hline(
                                r.x_range(),
                                r.bottom() + 3.0,
                                egui::Stroke::new(2.0_f32, ACCENT),
                            );
                        }
                        if response.clicked() {
                            self.tab = tab;
                            if tab == Tab::Optimise {
                                self.refresh_tweaks();
                            }
                        }
                    }
                });
                ui.add_space(S_SM);

                // The rule that the selected tab's underline sits on. Without
                // it the tab strip and the content below are one undivided
                // field of the same colour.
                let rect = ui.max_rect();
                ui.painter().hline(
                    rect.x_range(),
                    ui.cursor().top(),
                    egui::Stroke::new(1.0_f32, LINE),
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
            if now < until {
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
        let status = self.last.status;

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

        ui.horizontal_top(|ui| {
            ui.add_space(2.0);
            status_dot(ui, status_colour(status), 7.0);
            ui.add_space(S_XS);

            let left_w = (ui.available_width() - reserved).max(220.0);
            ui.allocate_ui_with_layout(
                egui::vec2(left_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(left_w);
                    ui.label(
                        egui::RichText::new(status.headline()).size(T_LEAD).strong().color(FG),
                    );
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
                facts.push(Fact::name(if n.ssid.is_empty() {
                    "—".to_string()
                } else {
                    n.ssid.clone()
                }));

                if let Some(pct) = n.signal_pct {
                    // Signal is the only fact on this line with thresholds, so
                    // it is the only one that gets a colour. Colouring more
                    // than one would mean none of them stood out.
                    let value = match n.rssi_dbm {
                        Some(r) => format!("{pct}% · {r} dBm"),
                        None => format!("{pct}%"),
                    };
                    let colour = if pct >= 67 {
                        GREEN
                    } else if pct >= 34 {
                        YELLOW
                    } else {
                        RED
                    };
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
                facts.push(Fact::name(crate::i18n::word_wired().to_string()));
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
    /// Sits in front of the value, quiet and small. Empty for a value that
    /// names itself, like a network's SSID.
    label: &'static str,
    value: String,
    colour: egui::Color32,
}

impl Fact {
    fn new(label: &'static str, value: String, colour: egui::Color32) -> Self {
        Fact { label, value, colour }
    }

    /// A value that needs no label.
    fn name(value: String) -> Self {
        Fact { label: "", value, colour: FG }
    }
}

/// Draw the connection facts as a wrapping row of labelled values.
///
/// Wrapping rather than truncating: the old single line ran off the right edge
/// of a narrow window and took the gateway with it, and a fact you cannot see
/// is worse than a second row.
fn connection_row(ui: &mut egui::Ui, facts: &[Fact]) {
    ui.horizontal_wrapped(|ui| {
        // Tighter than the app's default item spacing — a label and the value
        // it names have to read as one unit, and at S_SM they read as two.
        ui.spacing_mut().item_spacing.x = S_XS;

        for (i, fact) in facts.iter().enumerate() {
            if i > 0 {
                // A painted rule instead of a `·`. The old separator was the
                // same character used *inside* several of the values, so the
                // boundaries between facts and the punctuation within them
                // were indistinguishable.
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(S_SM, T_META), egui::Sense::hover());
                ui.painter().vline(
                    rect.center().x,
                    egui::Rangef::new(rect.top() + 1.0, rect.bottom() - 1.0),
                    egui::Stroke::new(1.0_f32, LINE),
                );
            }

            // One widget per fact, not two.
            //
            // A wrapping row breaks between widgets, so a label and its value
            // drawn separately are two things the layout is free to put on
            // different lines -- and it did, leaving "adapter" at the end of
            // one line and "Wi-Fi" alone at the start of the next. Composing
            // both runs into a single laid-out job makes the pair atomic:
            // the row can wrap around it but never through it.
            let mut job = egui::text::LayoutJob::default();
            let gap = if fact.label.is_empty() {
                0.0
            } else {
                append(&mut job, 0.0, fact.label, T_MICRO, FG_DIM, false);
                // The gap goes in as the value's leading space rather than as
                // a run containing a space character: a whitespace-only run
                // between two runs of different fonts collapsed to nothing,
                // and "karta" ran straight into "Wi-Fi".
                S_XS
            };
            // Monospaced, like every other measurement in the app: these
            // refresh as the link changes, and proportional digits make the
            // whole row shuffle sideways when one of them does.
            append(&mut job, gap, &fact.value, T_META, fact.colour, true);
            ui.label(job);
        }
    });
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

/// Below this width a row of things laid out side by side stops fitting and
/// has to wrap or stack instead.
///
/// One number, shared, because a layout that breaks at a different width in
/// each tab reads as a bug rather than as a design. It is measured against
/// the width actually available for content, not the window, so a panel
/// inside a panel gets the same treatment.
pub const NARROW: f32 = 860.0;

pub fn is_narrow(ui: &egui::Ui) -> bool {
    ui.available_width() < NARROW
}

/// Small stat card used on the live and load-test tabs.
/// How wide a tooltip carrying prose is allowed to get.
///
/// egui lays a tooltip out on one line unless it is told not to, and these
/// are paragraphs. Around this width a line holds some sixty characters,
/// which is the span the eye can return from without losing its place.
pub const TIP_WIDTH: f32 = 380.0;

/// Prose in a tooltip, laid out to be read rather than scanned.
///
/// The text arrives as paragraphs separated by a blank line and is drawn as
/// paragraphs: a four-sentence explanation set as one block at the caption
/// size was technically present and practically unread. The opening paragraph
/// answers "what is this" and is set in the primary colour; what follows is
/// the detail, and is dimmer so the eye can stop after the first if that was
/// all it needed.
pub fn tip_prose(ui: &mut egui::Ui, text: &str) {
    ui.set_max_width(TIP_WIDTH);
    for (i, para) in text.split("\n\n").enumerate() {
        if i > 0 {
            ui.add_space(S_SM);
        }
        ui.label(egui::RichText::new(para).size(T_BODY).color(if i == 0 { FG } else { FG_DIM }));
    }
}

/// A tooltip's title: what the thing is called, over the prose about it.
pub fn tip_heading(ui: &mut egui::Ui, text: &str) {
    ui.set_max_width(TIP_WIDTH);
    ui.label(egui::RichText::new(text).size(T_HEAD).color(FG).strong());
    ui.add_space(S_XS);
    ui.separator();
    ui.add_space(S_SM);
}

/// The common case: a card sized to its own content, with no explanation.
pub fn stat_card(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    sub: &str,
    colour: egui::Color32,
) -> egui::Response {
    stat_card_ex(ui, label, value, sub, colour, None, "")
}

/// One headline number, its name, and a line of context under it.
///
/// `width` pins the card's inner width, which is what a row of cards laid out
/// in columns needs: left to size themselves, cards came out different widths
/// depending on how many digits their value happened to have that second, and
/// the row stopped being a row.
///
/// `tip` is what the number means — not a repeat of the label. A card says
/// "Jitter, 2.3 ms" to someone who already knows what jitter is and nothing
/// at all to anyone else, and the second group is who this app is for.
pub fn stat_card_ex(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    sub: &str,
    colour: egui::Color32,
    width: Option<f32>,
    tip: &str,
) -> egui::Response {
    let response = egui::Frame::none()
        .fill(BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(S_MD))
        .show(ui, |ui| {
            match width {
                Some(w) => {
                    ui.set_min_width(w);
                    ui.set_max_width(w);
                }
                None => ui.set_min_width(140.0),
            }
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(label).size(T_META).color(FG_DIM));
                // The value is the reason the card exists and it changes every
                // second, so it is the one that most needs its digits to stay
                // in place between frames.
                ui.label(figure(value, T_METRIC, colour).strong());
                ui.add_space(S_XS * 0.5);
                // A card with nothing to say on the third line still keeps the
                // line. Without it that card is shorter than the ones beside
                // it, and a row of cards at different heights reads as a
                // layout fault rather than as a card with less to say.
                let sub = if sub.is_empty() { "\u{a0}" } else { sub };
                ui.label(egui::RichText::new(sub).size(T_MICRO).color(FG_DIM));
            });
        })
        .response;

    if tip.is_empty() {
        response
    } else {
        response.on_hover_ui(|ui| {
            tip_heading(ui, label);
            tip_prose(ui, tip);
        })
    }
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

// Every button in the app was `ui.button`, which means every button looked the
// same: the export sitting next to the diagnostic sitting next to the dismiss,
// all in the same grey, all claiming the same weight. A row of equals is a row
// with no entry point — the eye has to read all of it to find the one thing it
// came for. These four levels exist so a row can say which button that is.
//
// They are painted rather than configured through `Visuals`, because egui
// derives hover and pressed from the widget visuals, and an explicit `fill()`
// on a `Button` freezes it in every state — the button stops answering the
// pointer. A button that does not react to the cursor does not read as
// clickable, and that is the one thing it has to say.

/// One height for every button, so a row of them shares a baseline and a
/// centre line regardless of what each one is labelled.
pub const BTN_H: f32 = 30.0;
/// Matches the 6.0 the stat cards and the note panels already use.
pub const BTN_R: f32 = 6.0;
/// Horizontal breathing room inside a button, on the same spacing scale.
const BTN_PAD_X: f32 = S_MD;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    /// The action the row exists for. At most one per row — a second primary
    /// makes both of them secondary.
    Primary,
    /// A real action that is not the point of the row.
    Secondary,
    /// A side action that should stay legible without competing: dismiss,
    /// export, toggle a view.
    Ghost,
    /// Changes the machine, or cannot be taken back. Outlined rather than
    /// filled: it should be findable, not inviting.
    // Unused until the optimise tab is moved over — that is where the buttons
    // that write to the registry live.
    #[allow(dead_code)]
    Danger,
}

/// Fill, stroke and text for a button in a given state.
fn btn_colours(
    emphasis: Emphasis,
    enabled: bool,
    hovered: bool,
    pressed: bool,
) -> (egui::Color32, egui::Color32, egui::Color32) {
    use egui::Color32 as C;
    const CLEAR: egui::Color32 = C::TRANSPARENT;

    if !enabled {
        // Dimmed rather than hidden. "Not now" is an answer, and a control
        // that vanishes when unavailable teaches the user it was never there.
        return (BG2, LINE.linear_multiply(0.6), FG_DIM.linear_multiply(0.55));
    }

    match emphasis {
        Emphasis::Primary => {
            let fill = if pressed {
                C::from_rgb(0x3d, 0x8a, 0xdb)
            } else if hovered {
                C::from_rgb(0x6c, 0xb4, 0xff)
            } else {
                ACCENT
            };
            // Dark text on the accent. White on `#4da3ff` sits near 2.4:1;
            // the page background against it clears 7:1.
            (fill, CLEAR, BG)
        }
        Emphasis::Secondary => {
            let fill = if pressed {
                C::from_rgb(0x3a, 0x41, 0x50)
            } else if hovered {
                C::from_rgb(0x32, 0x38, 0x46)
            } else {
                BG3
            };
            (fill, LINE, FG)
        }
        Emphasis::Ghost => {
            let fill = if pressed {
                BG3
            } else if hovered {
                BG2
            } else {
                CLEAR
            };
            // The label lifts to full strength on hover, which is most of what
            // tells you a ghost button is a button at all.
            (fill, CLEAR, if hovered || pressed { FG } else { FG_DIM })
        }
        Emphasis::Danger => {
            let fill = if pressed {
                C::from_rgb(0x3a, 0x23, 0x28)
            } else if hovered {
                C::from_rgb(0x2e, 0x1e, 0x22)
            } else {
                CLEAR
            };
            (fill, RED.linear_multiply(0.55), RED)
        }
    }
}

/// Width a button needs for a label, before any minimum is applied.
///
/// A toggle whose two labels are different lengths resizes as you click it,
/// and every button to its right slides. Measure both, pass the larger as
/// `min_w`, and the row holds still.
pub fn btn_width(ui: &egui::Ui, label: &str) -> f32 {
    let font = egui::FontId::new(T_BODY, egui::FontFamily::Proportional);
    let galley =
        ui.fonts(|f| f.layout_no_wrap(label.to_string(), font, egui::Color32::PLACEHOLDER));
    galley.size().x + BTN_PAD_X * 2.0
}

/// A button at a given emphasis. `min_w` of 0.0 means "fit the label".
pub fn button_ex(
    ui: &mut egui::Ui,
    label: &str,
    emphasis: Emphasis,
    enabled: bool,
    min_w: f32,
) -> egui::Response {
    let font = egui::FontId::new(T_BODY, egui::FontFamily::Proportional);
    // Laid out in PLACEHOLDER so the paint call can supply the colour once the
    // response says which state we are in.
    let galley =
        ui.fonts(|f| f.layout_no_wrap(label.to_string(), font, egui::Color32::PLACEHOLDER));

    let w = (galley.size().x + BTN_PAD_X * 2.0).max(min_w);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(w, BTN_H), sense);

    if ui.is_rect_visible(rect) {
        let (fill, stroke, text) = btn_colours(
            emphasis,
            enabled,
            response.hovered(),
            response.is_pointer_button_down_on(),
        );

        ui.painter().rect(rect, BTN_R, fill, egui::Stroke::new(1.0_f32, stroke));

        // Keyboard focus, drawn outside the button so it never eats into the
        // label or the fill.
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                BTN_R + 2.0,
                egui::Stroke::new(1.0_f32, ACCENT),
            );
        }

        let pos = rect.center() - galley.size() * 0.5;
        ui.painter().galley(pos, galley, text);
    }

    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The common case: enabled, sized to its label.
pub fn button(ui: &mut egui::Ui, label: &str, emphasis: Emphasis) -> egui::Response {
    button_ex(ui, label, emphasis, true, 0.0)
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
