//! Outage history: when, how long, whose fault — and, once an entry is
//! selected, why.
//!
//! The table alone answers "did it drop" and nothing else. Every row carries
//! the connection state and the sweeps that preceded it, so selecting one
//! opens the reasoning: the cause, the evidence it was read from, the lead-up
//! plotted, and a button straight to the fix where one exists.

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};

use super::{figure, S_MD, S_SM, S_XS, T_BODY, T_HEAD, T_META, T_TITLE, App, Tab, FG, FG_DIM, GREEN, RED, YELLOW};
use crate::cause::{self, Cause, Confidence};
use crate::diagnose::format_datetime;
use crate::i18n;
use crate::monitor::LeadSample;
use crate::store::Event;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let day = app.store.events_since(24.0 * 3600.0);

    if day.is_empty() {
        ui.label(
            egui::RichText::new(i18n::hist_none_24h())
                .size(T_TITLE)
                .strong()
                .color(GREEN),
        );
    } else {
        // This headline is rebuilt on every frame, so the choice has to be a
        // function of the data alone — see `dominant_scope`.
        let worst = crate::monitor::dominant_scope(day.iter().map(|e| e.scope.as_str()))
            .unwrap_or("");
        let where_text = match worst {
            "lan" => i18n::hist_where_lan(),
            "adapter" => i18n::hist_where_adapter(),
            "isp" => i18n::hist_where_isp(),
            "dns" => i18n::hist_where_dns(),
            _ => i18n::hist_where_other(),
        };
        ui.label(
            egui::RichText::new(i18n::hist_summary(day.len(), where_text))
            .size(T_TITLE)
            .strong()
            .color(RED),
        );
    }

    ui.add_space(S_XS);
    ui.label(
        egui::RichText::new(i18n::hist_blurb())
        .size(T_BODY)
        .color(FG_DIM),
    );
    ui.add_space(S_MD);

    let events = app.store.recent_events(300);
    if events.is_empty() {
        ui.label(egui::RichText::new(i18n::hist_nothing_logged()).color(FG_DIM));
        return;
    }

    // A selection made before the list refreshed may name a row that is no
    // longer here; dropping it is better than showing the wrong outage.
    if let Some(id) = app.selected_outage {
        if !events.iter().any(|e| e.id == id) {
            app.selected_outage = None;
        }
    }

    let available = ui.available_height();
    egui::ScrollArea::vertical()
        .id_salt("history_table")
        .max_height(if app.selected_outage.is_some() { available * 0.4 } else { available })
        .show(ui, |ui| {
            table(app, ui, &events);
        });

    let Some(id) = app.selected_outage else {
        ui.add_space(S_SM);
        ui.label(egui::RichText::new(i18n::hist_select_hint()).size(T_META).color(FG_DIM));
        return;
    };
    let Some(event) = events.iter().find(|e| e.id == id) else {
        return;
    };

    ui.add_space(S_SM);
    ui.separator();
    ui.add_space(S_SM);
    egui::ScrollArea::vertical()
        .id_salt("history_detail")
        .show(ui, |ui| detail(app, ui, event, &events));
}

fn table(app: &mut App, ui: &mut egui::Ui, events: &[Event]) {
    egui::Grid::new("history")
        .num_columns(4)
        .striped(true)
        .spacing([16.0, 5.0])
        .show(ui, |ui| {
            for h in [
                i18n::hist_col_started(),
                i18n::hist_col_duration(),
                i18n::hist_col_kind(),
                i18n::hist_col_detail(),
            ] {
                ui.label(egui::RichText::new(h).size(T_META).color(FG_DIM));
            }
            ui.end_row();

            for e in events {
                let colour = match e.scope.as_str() {
                    "lan" | "adapter" | "isp" => RED,
                    _ => YELLOW,
                };
                let selected = app.selected_outage == Some(e.id);
                let started = ui.selectable_label(
                    selected,
                    egui::RichText::new(format_datetime(e.ts_start))
                        .size(T_META)
                        .monospace()
                        .color(FG),
                );
                if started.clicked() {
                    app.selected_outage = if selected { None } else { Some(e.id) };
                }
                // A duration column is read by comparing rows, which only
                // works if the digits sit in the same place on each one.
                ui.label(figure(
                    match e.duration_s() {
                        Some(d) => format!("{d:.0} s"),
                        None => i18n::hist_ongoing().into(),
                    },
                    T_META,
                    colour,
                ));
                ui.label(egui::RichText::new(i18n::event_kind(&e.kind)).size(T_META).color(colour));
                ui.label(egui::RichText::new(&e.detail).size(T_META).color(FG_DIM));
                ui.end_row();
            }
        });
}

fn detail(app: &mut App, ui: &mut egui::Ui, event: &Event, events: &[Event]) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(i18n::hist_cause_for(&format_datetime(event.ts_start)))
                .size(T_TITLE)
                .strong()
                .color(FG),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(i18n::hist_btn_close()).clicked() {
                app.selected_outage = None;
            }
        });
    });
    ui.add_space(S_SM);

    // Changes applied in the hour before the outage: the correlation the app
    // has always had the data for and never drawn.
    let tweaks = app.store.tweaks_between(event.ts_start - 3600.0, event.ts_start);
    let causes = cause::analyse(event, events, &tweaks);

    ui.label(egui::RichText::new(i18n::hist_cause_heading()).size(T_BODY).strong().color(FG_DIM));
    ui.add_space(S_XS);
    for c in &causes {
        cause_row(app, ui, c);
    }

    if !tweaks.is_empty() {
        ui.add_space(S_SM);
        ui.label(
            egui::RichText::new(i18n::hist_tweaks_heading()).size(T_BODY).strong().color(FG_DIM),
        );
        for t in &tweaks {
            ui.label(
                egui::RichText::new(format!(
                    "{}  {}  {}",
                    format_datetime(t.ts),
                    i18n::tweak_name(&t.tweak_id),
                    t.action
                ))
                .size(T_META)
                .monospace()
                .color(FG_DIM),
            );
        }
    }

    ui.add_space(S_MD);
    lead_up(app, ui, event);

    ui.add_space(S_MD);
    ui.columns(2, |cols| {
        state_block(&mut cols[0], i18n::hist_state_heading(), event.context_json());
        let recovery = event.context_end_json();
        if recovery.is_some() {
            state_block(&mut cols[1], i18n::hist_recovery_heading(), recovery);
        } else {
            cols[1].label(
                egui::RichText::new(i18n::hist_recovery_heading()).size(T_BODY).strong().color(FG_DIM),
            );
            cols[1].label(egui::RichText::new(i18n::hist_no_recovery()).size(T_META).color(YELLOW));
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
        .fill(egui::Color32::from_black_alpha(40))
        .inner_margin(egui::Margin::symmetric(S_MD, S_SM))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(c.title()).size(T_HEAD).strong().color(colour));
                ui.label(
                    egui::RichText::new(format!("({})", c.confidence.label()))
                        .size(T_META)
                        .color(FG_DIM),
                );
                if let Some(tweak) = c.fix_tweak {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(i18n::hist_btn_fix()).clicked() {
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

fn parse_lead(event: &Event) -> Vec<LeadSample> {
    event
        .context_json()
        .and_then(|v| v.get("lead_up").cloned())
        .and_then(|v| serde_json::from_value::<Vec<LeadSample>>(v).ok())
        .unwrap_or_default()
}

/// Signal and router latency across the whole episode: the lead-up from the
/// event's own context, the outage and the recovery from the sample table.
/// Time is drawn relative to the start of the outage, so zero is the moment it
/// broke and everything left of it is what led there.
///
/// Two lines are enough to separate the two stories that look identical in the
/// table: a signal sliding away before the router stops answering, and a
/// router that stops answering while the signal never moves.
fn lead_up(app: &App, ui: &mut egui::Ui, event: &Event) {
    let lead = parse_lead(event);
    ui.label(egui::RichText::new(i18n::hist_leadup_heading()).size(T_BODY).strong().color(FG_DIM));

    let t0 = event.ts_start;
    let from = lead.first().map(|s| s.ts).unwrap_or(t0 - 180.0);
    let to = event.ts_end.unwrap_or_else(crate::store::now) + 60.0;

    // Router latency comes from the stored samples rather than the lead-up,
    // because the lead-up stops where the outage begins and the shape of the
    // outage itself is half the evidence.
    let rtt: PlotPoints = app
        .store
        .samples_between(from, to)
        .into_iter()
        .filter(|(_, target, _, _)| target == "gateway")
        .map(|(ts, _, rtt_ms, ok)| {
            // A lost ping plotted as zero would read as an instant reply, so
            // it is drawn as a spike to the top of the range instead.
            [ts - t0, if ok { rtt_ms.unwrap_or(0.0) } else { LOST_PING_MS }]
        })
        .collect();

    let rssi: PlotPoints = lead
        .iter()
        .filter_map(|s| s.rssi_dbm.map(|r| [s.ts - t0, r as f64]))
        .collect();

    if lead.is_empty() && rtt.points().is_empty() {
        ui.label(egui::RichText::new(i18n::hist_no_leadup()).size(T_META).color(FG_DIM));
        return;
    }

    ui.add_space(S_XS);
    Plot::new("lead_up")
        .height(140.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .x_axis_label(i18n::hist_leadup_axis())
        .show(ui, |plot| {
            plot.line(Line::new(rssi).name(i18n::hist_leadup_rssi()).color(YELLOW));
            plot.line(Line::new(rtt).name(i18n::hist_leadup_rtt()).color(GREEN));
        });

    if lead.is_empty() {
        ui.label(egui::RichText::new(i18n::hist_no_leadup()).size(T_META).color(FG_DIM));
    }
}

/// Where a lost ping is drawn. High enough to stand out against real replies,
/// low enough not to flatten the rest of the line.
const LOST_PING_MS: f64 = 250.0;

/// The stored state, rendered as the fields that mean something to a person.
/// The raw JSON is never shown: it is a storage format, not a report.
fn state_block(ui: &mut egui::Ui, heading: &str, state: Option<serde_json::Value>) {
    ui.label(egui::RichText::new(heading).size(T_BODY).strong().color(FG_DIM));
    let Some(v) = state else {
        ui.label(egui::RichText::new(i18n::hist_no_state()).size(T_META).color(FG_DIM));
        return;
    };

    let rows: Vec<(&str, String)> = [
        ("SSID", v["ssid"].as_str().unwrap_or("").to_string()),
        ("BSSID", v["bssid"].as_str().unwrap_or("").to_string()),
        ("RSSI", fmt_num(&v["rssi_dbm"], " dBm")),
        ("Signal", fmt_num(&v["signal_pct"], "%")),
        ("Channel", fmt_num(&v["channel"], "")),
        ("Band", v["band"].as_str().unwrap_or("").to_string()),
        ("PHY", v["phy"].as_str().unwrap_or("").to_string()),
        ("RX", fmt_num(&v["rx_mbps"], " Mbps")),
        ("Gateway", v["gateway"].as_str().unwrap_or("").to_string()),
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
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}
