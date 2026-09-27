//! Outage history: when, how long, whose fault — and, once an entry is
//! selected, why.
//!
//! The table alone answers "did it drop" and nothing else. Every row carries
//! the connection state and the sweeps that preceded it, so selecting one
//! opens the reasoning: the cause, the evidence it was read from, the lead-up
//! plotted, and a button straight to the fix where one exists.

use eframe::egui;
use egui_plot::{HLine, Line, Plot, PlotPoints, Points, Polygon, VLine};

use super::{
    button, button_ex, card, figure, legend_row, y_steps, App, Emphasis, Key, Tab, BG2, BG3, FG,
    FG_DIM, GREEN, RED, SERIES_COLOURS, S_LG, S_MD, S_SM, S_XS, T_BODY, T_HEAD, T_META, T_TITLE,
    YELLOW,
};
use crate::cause::{self, Cause, Confidence, Evidence};
use crate::clock::format_datetime;
use crate::i18n;
use crate::probe::eventlog::SysEvent;
use crate::store::Event;
use std::rc::Rc;

/// Everything the detail panel needs from one outage's stored context, read
/// from the database once and parsed once.
///
/// Before this, the panel ran `events_since(24h)` and `recent_events(300)`
/// with the context columns attached, then parsed the selected row's 33 KB of
/// JSON twice per frame — once for the cause rules and once for the lead-up
/// plot — cloning the sweep series each time.
pub struct OutageDetail {
    /// The row this was read for. A different selection throws it away.
    pub id: i64,
    /// `None` when the row carries no context, or it did not parse.
    pub evidence: Option<Evidence>,
    /// State when the outage opened, for the "failed on" block.
    pub state: Option<serde_json::Value>,
    /// State it recovered into, for the "came back into" block.
    pub recovery: Option<serde_json::Value>,
    /// Everything the panel reads from the other tables, and the verdict
    /// drawn from it. `None` until the first frame that shows the outage.
    derived: Option<Derived>,
}

/// What the detail panel reads around one outage, held between frames.
///
/// All of this used to be read on every frame: the tweak log, the router's
/// answers, the cause rules and, heaviest by far, every sample from the start
/// of the lead-up to a minute after the end. Behind an eight-hour outage that
/// is 170 000 rows, re-read sixty times a second while the pointer was over
/// the plot.
struct Derived {
    /// The inputs the causes are drawn from. Any change re-reads them.
    key: DerivedKey,
    built_at: f64,
    tweaks: Vec<crate::store::TweakLogRow>,
    log: Vec<SysEvent>,
    causes: Vec<Cause>,
    /// Every sample read so far, unthinned. While the outage is still open
    /// only the rows after `raw.to` are read, so an eight-hour outage costs
    /// five seconds of rows per refresh rather than eight hours.
    raw: RawLead,
    series: LeadSeries,
}

/// The samples behind the lead-up plot, relative to the start of the outage.
#[derive(Default)]
struct RawLead {
    /// The newest sample taken in, which the next read starts after. Not the
    /// end of the span asked for: a sweep's rows carry the moment it began
    /// and land when it ends, so one running at read time is not there yet
    /// and must not be skipped past. Sweeps are written one at a time, in
    /// order, so everything up to this one is.
    newest: Option<f64>,
    router: Vec<[f64; 2]>,
    router_lost: Vec<f64>,
    /// The internet is the best of the public anchors in each second, the
    /// way the monitor reads it: one slow anchor is not the internet being
    /// slow. Keyed by the second, so rows read later join theirs.
    net: std::collections::BTreeMap<i64, (Option<f64>, bool)>,
}

impl RawLead {
    /// Takes in samples newer than any already held.
    fn absorb(&mut self, samples: &[(f64, String, Option<f64>, bool)], t0: f64) {
        let after = self.newest;
        for (ts, target, rtt_ms, ok) in samples {
            if after.is_some_and(|a| *ts <= a) {
                continue;
            }
            self.newest = Some(self.newest.map_or(*ts, |n| n.max(*ts)));
            let x = ts - t0;
            match target.as_str() {
                "gateway" => match (ok, rtt_ms) {
                    (true, Some(ms)) => self.router.push([x, *ms]),
                    _ => self.router_lost.push(x),
                },
                "cloudflare" | "google" => {
                    let slot = self.net.entry(x.round() as i64).or_insert((None, false));
                    if let (true, Some(ms)) = (ok, rtt_ms) {
                        slot.0 = Some(slot.0.map_or(*ms, |best: f64| best.min(*ms)));
                    }
                    slot.1 |= ok;
                }
                _ => {}
            }
        }
    }
}

#[derive(PartialEq)]
struct DerivedKey {
    ts_end: Option<f64>,
    kind: String,
    /// Whether the event log read had landed. It arrives after the first
    /// frame, and the causes read it.
    log_ready: bool,
    /// The outage list the cause rules compare against: its length and its
    /// newest row, which is where a change shows.
    history: (usize, Option<i64>),
}

/// While an outage is open, or less than a minute closed, its plot still
/// grows at the right edge. That is re-read this often.
const LIVE_REFRESH_S: f64 = 5.0;

/// The most points either line of the lead-up plot is drawn with. An hour at
/// one sweep a second is 3600 per line, and egui re-tessellates and hit-tests
/// all of them every frame; past this the plot is thinned.
const MAX_PLOT_POINTS: usize = 2000;

/// The lead-up plot's series, relative to the start of the outage.
#[derive(Default)]
struct LeadSeries {
    from: f64,
    to: f64,
    router: Vec<[f64; 2]>,
    router_lost: Vec<f64>,
    internet: Vec<[f64; 2]>,
    internet_lost: Vec<f64>,
    rssi: Vec<[f64; 2]>,
    /// The stored context had no lead-up: an older row, or one pruned past
    /// the sample retention.
    no_lead: bool,
}

/// Reads and parses the selected outage's context, unless it is already in
/// hand. This is the only place the heavy columns are ever fetched.
fn ensure_detail(app: &mut App, id: i64) {
    if matches!(&app.outage_detail, Some(d) if d.id == id) {
        return;
    }
    let ctx = app.store.event_context(id);
    app.outage_detail = Some(OutageDetail {
        id,
        evidence: ctx.as_ref().and_then(Evidence::from_context),
        state: ctx.as_ref().and_then(|c| c.context_json()),
        recovery: ctx.as_ref().and_then(|c| c.context_end_json()),
        derived: None,
    });
}

/// Reads the tables around the outage and runs the cause rules, unless what
/// is in hand was built from the same inputs and is not still growing.
fn ensure_derived(app: &App, detail: &mut OutageDetail, event: &Event, events: &[Event]) {
    let now = crate::store::now();
    let log = match &app.syslog {
        Some((id, log)) if *id == event.id => Some(log),
        _ => None,
    };
    let key = DerivedKey {
        ts_end: event.ts_end,
        kind: event.kind.clone(),
        log_ready: log.is_some(),
        history: (events.len(), events.first().map(|e| e.id)),
    };
    let settled = event.ts_end.is_some_and(|end| now >= end + 60.0);
    if let Some(d) = &detail.derived {
        if d.key == key && (settled || now - d.built_at < LIVE_REFRESH_S) {
            return;
        }
    }

    // Changes applied in the hour before the outage: the correlation the app
    // has always had the data for and never drawn.
    let tweaks = app.store.tweaks_between(event.ts_start - 3600.0, event.ts_start);
    let log = log.cloned().unwrap_or_default();
    let router = app.store.router_between(
        event.ts_start - cause::ROUTER_MARGIN_S,
        event.ts_end.unwrap_or(event.ts_start) + cause::ROUTER_MARGIN_S,
    );
    let causes = cause::analyse(event, detail.evidence.as_ref(), events, &tweaks, &log, &router);

    let lead: &[crate::monitor::LeadSample] =
        detail.evidence.as_ref().map(|e| e.lead.as_slice()).unwrap_or(&[]);
    let t0 = event.ts_start;
    let from = lead.first().map(|s| s.ts).unwrap_or(t0 - 180.0);
    // A minute of recovery, but never past the present: for an outage that
    // just ended, the rest of that minute is an empty stretch of axis.
    let to = (event.ts_end.unwrap_or(now) + 60.0).min(now);
    let mut raw = detail.derived.take().map(|d| d.raw).unwrap_or_default();
    raw.absorb(&app.store.samples_between(raw.newest.unwrap_or(from), to), t0);
    let series = lead_series(&raw, lead, t0, from, to);

    detail.derived = Some(Derived { key, built_at: now, tweaks, log, causes, raw, series });
}

/// The plot's series: the samples read so far, thinned to what a plot can
/// draw every frame, and the signal from the lead-up.
///
/// Router and internet latency come from the stored samples rather than the
/// lead-up, because the lead-up stops where the outage begins and the shape
/// of the outage itself is half the evidence.
fn lead_series(
    raw: &RawLead,
    lead: &[crate::monitor::LeadSample],
    t0: f64,
    from: f64,
    to: f64,
) -> LeadSeries {
    let net = &raw.net;
    let internet = net.iter().filter_map(|(x, (ms, _))| ms.map(|ms| [*x as f64, ms])).collect();
    let internet_lost =
        net.iter().filter(|(_, (_, any_ok))| !any_ok).map(|(x, _)| *x as f64).collect();
    let rssi = lead.iter().filter_map(|s| s.rssi_dbm.map(|r| [s.ts - t0, r as f64])).collect();

    let bucket = (to - from) / MAX_PLOT_POINTS as f64;
    LeadSeries {
        from,
        to,
        router: thin(raw.router.clone(), bucket),
        router_lost: thin_marks(raw.router_lost.clone(), bucket),
        internet: thin(internet, bucket),
        internet_lost: thin_marks(internet_lost, bucket),
        rssi,
        no_lead: lead.is_empty(),
    }
}

/// At most one point per `bucket` seconds, and that one the slowest: a
/// thinned plot that dropped the spike would be hiding the evidence. Points
/// already sparse enough are returned untouched.
fn thin(points: Vec<[f64; 2]>, bucket: f64) -> Vec<[f64; 2]> {
    if points.len() <= MAX_PLOT_POINTS || bucket <= 0.0 {
        return points;
    }
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(MAX_PLOT_POINTS + 1);
    let mut slot = i64::MIN;
    for p in points {
        let s = (p[0] / bucket).floor() as i64;
        match out.last_mut() {
            Some(last) if s == slot => {
                if p[1] > last[1] {
                    *last = p;
                }
            }
            _ => {
                slot = s;
                out.push(p);
            }
        }
    }
    out
}

/// At most one lost-ping mark per `bucket` seconds: a run of them is one red
/// band either way, and each mark is a shape egui draws every frame.
fn thin_marks(xs: Vec<f64>, bucket: f64) -> Vec<f64> {
    if xs.len() <= MAX_PLOT_POINTS || bucket <= 0.0 {
        return xs;
    }
    let mut out: Vec<f64> = Vec::with_capacity(MAX_PLOT_POINTS + 1);
    for x in xs {
        if out.last().is_none_or(|last| (x / bucket).floor() != (last / bucket).floor()) {
            out.push(x);
        }
    }
    out
}

/// Width of the outage list beside the detail. Wide enough for a date, a
/// duration and a line of detail; the rest goes to the detail, which is
/// where the reading happens.
const LIST_W: f32 = 340.0;
const ROW_H: f32 = 54.0;

/// The outage list and the last day of it, held between frames.
///
/// Both were queried on every frame, and the tab repaints at 60 Hz whenever
/// it scrolls or the pointer moves. Outages open and close at the sweep's
/// pace, so a second-old list is as current as the data.
pub struct ListCache {
    built_at: f64,
    filter: Option<Filter>,
    /// How many entries the filter matched, of which the newest
    /// [`LIST_MAX`] are in `recent`.
    matched: usize,
    recent: Rc<Vec<Event>>,
    day: Rc<Vec<Event>>,
}

/// Rows the list holds at most. It is laid out in full every frame (see
/// [`list`]), which is fine for this many and not for a year of slowdowns.
const LIST_MAX: usize = 300;

/// The list narrowed to one stretch of time and one kind of trouble, set by
/// clicking a figure on the statistics tab: the figure there says how many,
/// and this shows which.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub from: f64,
    pub to: f64,
    pub kinds: Kinds,
    /// What the list says it is showing.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kinds {
    Any,
    /// Every kind but slowness.
    Breaks,
    Slow,
    One(String),
}

impl Filter {
    fn keeps(&self, e: &Event) -> bool {
        let slow = crate::tally::is_slow(e);
        match &self.kinds {
            Kinds::Any => true,
            Kinds::Breaks => !slow,
            Kinds::Slow => slow,
            Kinds::One(kind) => e.kind == *kind,
        }
    }
}

const LIST_REFRESH_S: f64 = 1.0;

fn list_cache(app: &mut App) -> (Rc<Vec<Event>>, Rc<Vec<Event>>) {
    let now = crate::store::now();
    let fresh = app
        .history_list
        .as_ref()
        .filter(|c| now - c.built_at < LIST_REFRESH_S && c.filter == app.history_filter);
    if let Some(c) = fresh {
        return (Rc::clone(&c.recent), Rc::clone(&c.day));
    }
    // Filtered, the list is the newest matches inside the stretch, not the
    // matches among the newest rows: a week of slowdowns can be all of those.
    let (recent, matched) = match &app.history_filter {
        Some(f) => {
            let mut v = app.store.events_overlapping(f.from, f.to);
            v.retain(|e| f.keeps(e));
            let matched = v.len();
            v.reverse();
            v.truncate(LIST_MAX);
            (v, matched)
        }
        None => {
            let v = app.store.recent_events(LIST_MAX);
            let n = v.len();
            (v, n)
        }
    };
    let c = ListCache {
        built_at: now,
        filter: app.history_filter.clone(),
        matched,
        recent: Rc::new(recent),
        day: Rc::new(app.store.events_since(24.0 * 3600.0)),
    };
    let out = (Rc::clone(&c.recent), Rc::clone(&c.day));
    app.history_list = Some(c);
    out
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let (events, day) = list_cache(app);
    let events = events.as_slice();

    // A selection made before the list refreshed may name a row that is no
    // longer here; dropping it is better than showing the wrong outage.
    if let Some(id) = app.selected_outage {
        if !events.iter().any(|e| e.id == id) {
            app.selected_outage = None;
        }
    }
    if app.selected_outage.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.selected_outage = None;
    }
    anchor_on_change(app, ui);

    // The list's width, animated, so opening an outage slides the list aside
    // rather than swapping one screen for another. Fed the full width while
    // nothing is open, so the slide starts from where the list really was.
    let full = ui.available_width();
    let open = app.selected_outage.and_then(|id| events.iter().find(|e| e.id == id));
    let target = if open.is_some() { LIST_W } else { full };
    let list_w = ui.ctx().animate_value_with_time(egui::Id::new("history_list_w"), target, 0.2);

    let Some(event) = open else {
        // Nothing open: one page, the day's summary at its top and every
        // outage under it across the whole width. The summary scrolls away
        // with the list instead of standing over it.
        anchored_scroll(ui, "history_page", |ui| {
            header(app, ui, events, &day);
            if clear_confirm(app, ui, events) {
                return None;
            }
            if events.is_empty() {
                filter_bar(app, ui);
                card(ui, i18n::hist_list_heading(), |ui| {
                    ui.label(
                        egui::RichText::new(i18n::hist_nothing_logged()).size(T_BODY).color(FG_DIM),
                    );
                });
                return None;
            }
            list(app, ui, events)
        });
        return;
    };

    if super::is_narrow(ui) {
        // One at a time: squeezed side by side into a narrow window, the
        // detail was a few lines tall.
        close_button(app, ui);
        egui::ScrollArea::vertical()
            .id_salt("history_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| detail(app, ui, event, events));
        return;
    }

    egui::SidePanel::left("history_list")
        .resizable(false)
        .exact_width(list_w)
        .show_separator_line(false)
        .frame(egui::Frame::none().inner_margin(egui::Margin { right: S_LG, ..Default::default() }))
        .show_inside(ui, |ui| anchored_scroll(ui, "history_table", |ui| list(app, ui, events)));

    egui::CentralPanel::default().frame(egui::Frame::none()).show_inside(ui, |ui| {
        close_button(app, ui);
        egui::ScrollArea::vertical()
            .id_salt("history_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| detail(app, ui, event, events));
    });
}

/// The way back to the whole list, over the open outage.
fn close_button(app: &mut App, ui: &mut egui::Ui) {
    if button(ui, i18n::btn_back_to_list(), Emphasis::Ghost).on_hover_text("Esc").clicked() {
        app.selected_outage = None;
    }
    ui.add_space(S_SM);
}

/// Where each row of the list was on screen last frame, by outage id.
fn row_y_id() -> egui::Id {
    egui::Id::new("history_row_y")
}

/// The selection last frame, to see it change.
fn prev_selected_id() -> egui::Id {
    egui::Id::new("history_prev_selected")
}

/// A row that has to land at a given height on screen in the view that is
/// about to be drawn.
fn anchor_id() -> egui::Id {
    egui::Id::new("history_anchor")
}

/// When an outage is opened or closed, pins the row it concerns to where it
/// was on screen.
///
/// The full list and the narrowed one are separate scroll areas, and the page
/// has the summary above its rows. Switching between them left the narrowed
/// list at its own old offset and then scrolled it to the clicked row, so the
/// whole list flew up past the pointer. Now the row stays under the pointer
/// and the list is placed around it.
fn anchor_on_change(app: &App, ui: &mut egui::Ui) {
    let prev: Option<Option<i64>> = ui.data(|d| d.get_temp(prev_selected_id()));
    let now = app.selected_outage;
    if prev == Some(now) {
        return;
    }
    ui.data_mut(|d| d.insert_temp(prev_selected_id(), now));
    // Opening pins the row opened; closing pins the one that was open.
    let Some(key) = now.or(prev.flatten()) else {
        return;
    };
    let rows: Vec<(i64, f32)> = ui.data(|d| d.get_temp(row_y_id())).unwrap_or_default();
    if let Some((_, y)) = rows.iter().find(|(id, _)| *id == key) {
        ui.data_mut(|d| d.insert_temp(anchor_id(), (key, *y)));
    }
}

/// A vertical scroll area whose content can ask, by returning how far off a
/// row is, to be shifted by that much on the next frame. The shift is set, not
/// scrolled to, so there is nothing to watch travel.
fn anchored_scroll(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui) -> Option<f32>) {
    let fix = egui::Id::new(("history_fix", salt));
    let mut area = egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]);
    if let Some(y) = ui.data_mut(|d| d.remove_temp::<f32>(fix)) {
        area = area.vertical_scroll_offset(y);
    }
    let out = area.show(ui, add);
    if let Some(delta) = out.inner {
        ui.data_mut(|d| d.insert_temp(fix, (out.state.offset.y + delta).max(0.0)));
        ui.ctx().request_repaint();
    }
}

/// Whether the clear-history question is on screen.
fn confirm_id() -> egui::Id {
    egui::Id::new("history_confirm_clear")
}

/// The day in one sentence, what the entries hold, and the report.
fn header(app: &mut App, ui: &mut egui::Ui, events: &[Event], day: &[Event]) {
    let can_clear = events.iter().any(|e| e.ts_end.is_some());
    let (text, colour) = if day.is_empty() {
        let watched =
            app.last.observed_from.map_or(0.0, |from| (crate::store::now() - from).max(0.0));
        let (text, good) = quiet_headline(watched);
        (text, if good { GREEN } else { FG_DIM })
    } else {
        // This headline is rebuilt on every frame, so the choice has to be a
        // function of the data alone: see `dominant_scope`.
        let worst =
            crate::monitor::dominant_scope(day.iter().map(|e| e.scope.as_str())).unwrap_or("");
        let where_text = match worst {
            "lan" => i18n::hist_where_lan(),
            "adapter" => i18n::hist_where_adapter(),
            "isp" => i18n::hist_where_isp(),
            "dns" => i18n::hist_where_dns(),
            _ => i18n::hist_where_other(),
        };
        (i18n::hist_summary(day.len(), where_text), RED)
    };

    ui.horizontal_wrapped(|ui| {
        super::status_dot(ui, colour, 5.0);
        ui.add_space(S_XS);
        ui.label(egui::RichText::new(&text).size(T_TITLE).strong().color(FG));
    });
    ui.add_space(S_XS);
    ui.label(egui::RichText::new(i18n::hist_blurb()).size(T_META).color(FG_DIM));
    ui.add_space(S_MD);

    // The toolbar has a line of its own. Beside the headline it ran into the
    // headline's text as soon as the sentence was a long one.
    ui.horizontal(|ui| {
        report_row(app, ui);
        // Apart from the report, at the far edge: deleting the evidence and
        // saving a copy of it are not neighbouring choices.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button_ex(ui, i18n::hist_btn_clear(), Emphasis::Ghost, can_clear, 0.0).clicked() {
                ui.data_mut(|d| d.insert_temp(confirm_id(), true));
            }
        });
    });
    ui.add_space(S_MD);
}

/// The question before the history is deleted, in place of the list while
/// it is asked. Returns whether it is on screen.
///
/// Asked in the page rather than in a pop-up, and with the report named in
/// it: the outage history is the evidence this app exists to keep, and the
/// moment before deleting it is the one moment the way to keep a copy is
/// worth pointing at.
fn clear_confirm(app: &mut App, ui: &mut egui::Ui, events: &[Event]) -> bool {
    let asking = ui.data(|d| d.get_temp::<bool>(confirm_id())).unwrap_or(false);
    if !asking {
        return false;
    }

    let mut close = false;
    egui::Frame::none()
        .fill(BG2)
        .rounding(6.0)
        .stroke(egui::Stroke::new(1.0_f32, RED.linear_multiply(0.55)))
        .inner_margin(egui::Margin::same(S_LG))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                super::status_dot(ui, RED, 5.0);
                ui.add_space(S_XS);
                ui.label(
                    egui::RichText::new(i18n::hist_btn_clear().trim_end_matches('…'))
                        .size(T_HEAD)
                        .strong()
                        .color(FG),
                );
            });
            ui.add_space(S_SM);
            ui.label(egui::RichText::new(i18n::hist_clear_confirm()).size(T_BODY).color(FG));
            if events.iter().any(|e| e.ts_end.is_none()) {
                ui.label(
                    egui::RichText::new(i18n::hist_clear_keeps_running())
                        .size(T_META)
                        .color(FG_DIM),
                );
            }
            ui.add_space(S_MD);
            ui.horizontal(|ui| {
                if button(ui, i18n::hist_btn_clear_confirm(), Emphasis::Danger).clicked() {
                    let now = ui.input(|i| i.time);
                    match app.store.clear_events() {
                        Ok(n) => {
                            // Everything cached against a row that no longer
                            // exists goes with it.
                            app.selected_outage = None;
                            app.outage_detail = None;
                            app.syslog = None;
                            app.history_list = None;
                            app.toast(i18n::hist_cleared(n), GREEN, now);
                        }
                        Err(e) => app.toast(i18n::hist_clear_failed(&e.to_string()), RED, now),
                    }
                    close = true;
                }
                if button(ui, i18n::live_btn_report(), Emphasis::Secondary).clicked() {
                    let range = app.report_range;
                    super::report::save_in_background(app, range);
                }
                if button(ui, i18n::hist_btn_cancel(), Emphasis::Ghost).clicked() {
                    close = true;
                }
            });
        });
    if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|d| d.insert_temp(confirm_id(), false));
    }
    true
}

/// How an outage is coloured: red where the connection was gone, yellow
/// where it was only worse.
fn scope_colour(e: &Event) -> egui::Color32 {
    match e.scope.as_str() {
        "lan" | "adapter" | "isp" => RED,
        _ => YELLOW,
    }
}

fn duration_text(e: &Event) -> String {
    match e.duration_s() {
        Some(d) => i18n::span(d),
        None => i18n::hist_ongoing().into(),
    }
}

/// What the list is narrowed to, and the way back to all of it.
fn filter_bar(app: &mut App, ui: &mut egui::Ui) {
    let Some(label) = app.history_filter.as_ref().map(|f| f.label.clone()) else {
        return;
    };
    let matched = app.history_list.as_ref().map_or(0, |c| c.matched);
    ui.horizontal_wrapped(|ui| {
        super::status_dot(ui, super::ACCENT, 4.0);
        ui.label(egui::RichText::new(i18n::hist_filtered(&label)).size(T_BODY).strong().color(FG));
        // Said, not left to be noticed: the figure that opened this list
        // counted all of them.
        if matched > LIST_MAX {
            ui.label(
                egui::RichText::new(i18n::hist_filtered_newest(LIST_MAX, matched))
                    .size(T_META)
                    .color(FG_DIM),
            );
        }
        if button(ui, i18n::hist_show_all(), Emphasis::Ghost).clicked() {
            app.history_filter = None;
            app.selected_outage = None;
        }
    });
    ui.add_space(S_SM);
}

/// The outage rows. Returns how far the pinned row is from where it has to
/// be, when a view change pinned one; see [`anchor_on_change`].
fn list(app: &mut App, ui: &mut egui::Ui, events: &[Event]) -> Option<f32> {
    filter_bar(app, ui);
    ui.label(
        egui::RichText::new(format!("{} · {}", i18n::hist_list_heading(), events.len()))
            .size(T_META)
            .color(FG_DIM),
    );
    if app.selected_outage.is_none() {
        ui.label(egui::RichText::new(i18n::hist_select_hint()).size(T_META).color(FG_DIM));
    }
    ui.add_space(S_XS);
    // The frame that finds the pinned row out of place is not shown: the next
    // one, shifted, is. One blank frame instead of one frame of the list in
    // the wrong place.
    let anchor: Option<(i64, f32)> = ui.data_mut(|d| d.remove_temp(anchor_id()));
    if anchor.is_some() {
        ui.set_invisible();
    }
    // Drawn in full rather than a visible slice: the list scrolls with the
    // page above it, and a row has to be laid out to be measured. At most
    // `LIST_MAX` rows, and `list_row` paints nothing for one that is off screen.
    let mut rows = Vec::with_capacity(events.len());
    let mut delta = None;
    for e in events {
        let selected = app.selected_outage == Some(e.id);
        let resp = list_row(ui, e, selected);
        rows.push((e.id, resp.rect.top()));
        if let Some((id, y)) = anchor {
            if id == e.id && (resp.rect.top() - y).abs() > 0.5 {
                delta = Some(resp.rect.top() - y);
            }
        }
        if resp.clicked() {
            // A second click on the open one closes it.
            app.selected_outage = if selected { None } else { Some(e.id) };
        }
        ui.add_space(2.0);
    }
    ui.data_mut(|d| d.insert_temp(row_y_id(), rows));
    delta
}

/// One outage in the list: when and how long on the first line, what kind
/// and the detail on the second.
fn list_row(ui: &mut egui::Ui, e: &Event, selected: bool) -> egui::Response {
    let colour = scope_colour(e);
    let kind = i18n::event_kind(&e.kind);
    let detail = format!("  ·  {}", e.detail);
    let mut sub = vec![(kind.as_str(), FG)];
    if !e.detail.is_empty() {
        sub.push((detail.as_str(), FG_DIM));
    }
    super::list_row(
        ui,
        ROW_H,
        selected,
        colour,
        &format_datetime(e.ts_start),
        true,
        Some((&duration_text(e), colour)),
        &sub,
    )
}

fn detail(app: &mut App, ui: &mut egui::Ui, event: &Event, events: &[Event]) {
    // The widgets below need `&mut App`, so the cached context is lifted out
    // for the duration of the panel and put back at the end. Taking it is not
    // a reload: `ensure_detail` only reads the database when the selection
    // changed.
    ensure_detail(app, event.id);
    let Some(mut cached) = app.outage_detail.take() else { return };
    ensure_derived(app, &mut cached, event, events);
    let detail = &cached;
    let Some(derived) = detail.derived.as_ref() else { return };

    ui.label(
        egui::RichText::new(i18n::hist_cause_for(&format_datetime(event.ts_start)))
            .size(T_TITLE)
            .strong()
            .color(FG),
    );
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(i18n::event_kind(&event.kind))
                .size(T_BODY)
                .color(scope_colour(event)),
        );
        ui.label(egui::RichText::new("·").size(T_BODY).color(FG_DIM));
        // Digits in the figure face; "ongoing" is a word, and set in it
        // looked like a typo.
        let duration = duration_text(event);
        if event.ts_end.is_some() {
            ui.label(figure(duration, T_BODY, FG));
        } else {
            ui.label(egui::RichText::new(duration).size(T_BODY).color(FG));
        }
        if !event.detail.is_empty() {
            ui.label(egui::RichText::new("·").size(T_BODY).color(FG_DIM));
            ui.label(egui::RichText::new(&event.detail).size(T_BODY).color(FG_DIM));
        }
    });
    ui.add_space(S_MD);

    request_log(app, event);
    let (tweaks, log) = (&derived.tweaks, &derived.log);

    card(ui, i18n::hist_cause_heading(), |ui| {
        for c in &derived.causes {
            cause_row(app, ui, c);
        }
    });

    if !tweaks.is_empty() {
        card(ui, i18n::hist_tweaks_heading(), |ui| {
            for t in tweaks {
                ui.horizontal_wrapped(|ui| {
                    ui.label(figure(format_datetime(t.ts), T_META, FG_DIM));
                    ui.label(
                        egui::RichText::new(i18n::tweak_name(&t.tweak_id)).size(T_BODY).color(FG),
                    );
                    ui.label(egui::RichText::new(&t.action).size(T_META).color(FG_DIM));
                });
            }
        });
    }

    card(ui, i18n::hist_leadup_heading(), |ui| {
        lead_up(ui, event, &derived.series);
    });

    // Failure state and recovery state are meant to be compared, so they sit
    // side by side wherever the card allows it. Where it does not, one above
    // the other still compares; two columns of truncated values does not.
    card(ui, i18n::hist_conn_heading(), |ui| {
        let failed_on = |ui: &mut egui::Ui| {
            state_block(ui, i18n::hist_state_heading(), detail.state.as_ref());
        };
        let came_back_into = |ui: &mut egui::Ui| match detail.recovery.as_ref() {
            Some(state) => state_block(ui, i18n::hist_recovery_heading(), Some(state)),
            None => {
                sub_heading(ui, i18n::hist_recovery_heading());
                ui.label(egui::RichText::new(i18n::hist_no_recovery()).size(T_META).color(YELLOW));
            }
        };
        if ui.available_width() < 520.0 {
            failed_on(ui);
            ui.add_space(S_MD);
            came_back_into(ui);
        } else {
            ui.columns(2, |cols| {
                failed_on(&mut cols[0]);
                came_back_into(&mut cols[1]);
            });
        }
    });

    card(ui, i18n::hist_log_heading(), |ui| system_log(app, ui, event, log));

    app.outage_detail = Some(cached);
}

/// A heading inside a card, one step below the card's own title.
fn sub_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(T_BODY).strong().color(FG_DIM));
    ui.add_space(S_XS);
}

/// The headline over an empty last day, and whether it is good news.
///
/// `watched_s` is the current unbroken stretch of watching, which the monitor
/// already keeps, so this costs nothing per frame. It can undercount a day
/// that was watched in pieces; that errs towards saying less.
fn quiet_headline(watched_s: f64) -> (String, bool) {
    if watched_s >= 24.0 * 3600.0 * 0.95 {
        (i18n::hist_none_24h().to_string(), true)
    } else {
        (i18n::f_hist_none_partial(&i18n::span(watched_s)), false)
    }
}

/// The range picker and the button that writes the report for it: every
/// outage in the range with its cause, its evidence and the path it broke on.
fn report_row(app: &mut App, ui: &mut egui::Ui) {
    use super::report::Range;
    ui.label(egui::RichText::new(i18n::hist_report_range()).size(T_BODY).color(FG_DIM));
    // egui sizes a combo box to the default control height, which is lower
    // than the app's buttons, so in a centred row it sat below them. Held to
    // the button height, it shares their line.
    ui.spacing_mut().interact_size.y = super::BTN_H;
    egui::ComboBox::from_id_salt("report_range")
        .selected_text(egui::RichText::new(app.report_range.label()).size(T_BODY))
        .show_ui(ui, |ui| {
            for r in Range::ALL {
                ui.selectable_value(&mut app.report_range, r, r.label());
            }
        });
    let label = if app.report_busy { i18n::hist_report_saving() } else { i18n::live_btn_report() };
    if button_ex(ui, label, Emphasis::Secondary, !app.report_busy, 0.0).clicked() {
        let range = app.report_range;
        super::report::save_in_background(app, range);
    }
}

/// Starts the event log read for a newly selected outage, at most once.
///
/// `wevtutil` is a process launch and a few hundred milliseconds, which is
/// nothing once and unbearable sixty times a second, so the result is cached
/// against the row id and a read already running is left alone.
fn request_log(app: &mut App, event: &Event) {
    let already = matches!(&app.syslog, Some((id, _)) if *id == event.id);
    if already || app.syslog_pending == Some(event.id) {
        return;
    }

    app.syslog_pending = Some(event.id);
    let id = event.id;
    let (from, to) = crate::probe::eventlog::span_around(event);
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(super::Job::SysLog(id, crate::probe::eventlog::window(from, to)));
    });
}

/// The log lines themselves, under the verdicts they produced.
///
/// The causes above are this module's reading of these lines; showing the
/// lines as well is what makes that reading checkable. Ordinary transitions
/// stay in the list next to the faults, because "the machine woke here" is
/// often the line that makes the rest make sense.
fn system_log(app: &App, ui: &mut egui::Ui, event: &Event, log: &[SysEvent]) {
    if app.syslog_pending == Some(event.id) {
        ui.label(egui::RichText::new(i18n::hist_log_loading()).size(T_META).color(FG_DIM));
        return;
    }
    if log.is_empty() {
        ui.label(egui::RichText::new(i18n::hist_log_none()).size(T_META).color(FG_DIM));
        return;
    }

    egui::Grid::new("system_log").num_columns(4).spacing([14.0, 3.0]).show(ui, |ui| {
        for e in log {
            let colour = if e.kind.is_fault() { YELLOW } else { FG_DIM };
            ui.label(figure(i18n::clock_offset(e.offset_from(event.ts_start)), T_META, FG_DIM));
            ui.label(egui::RichText::new(i18n::log_kind(e.kind)).size(T_META).color(colour));
            ui.label(
                egui::RichText::new(format!("{} ({})", e.provider, e.id))
                    .size(T_META)
                    .monospace()
                    .color(FG_DIM),
            );
            // The provider's own fields — the SSID it dropped, the reason it
            // gave. This is the part a support call can be read from.
            ui.label(egui::RichText::new(&e.detail).size(T_META).monospace().color(FG_DIM));
            ui.end_row();
        }
    });
}

fn cause_row(app: &mut App, ui: &mut egui::Ui, c: &Cause) {
    let colour = match c.confidence {
        Confidence::Certain => RED,
        Confidence::Likely => YELLOW,
        Confidence::Possible => FG_DIM,
    };
    egui::Frame::none()
        .fill(BG3)
        .rounding(super::BTN_R)
        .inner_margin(egui::Margin::symmetric(S_MD, S_SM))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(c.title()).size(T_HEAD).strong().color(colour));
                ui.label(
                    egui::RichText::new(format!("({})", c.confidence.label()))
                        .size(T_META)
                        .color(FG_DIM),
                );
                if let Some(tweak) = c.fix_tweak {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button(ui, i18n::hist_btn_fix(), Emphasis::Primary).clicked() {
                            open_tweak(app, tweak);
                        }
                    });
                }
            });
            ui.label(egui::RichText::new(&c.evidence).size(T_META).italics().color(FG_DIM));
            let advice = c.advice();
            if !advice.is_empty() {
                ui.add_space(S_XS);
                ui.label(egui::RichText::new(advice).size(T_META).color(FG));
            }
        });
    ui.add_space(S_XS);
}

/// Jumps to the Optimise tab with the relevant tweak already selected, so the
/// step from "this is why" to "this is the fix" is one click and not a hunt.
fn open_tweak(app: &mut App, tweak_id: &str) {
    if let Some(i) = app.tweak_states.iter().position(|(id, _, _)| id == tweak_id) {
        app.selected_tweak = Some(i);
        app.tab = Tab::Optimise;
    }
}

/// The whole episode on one time axis: the lead-up from the event's own
/// context, the outage and the recovery from the sample table. Time is drawn
/// relative to the start of the outage, so zero is the moment it broke and
/// everything left of it is what led there.
///
/// Two plots, one above the other, sharing the time axis and the cursor.
/// They used to be one plot holding the router in milliseconds and the
/// signal in dBm: two units on one scale, so the signal lay flat along the
/// bottom at -60 and the router flat along zero, and a lost ping was drawn as
/// a fake 250 ms reply. Now each quantity has its own scale, a ping that got
/// no reply is a mark of its own, and the outage is a shaded span rather than
/// a moment to be found.
///
/// The router and the internet side by side is what separates the two
/// stories the table cannot: both lines failing together is this side of the
/// router, the internet failing alone is the provider. The signal below
/// shows whether the Wi-Fi was sliding away first.
fn lead_up(ui: &mut egui::Ui, event: &Event, series: &LeadSeries) {
    let LeadSeries { from, to, router, router_lost, internet, internet_lost, rssi, no_lead } =
        series;
    let (from, to) = (*from, *to);
    let t0 = event.ts_start;
    let end = event.ts_end.unwrap_or_else(crate::store::now);

    if router.is_empty() && router_lost.is_empty() && internet.is_empty() && rssi.is_empty() {
        ui.label(egui::RichText::new(i18n::hist_no_leadup()).size(T_META).color(FG_DIM));
        return;
    }

    // Lost pings sit in a band just above the highest real reply, so they
    // are in view without squashing every real reply towards zero.
    let peak = router.iter().chain(internet).map(|p| p[1]).fold(0.0_f64, f64::max);
    let ceiling = (peak * 1.15).max(20.0);
    let lost_y = ceiling * 1.08;

    let (x_min, x_max) = (from - t0, to - t0);
    // At least a second wide: the monitor stamps start and end to the
    // sweep, and an outage it opened and closed in one sweep would otherwise
    // be a span with no width, drawn as nothing.
    let outage_end = (end - t0).max(1.0);
    let outage = |y_lo: f64, y_hi: f64| {
        Polygon::new(PlotPoints::from(vec![
            [0.0, y_lo],
            [outage_end, y_lo],
            [outage_end, y_hi],
            [0.0, y_hi],
        ]))
        .fill_color(RED.gamma_multiply(0.12))
        .stroke(egui::Stroke::NONE)
        .name(i18n::hist_leadup_outage())
    };
    // One group for both plots: dragging nothing, but hovering one shows
    // the same second in the other.
    let group = egui::Id::new(("lead_up", event.id));
    let router_colour = SERIES_COLOURS[0];
    let net_colour = SERIES_COLOURS[2];
    let signal_colour = SERIES_COLOURS[4];

    ui.label(egui::RichText::new(i18n::hist_leadup_rtt_caption()).size(T_META).color(FG_DIM));
    ui.add_space(S_XS);
    legend_row(
        ui,
        &[
            (i18n::hist_leadup_rtt(), router_colour, Key::Line),
            (i18n::hist_leadup_net(), net_colour, Key::Line),
            (i18n::hist_leadup_lost(), RED, Key::Dot),
            (i18n::hist_leadup_outage(), RED.gamma_multiply(0.35), Key::Span),
        ],
    );
    Plot::new(("lead_up_rtt", event.id))
        .height(170.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .show_x(true)
        .include_x(x_min)
        .include_x(x_max)
        .include_y(0.0)
        .include_y(lost_y * 1.04)
        .y_axis_min_width(44.0)
        .y_grid_spacer(egui_plot::uniform_grid_spacer(y_steps))
        .y_axis_formatter(|mark, _| format!("{:.0}", mark.value))
        .link_axis(group, true, false)
        .link_cursor(group, true, false)
        .label_formatter(|name, p| {
            if name.is_empty() {
                String::new()
            } else if name == i18n::hist_leadup_lost() || name == i18n::hist_leadup_outage() {
                format!("{name}\n{}", i18n::clock_offset(p.x))
            } else {
                format!("{name}: {:.0} ms\n{}", p.y, i18n::clock_offset(p.x))
            }
        })
        .show(ui, |plot| {
            plot.polygon(outage(0.0, lost_y * 1.04));
            plot.vline(VLine::new(0.0).color(RED.linear_multiply(0.6)));
            plot.line(
                Line::new(PlotPoints::from(router.clone()))
                    .name(i18n::hist_leadup_rtt())
                    .color(router_colour),
            );
            plot.line(
                Line::new(PlotPoints::from(internet.clone()))
                    .name(i18n::hist_leadup_net())
                    .color(net_colour),
            );
            // Both kinds of silence in one legend entry and one colour: what
            // matters is that nothing came back, and the row says from whom.
            let lost: Vec<[f64; 2]> = router_lost
                .iter()
                .map(|x| [*x, lost_y])
                .chain(internet_lost.iter().map(|x| [*x, lost_y * 0.96]))
                .collect();
            plot.points(
                Points::new(PlotPoints::from(lost))
                    .radius(2.5)
                    .filled(true)
                    .color(RED)
                    .name(i18n::hist_leadup_lost()),
            );
        });

    ui.add_space(S_SM);
    if rssi.is_empty() {
        ui.label(egui::RichText::new(i18n::hist_leadup_no_signal()).size(T_META).color(FG_DIM));
    } else {
        ui.label(egui::RichText::new(i18n::hist_leadup_rssi_caption()).size(T_META).color(FG_DIM));
        ui.add_space(S_XS);
        legend_row(
            ui,
            &[
                (i18n::hist_leadup_rssi(), signal_colour, Key::Line),
                (i18n::hist_leadup_weak(), YELLOW.gamma_multiply(0.7), Key::Line),
            ],
        );
        // A fixed span from a strong signal to where Wi-Fi gives up, so a
        // drop of 3 dB does not fill the plot and look like a collapse.
        let lo = rssi.iter().map(|p| p[1]).fold(-90.0_f64, f64::min);
        let hi = rssi.iter().map(|p| p[1]).fold(-30.0_f64, f64::max);
        Plot::new(("lead_up_rssi", event.id))
            .height(140.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .include_x(x_min)
            .include_x(x_max)
            .include_y(lo)
            .include_y(hi)
            .y_axis_min_width(44.0)
            .y_grid_spacer(egui_plot::uniform_grid_spacer(y_steps))
            .y_axis_formatter(|mark, _| format!("{:.0}", mark.value))
            .link_axis(group, true, false)
            .link_cursor(group, true, false)
            .x_axis_label(i18n::hist_leadup_axis())
            .label_formatter(|name, p| {
                if name.is_empty() {
                    String::new()
                } else if name == i18n::hist_leadup_outage() {
                    format!(
                        "{name}
{}",
                        i18n::clock_offset(p.x)
                    )
                } else {
                    format!("{name}: {:.0} dBm\n{}", p.y, i18n::clock_offset(p.x))
                }
            })
            .show(ui, |plot| {
                plot.polygon(outage(lo, hi));
                plot.vline(VLine::new(0.0).color(RED.linear_multiply(0.6)));
                // Where the link starts to struggle: below it, the card
                // drops to slow rates and retries.
                plot.hline(
                    HLine::new(-70.0)
                        .color(YELLOW.gamma_multiply(0.7))
                        .style(egui_plot::LineStyle::dashed_dense()),
                );
                plot.line(
                    Line::new(PlotPoints::from(rssi.clone()))
                        .name(i18n::hist_leadup_rssi())
                        .color(signal_colour),
                );
            });
    }

    if *no_lead {
        ui.label(egui::RichText::new(i18n::hist_no_leadup()).size(T_META).color(FG_DIM));
    }
}

/// The stored state, rendered as the fields that mean something to a person.
/// The raw JSON is never shown: it is a storage format, not a report.
fn state_block(ui: &mut egui::Ui, heading: &str, state: Option<&serde_json::Value>) {
    sub_heading(ui, heading);
    let Some(v) = state else {
        ui.label(egui::RichText::new(i18n::hist_no_state()).size(T_META).color(FG_DIM));
        return;
    };

    let rows: Vec<(&str, String)> = [
        ("SSID", v["ssid"].as_str().unwrap_or("").to_string()),
        ("BSSID", v["bssid"].as_str().unwrap_or("").to_string()),
        ("RSSI", fmt_num(&v["rssi_dbm"], " dBm")),
        (i18n::st_signal(), fmt_num(&v["signal_pct"], "%")),
        (i18n::st_channel(), fmt_num(&v["channel"], "")),
        (i18n::st_band(), v["band"].as_str().unwrap_or("").to_string()),
        ("PHY", v["phy"].as_str().unwrap_or("").to_string()),
        ("RX", fmt_num(&v["rx_mbps"], " Mbps")),
        (i18n::st_gateway(), v["gateway"].as_str().unwrap_or("").to_string()),
        ("DNS", join_strs(&v["dns"])),
    ]
    .into_iter()
    .filter(|(_, val)| !val.is_empty())
    .collect();

    egui::Grid::new(heading).num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
        for (k, val) in rows {
            ui.label(egui::RichText::new(k).size(T_META).color(FG_DIM));
            ui.label(egui::RichText::new(val).size(T_META).monospace().color(FG));
            ui.end_row();
        }
    });
}

fn fmt_num(v: &serde_json::Value, unit: &str) -> String {
    match v.as_i64() {
        Some(n) => format!("{n}{unit}"),
        None => String::new(),
    }
}

fn join_strs(v: &serde_json::Value) -> String {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_from_the_statistics_keeps_only_what_its_figure_counted() {
        let ev = |kind: &str| Event {
            id: 0,
            ts_start: 0.0,
            ts_end: Some(1.0),
            kind: kind.into(),
            scope: String::new(),
            detail: String::new(),
        };
        let f = |kinds| Filter { from: 0.0, to: 1.0, kinds, label: String::new() };
        let (slow, down) = (ev("degraded"), ev("isp_down"));
        assert!(f(Kinds::Breaks).keeps(&down) && !f(Kinds::Breaks).keeps(&slow));
        assert!(f(Kinds::Slow).keeps(&slow) && !f(Kinds::Slow).keeps(&down));
        assert!(f(Kinds::Any).keeps(&slow) && f(Kinds::Any).keeps(&down));
        let one = f(Kinds::One("isp_down".into()));
        assert!(one.keeps(&down) && !one.keeps(&ev("lan_down")));
    }

    #[test]
    fn a_long_outage_is_plotted_thinned_without_losing_its_worst_reading() {
        // Eight hours at two sweeps a second: what game mode records.
        let t0: f64 = 100_000.0;
        let (from, to) = (t0 - 180.0, t0 + 8.0 * 3600.0);
        let mut samples = Vec::new();
        let mut ts: f64 = from;
        while ts < to {
            let spike = (ts - (t0 + 3600.0)).abs() < 0.25;
            let rtt = if spike { 900.0 } else { 5.0 };
            samples.push((ts, "gateway".to_string(), Some(rtt), true));
            samples.push((ts, "cloudflare".to_string(), None, false));
            ts += 0.5;
        }
        let mut raw = RawLead::default();
        raw.absorb(&samples, t0);
        let s = lead_series(&raw, &[], t0, from, to);
        assert!(s.router.len() <= MAX_PLOT_POINTS + 1, "{}", s.router.len());
        assert!(s.internet_lost.len() <= MAX_PLOT_POINTS + 1, "{}", s.internet_lost.len());
        assert!(s.router.iter().any(|p| p[1] == 900.0), "the spike survives thinning");
        assert!(s.router.windows(2).all(|w| w[0][0] < w[1][0]), "still in time order");
        assert!(s.no_lead);

        // A short one is drawn exactly as recorded.
        let mut short = RawLead::default();
        short.absorb(&samples[..400], t0);
        assert_eq!(lead_series(&short, &[], t0, from, from + 100.0).router.len(), 200);
    }

    #[test]
    fn an_open_outage_read_in_pieces_is_the_same_as_read_whole() {
        let t0 = 1_000.0;
        let row = |ts: f64, target: &str, ok: bool| (ts, target.to_string(), ok.then_some(4.0), ok);
        let all: Vec<_> = (0..40)
            .flat_map(|i| {
                let ts = t0 - 10.0 + i as f64 * 0.5;
                [row(ts, "gateway", i % 7 != 0), row(ts, "cloudflare", i % 5 != 0)]
            })
            .collect();
        let mut whole = RawLead::default();
        whole.absorb(&all, t0);

        // Each read starts at the newest row already held and so returns it
        // again, as `samples_between` does with its inclusive bounds.
        let mut pieces = RawLead::default();
        pieces.absorb(&all[..30], t0);
        let again = all.iter().position(|r| Some(r.0) == pieces.newest).unwrap();
        pieces.absorb(&all[again..], t0);

        assert_eq!(pieces.router, whole.router);
        assert_eq!(pieces.router_lost, whole.router_lost);
        assert_eq!(pieces.net, whole.net);
    }

    #[test]
    fn value_axis_steps_are_round_and_four_or_five_to_a_plot() {
        let steps = |lo: f64, hi: f64| {
            y_steps(egui_plot::GridInput { bounds: (lo, hi), base_step_size: 0.1 })[1]
        };
        // The two cases that drew no numbers: a 60 dB signal span, and a
        // latency span stretched to 300 ms by one spike.
        assert_eq!(steps(-90.0, -30.0), 20.0);
        assert_eq!(steps(0.0, 300.0), 100.0);
        assert_eq!(steps(0.0, 70.0), 20.0);
        // A flat or empty range still gets a usable step, not zero.
        assert!(steps(5.0, 5.0) > 0.0);
    }

    #[test]
    fn a_quiet_day_is_only_good_news_when_it_was_watched() {
        // Two minutes after a first start the tab said "No outages in the
        // last 24 hours" in green.
        let (text, good) = quiet_headline(120.0);
        assert!(!good);
        assert!(text.contains("2 min"), "{text}");
        assert!(quiet_headline(24.0 * 3600.0).1);
    }
}
