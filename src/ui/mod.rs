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

use std::sync::mpsc;
use std::sync::Arc;

use eframe::egui;

use crate::bandwidth::BloatResult;
use crate::diagnose::Finding;
use crate::monitor::{Monitor, Snapshot, Status};
use crate::probe::netstate::NetState;
use crate::settings::Settings;
use crate::store::Store;

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
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(radius * 2.0, radius * 2.0),
        egui::Sense::hover(),
    );
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
    ScanDone(Vec<Finding>),
    BloatProgress(String, f32),
    BloatDone(Box<BloatResult>),
    Traceroute(Vec<String>),
    AirDone(Box<crate::probe::airscan::AirScan>),
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
    pub selected_finding: Option<usize>,
    pub scanning: bool,
    pub scan_label: String,
    pub scan_progress: f32,

    pub bloat: BloatResult,
    pub bloat_running: bool,
    pub bloat_label: String,
    pub bloat_progress: f32,

    pub trace: Vec<String>,
    pub tracing: bool,

    pub tweak_states: Vec<(String, String, Option<bool>)>,
    pub selected_tweak: Option<usize>,

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
    pub elevated: bool,
    pub autostart_on: bool,

    pub toast: Option<(String, egui::Color32, f64)>,
    pub last_status: Status,
    pub outage_started: Option<f64>,

    pub tx: mpsc::Sender<Job>,
    pub rx: mpsc::Receiver<Job>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, store: Arc<Store>, settings: Settings) -> Self {
        apply_theme(&cc.egui_ctx);
        let monitor = Monitor::start(Arc::clone(&store), settings.clone());
        let (tx, rx) = mpsc::channel();

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
            selected_finding: None,
            scanning: false,
            scan_label: crate::i18n::diag_scan_hint().into(),
            scan_progress: 0.0,
            bloat: BloatResult::default(),
            bloat_running: false,
            bloat_label: String::new(),
            bloat_progress: 0.0,
            trace: Vec::new(),
            tracing: false,
            tweak_states: Vec::new(),
            selected_tweak: None,
            air: Default::default(),
            air_scanning: false,
            show_unavailable: true,
            selected_outage: None,
            elevated: crate::optimize::is_elevated(),
            autostart_on: crate::autostart::is_enabled(),
            toast: None,
            last_status: Status::Ok,
            outage_started: None,
            tx,
            rx,
        };
        app.refresh_tweaks();
        app
    }

    pub fn refresh_tweaks(&mut self) {
        let net = self.net.clone();
        self.tweak_states = crate::optimize::all()
            .iter()
            .map(|t| {
                let s = t.read(&net);
                (t.id().to_string(), s.text, s.optimal)
            })
            .collect();
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
                Job::ScanDone(findings) => {
                    self.findings = findings;
                    self.selected_finding = None;
                    self.scanning = false;
                    self.scan_label = crate::i18n::diag_scan_done().into();
                    self.scan_progress = 1.0;
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
                    self.monitor.set_paused(false);
                }
                Job::AirDone(scan) => {
                    self.air = *scan;
                    self.air_scanning = false;
                    self.monitor.set_paused(false);
                }
                Job::Traceroute(lines) => {
                    self.trace = lines;
                    self.tracing = false;
                }
            }
        }
    }

    fn drain_snapshots(&mut self, now: f64) {
        let mut newest = None;
        while let Ok(snap) = self.monitor.rx.try_recv() {
            newest = Some(snap);
        }
        let Some(snap) = newest else { return };

        // Announce a change of verdict — this is what the user needs to see
        // even if they were not looking at the window.
        if snap.status != self.last_status {
            if snap.status != Status::Ok {
                self.outage_started = Some(snap.ts);
                let text = format!(
                    "{}{}",
                    snap.status.headline(),
                    if snap.note.is_empty() { String::new() } else { format!(" — {}", snap.note) }
                );
                self.toast(text, status_colour(snap.status), now);
            } else if self.last_status != Status::Ok {
                let secs = self.outage_started.map(|s| snap.ts - s).unwrap_or(0.0);
                self.toast(
                    crate::i18n::toast_restored(secs),
                    GREEN,
                    now,
                );
                self.outage_started = None;
            }
            self.last_status = snap.status;
        }

        self.net = snap.net.clone();
        self.last = snap;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        self.drain_jobs();
        self.drain_snapshots(now);

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
                ui.horizontal(|ui| {
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
                        let text = egui::RichText::new(label)
                            .size(T_HEAD)
                            .color(if selected { FG } else { FG_DIM });
                        let text = if selected { text.strong() } else { text };

                        let response = ui.selectable_label(selected, text);
                        if selected {
                            let r = response.rect;
                            ui.painter().hline(
                                r.x_range(),
                                r.bottom() + 3.0,
                                egui::Stroke::new(2.0, ACCENT),
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
                    egui::Stroke::new(1.0, LINE),
                );
            });

        if let Some((text, colour, until)) = self.toast.clone() {
            if now < until {
                egui::TopBottomPanel::bottom("toast")
                    .frame(egui::Frame::none().fill(BG2).inner_margin(egui::Margin::symmetric(GUTTER, S_SM)))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            status_dot(ui, colour, 5.0);
                            ui.add_space(S_XS);
                            ui.label(egui::RichText::new(text).size(T_BODY).color(FG));
                            if ui.button(crate::i18n::btn_dismiss()).clicked() {
                                self.toast = None;
                            }
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
        ui.horizontal(|ui| {
            status_dot(ui, status_colour(status), 7.0);
            ui.add_space(S_XS);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(status.headline())
                        .size(T_LEAD)
                        .strong()
                        .color(FG),
                );
                ui.label(egui::RichText::new(self.connection_line()).size(T_BODY).color(FG_DIM));
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.elevated {
                    ui.label(egui::RichText::new(crate::i18n::hdr_administrator()).size(T_META).color(GREEN));
                } else {
                    if ui.button(crate::i18n::hdr_restart_elevated()).clicked() {
                        match crate::autostart::relaunch_elevated() {
                            Ok(()) => std::process::exit(0),
                            Err(e) => {
                                let now = ui.input(|i| i.time);
                                self.toast(e.to_string(), RED, now);
                            }
                        }
                    }
                    ui.label(
                        egui::RichText::new(crate::i18n::hdr_standard_mode())
                            .size(T_META)
                            .color(FG_DIM),
                    );
                }
            });
        });
    }

    fn connection_line(&self) -> String {
        let n = &self.net;
        if n.adapter_name.is_empty() {
            return crate::i18n::hdr_no_adapter().into();
        }
        match n.medium {
            crate::probe::netstate::Medium::Wifi => crate::i18n::conn_line_wifi(
                &n.adapter_name,
                if n.ssid.is_empty() { "?" } else { &n.ssid },
                n.signal_pct.unwrap_or(0),
                &n.rssi_dbm.map(|r| format!(" ({r} dBm)")).unwrap_or_default(),
                &n.channel.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
                &n.phy,
                n.rx_mbps.unwrap_or(0),
                &n.gateway.map(|g| g.to_string()).unwrap_or_else(|| "?".into()),
            ),
            _ => crate::i18n::conn_line_wired(
                &n.adapter_name,
                n.link_speed_mbps,
                &n.gateway.map(|g| g.to_string()).unwrap_or_else(|| "?".into()),
                &n.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", "),
            ),
        }
    }
}

/// Small stat card used on the live and load-test tabs.
pub fn stat_card(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    sub: &str,
    colour: egui::Color32,
) {
    egui::Frame::none()
        .fill(BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(S_MD))
        .show(ui, |ui| {
            ui.set_min_width(140.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(label).size(T_META).color(FG_DIM));
                // The value is the reason the card exists and it changes every
                // second, so it is the one that most needs its digits to stay
                // in place between frames.
                ui.label(figure(value, T_METRIC, colour).strong());
                ui.add_space(S_XS * 0.5);
                ui.label(egui::RichText::new(sub).size(T_MICRO).color(FG_DIM));
            });
        });
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
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.window_stroke = egui::Stroke::new(1.0, LINE);

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

    ctx.set_style(style);
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
