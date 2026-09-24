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
use crate::diagnose::format_datetime;
use crate::i18n;
use crate::probe::eventlog::SysEvent;
use crate::store::Event;

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
    });
}

/// Width of the outage list beside the detail. Wide enough for a date, a
/// duration and a line of detail; the rest goes to the detail, which is
/// where the reading happens.
const LIST_W: f32 = 340.0;
const ROW_H: f32 = 54.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let events = app.store.recent_events(300);
    header(app, ui, &events);
    if clear_confirm(app, ui, &events) {
        return;
    }
    if events.is_empty() {
        card(ui, i18n::hist_list_heading(), |ui| {
            ui.label(egui::RichText::new(i18n::hist_nothing_logged()).size(T_BODY).color(FG_DIM));
        });
        return;
    }

    // A selection made before the list refreshed may name a row that is no
    // longer here; dropping it is better than showing the wrong outage.
    if let Some(id) = app.selected_outage {
        if !events.iter().any(|e| e.id == id) {
            app.selected_outage = None;
        }
    }

    if super::is_narrow(ui) {
        // One at a time: the list, or one outage with a way back to it. Both
        // squeezed into a narrow window left the detail a few lines tall.
        match app.selected_outage.and_then(|id| events.iter().find(|e| e.id == id)) {
            Some(event) => {
                if button(ui, i18n::btn_back_to_list(), Emphasis::Ghost).clicked() {
                    app.selected_outage = None;
                }
                ui.add_space(S_SM);
                egui::ScrollArea::vertical()
                    .id_salt("history_detail")
                    .auto_shrink([false, false])
                    .show(ui, |ui| detail(app, ui, event, &events));
            }
            None => list(app, ui, &events),
        }
        return;
    }

    // Side by side there is room for both, so the newest outage is opened
    // rather than an empty pane asking to be clicked.
    if app.selected_outage.is_none() {
        app.selected_outage = events.first().map(|e| e.id);
    }

    egui::SidePanel::left("history_list")
        .resizable(false)
        .exact_width(LIST_W)
        .show_separator_line(false)
        .frame(egui::Frame::none().inner_margin(egui::Margin { right: S_LG, ..Default::default() }))
        .show_inside(ui, |ui| list(app, ui, &events));

    egui::CentralPanel::default().frame(egui::Frame::none()).show_inside(ui, |ui| {
        let Some(event) = app.selected_outage.and_then(|id| events.iter().find(|e| e.id == id))
        else {
            ui.label(egui::RichText::new(i18n::hist_select_hint()).size(T_BODY).color(FG_DIM));
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("history_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| detail(app, ui, event, &events));
    });
}

/// Whether the clear-history question is on screen.
fn confirm_id() -> egui::Id {
    egui::Id::new("history_confirm_clear")
}

/// The day in one sentence, what the entries hold, and the report.
fn header(app: &mut App, ui: &mut egui::Ui, events: &[Event]) {
    let can_clear = events.iter().any(|e| e.ts_end.is_some());
    let day = app.store.events_since(24.0 * 3600.0);
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

fn list(app: &mut App, ui: &mut egui::Ui, events: &[Event]) {
    ui.label(
        egui::RichText::new(format!("{} · {}", i18n::hist_list_heading(), events.len()))
            .size(T_META)
            .color(FG_DIM),
    );
    ui.add_space(S_XS);
    egui::ScrollArea::vertical().id_salt("history_table").auto_shrink([false, false]).show_rows(
        ui,
        ROW_H + 2.0,
        events.len(),
        |ui, range| {
            for e in &events[range] {
                let selected = app.selected_outage == Some(e.id);
                if list_row(ui, e, selected).clicked() {
                    app.selected_outage = Some(e.id);
                }
                ui.add_space(2.0);
            }
        },
    );
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
    let cached = app.outage_detail.take();
    let detail = cached.as_ref().expect("ensure_detail just stored one");

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

    // Changes applied in the hour before the outage: the correlation the app
    // has always had the data for and never drawn.
    let tweaks = app.store.tweaks_between(event.ts_start - 3600.0, event.ts_start);
    request_log(app, event);
    let log = match &app.syslog {
        Some((id, events)) if *id == event.id => events.clone(),
        _ => Vec::new(),
    };
    let router = app.store.router_between(
        event.ts_start - cause::ROUTER_MARGIN_S,
        event.ts_end.unwrap_or(event.ts_start) + cause::ROUTER_MARGIN_S,
    );
    let causes = cause::analyse(event, detail.evidence.as_ref(), events, &tweaks, &log, &router);

    card(ui, i18n::hist_cause_heading(), |ui| {
        for c in &causes {
            cause_row(app, ui, c);
        }
    });

    if !tweaks.is_empty() {
        card(ui, i18n::hist_tweaks_heading(), |ui| {
            for t in &tweaks {
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
        lead_up(app, ui, event, detail.evidence.as_ref());
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

    card(ui, i18n::hist_log_heading(), |ui| system_log(app, ui, event, &log));

    app.outage_detail = cached;
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
fn lead_up(app: &App, ui: &mut egui::Ui, event: &Event, evidence: Option<&Evidence>) {
    // The same parse the cause rules read, rather than a second one of the
    // same 33 KB.
    let lead: &[crate::monitor::LeadSample] = evidence.map(|e| e.lead.as_slice()).unwrap_or(&[]);

    let t0 = event.ts_start;
    let from = lead.first().map(|s| s.ts).unwrap_or(t0 - 180.0);
    let now = crate::store::now();
    let end = event.ts_end.unwrap_or(now);
    // A minute of recovery, but never past the present: for an outage that
    // just ended, the rest of that minute is an empty stretch of axis.
    let to = (end + 60.0).min(now);

    // Router and internet latency come from the stored samples rather than
    // the lead-up, because the lead-up stops where the outage begins and the
    // shape of the outage itself is half the evidence.
    let mut router = Vec::new();
    let mut router_lost = Vec::new();
    // The internet is the best of the public anchors in each second, the way
    // the monitor reads it: one slow anchor is not the internet being slow.
    let mut net: std::collections::BTreeMap<i64, (Option<f64>, bool)> = Default::default();
    for (ts, target, rtt_ms, ok) in app.store.samples_between(from, to) {
        let x = ts - t0;
        match target.as_str() {
            "gateway" => match (ok, rtt_ms) {
                (true, Some(ms)) => router.push([x, ms]),
                _ => router_lost.push(x),
            },
            "cloudflare" | "google" => {
                let slot = net.entry(x.round() as i64).or_insert((None, false));
                if let (true, Some(ms)) = (ok, rtt_ms) {
                    slot.0 = Some(slot.0.map_or(ms, |best: f64| best.min(ms)));
                }
                slot.1 |= ok;
            }
            _ => {}
        }
    }
    let internet: Vec<[f64; 2]> =
        net.iter().filter_map(|(x, (ms, _))| ms.map(|ms| [*x as f64, ms])).collect();
    let internet_lost: Vec<f64> =
        net.iter().filter(|(_, (_, any_ok))| !any_ok).map(|(x, _)| *x as f64).collect();

    let rssi: Vec<[f64; 2]> =
        lead.iter().filter_map(|s| s.rssi_dbm.map(|r| [s.ts - t0, r as f64])).collect();

    if router.is_empty() && router_lost.is_empty() && internet.is_empty() && rssi.is_empty() {
        ui.label(egui::RichText::new(i18n::hist_no_leadup()).size(T_META).color(FG_DIM));
        return;
    }

    // Lost pings sit in a band just above the highest real reply, so they
    // are in view without squashing every real reply towards zero.
    let peak = router.iter().chain(&internet).map(|p| p[1]).fold(0.0_f64, f64::max);
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
                Line::new(PlotPoints::from(router))
                    .name(i18n::hist_leadup_rtt())
                    .color(router_colour),
            );
            plot.line(
                Line::new(PlotPoints::from(internet))
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
                    Line::new(PlotPoints::from(rssi))
                        .name(i18n::hist_leadup_rssi())
                        .color(signal_colour),
                );
            });
    }

    if lead.is_empty() {
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
