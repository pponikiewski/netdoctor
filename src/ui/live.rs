//! Live tab: the latency plot and the headline numbers.

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints, VLine};

use super::{S_MD, S_SM, S_XS, T_BODY, T_META, latency_colour, stat_card, App, Job, ACCENT, FG_DIM, GREEN, RED, SERIES_COLOURS, YELLOW};
use crate::i18n;
use crate::probe::icmp;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    plot(app, ui);
    ui.add_space(S_MD);
    cards(app, ui);
    ui.add_space(S_MD);
    controls(app, ui);

    if !app.last.note.is_empty() || app.last.roamed {
        ui.add_space(S_SM);
        let mut note = app.last.note.clone();
        if app.last.roamed {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str(i18n::live_roamed());
        }
        egui::Frame::none()
            .fill(super::BG2)
            .rounding(6.0)
            .inner_margin(egui::Margin::same(S_MD))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(note).size(T_BODY).color(FG_DIM));
            });
    }
}

/// The legend's colour key, drawn to match the line it stands for.
///
/// This was a `▬` character tinted to the series colour, so its length and
/// weight came from the font rather than from the plot. A legend key should
/// look like a short piece of the line it names.
fn swatch(ui: &mut egui::Ui, colour: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 3.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 1.5, colour);
}

fn plot(app: &mut App, ui: &mut egui::Ui) {
    let targets = app.settings.targets();
    let mut series: Vec<(String, egui::Color32, Vec<(f64, Option<f64>)>)> = Vec::new();
    for (i, t) in targets.iter().enumerate() {
        let data = app.monitor.series(&t.key);
        if data.is_empty() {
            continue;
        }
        let label = match t.key.as_str() {
            "gateway" => i18n::word_router().to_string(),
            _ => t.label.clone(),
        };
        series.push((label, SERIES_COLOURS[i % SERIES_COLOURS.len()], data));
    }

    let newest = series
        .iter()
        .flat_map(|(_, _, d)| d.last().map(|(ts, _)| *ts))
        .fold(f64::NEG_INFINITY, f64::max);

    Plot::new("latency")
        .height(280.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .show_axes([false, true])
        .y_axis_label("ms")
        .x_axis_formatter(|mark, _| {
            // x is seconds relative to now, so label it as age.
            let back = -mark.value;
            if back < 1.0 {
                i18n::live_x_now().to_string()
            } else if back < 90.0 {
                format!("-{back:.0}s")
            } else {
                format!("-{:.0}m", back / 60.0)
            }
        })
        .show(ui, |plot_ui| {
            for (label, colour, data) in &series {
                // Split at gaps so a lost packet breaks the line instead of
                // drawing a straight segment across the outage.
                let mut run: Vec<[f64; 2]> = Vec::new();
                for (ts, rtt) in data {
                    let x = ts - newest;
                    match rtt {
                        Some(v) => run.push([x, *v]),
                        None => {
                            if run.len() > 1 {
                                plot_ui.line(
                                    Line::new(PlotPoints::from(std::mem::take(&mut run)))
                                        .color(*colour)
                                        .name(label),
                                );
                            } else {
                                run.clear();
                            }
                            plot_ui.vline(VLine::new(x).color(RED.linear_multiply(0.6)));
                        }
                    }
                }
                if run.len() > 1 {
                    plot_ui.line(Line::new(PlotPoints::from(run)).color(*colour).name(label));
                }
            }
        });

    ui.horizontal_wrapped(|ui| {
        for (i, (label, colour, _)) in series.iter().enumerate() {
            swatch(ui, *colour);
            ui.add_space(S_XS);
            let resp = ui.label(egui::RichText::new(label).size(T_BODY).color(FG_DIM));

            // Show the live per-target state on hover: which probe is failing
            // and why is exactly what you want when something is wrong.
            if let Some(key) = targets.get(i).map(|t| &t.key) {
                if let Some(sample) = app.last.results.get(key) {
                    let tip = match (&sample.rtt_ms, &sample.error) {
                        (Some(rtt), _) => format!("{label}: {rtt:.2} ms"),
                        (None, Some(err)) => format!("{label}: {err}"),
                        (None, None) => format!("{label}: {}", i18n::live_no_data()),
                    };
                    resp.on_hover_text(tip);
                }
            }
            ui.add_space(S_SM);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(i18n::live_red_line()).size(T_META).color(FG_DIM),
            );
        });
    });
}

fn cards(app: &mut App, ui: &mut egui::Ui) {
    let s = &app.settings;
    let cf = app.store.stats("cloudflare", 300.0);
    let gw = app.store.stats("gateway", 300.0);

    ui.horizontal(|ui| {
        match cf.avg {
            Some(avg) => stat_card(
                ui,
                i18n::live_card_latency(),
                &format!("{avg:.0} ms"),
                &i18n::live_minmax(cf.min.unwrap_or(0.0), cf.max.unwrap_or(0.0)),
                latency_colour(avg, s),
            ),
            None => stat_card(ui, i18n::live_card_latency(), "—", i18n::live_card_latency_sub_none(), FG_DIM),
        }

        match cf.jitter {
            Some(j) => {
                let colour = if j < s.jitter_good_ms {
                    GREEN
                } else if j < s.jitter_ok_ms {
                    YELLOW
                } else {
                    RED
                };
                stat_card(ui, i18n::live_card_jitter(), &format!("{j:.1} ms"), i18n::live_card_jitter_sub(), colour)
            }
            None => stat_card(ui, i18n::live_card_jitter(), "—", "", FG_DIM),
        }

        let loss_colour = if cf.loss_pct < s.loss_good_pct {
            GREEN
        } else if cf.loss_pct < s.loss_ok_pct {
            YELLOW
        } else {
            RED
        };
        stat_card(
            ui,
            i18n::live_card_loss(),
            &format!("{:.1}%", cf.loss_pct),
            i18n::live_card_loss_sub(),
            loss_colour,
        );

        match gw.avg {
            Some(avg) => stat_card(
                ui,
                i18n::live_card_router(),
                &format!("{avg:.1} ms"),
                &i18n::live_router_loss(gw.loss_pct),
                if avg < 10.0 { GREEN } else { YELLOW },
            ),
            None => stat_card(ui, i18n::live_card_router(), "—", i18n::live_card_router_none(), RED),
        }

        if !app.last.dns_error.is_empty() {
            stat_card(ui, i18n::live_card_dns(), i18n::live_card_dns_err(), &app.last.dns_error, RED);
        } else {
            match app.last.dns_ms {
                Some(ms) => stat_card(
                    ui,
                    i18n::live_card_dns(),
                    &format!("{ms:.0} ms"),
                    i18n::live_card_dns_sub(),
                    if ms < 60.0 { GREEN } else if ms < 150.0 { YELLOW } else { RED },
                ),
                None => stat_card(ui, i18n::live_card_dns(), "—", "", FG_DIM),
            }
        }

        let events = app.store.events_since(24.0 * 3600.0);
        if events.is_empty() {
            stat_card(
                ui,
                i18n::live_card_uninterrupted(),
                i18n::live_card_uninterrupted_val(),
                i18n::live_card_uninterrupted_sub(),
                GREEN,
            );
        } else {
            let last = events.iter().map(|e| e.ts_start).fold(f64::NEG_INFINITY, f64::max);
            let mins = (crate::store::now() - last) / 60.0;
            stat_card(
                ui,
                i18n::live_card_since_outage(),
                &format!("{mins:.0} min"),
                &i18n::live_outages_24h(events.len()),
                if events.len() < 3 { YELLOW } else { RED },
            );
        }
    });
}

fn controls(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let paused = app.monitor.is_paused();
        let btn = if paused { i18n::live_btn_resume() } else { i18n::live_btn_pause() };
        if ui.button(btn).clicked() {
            app.monitor.set_paused(!paused);
        }

        if ui
            .add_enabled(!app.tracing, egui::Button::new(i18n::live_btn_trace()))
            .clicked()
        {
            app.tracing = true;
            app.trace = vec![i18n::live_tracing().into()];
            let tx = app.tx.clone();
            std::thread::spawn(move || {
                let hops = icmp::traceroute(std::net::Ipv4Addr::new(1, 1, 1, 1), 20, 1000);
                let mut lines: Vec<String> = hops
                    .iter()
                    .map(|h| match h.addr {
                        Some(a) => format!(
                            "{:>2}  {:<16} {}",
                            h.hop,
                            a,
                            h.rtt_ms.map(|v| format!("{v:.1} ms")).unwrap_or_else(|| "*".into())
                        ),
                        None => format!("{:>2}  {:<16} *", h.hop, "*"),
                    })
                    .collect();
                lines.push(String::new());
                lines.push(i18n::live_trace_note_1().into());
                lines.push(i18n::live_trace_note_2().into());
                let _ = tx.send(Job::Traceroute(lines));
            });
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(i18n::live_btn_report()).clicked() {
                match super::report::save(app) {
                    Ok(path) => {
                        let now = ui.input(|i| i.time);
                        app.toast(i18n::live_report_saved(&path), GREEN, now);
                    }
                    Err(e) => {
                        let now = ui.input(|i| i.time);
                        app.toast(i18n::set_save_failed(&e.to_string()), RED, now);
                    }
                }
            }
        });
    });

    if !app.trace.is_empty() {
        ui.add_space(S_SM);
        egui::Frame::none()
            .fill(super::BG2)
            .rounding(6.0)
            .inner_margin(egui::Margin::same(S_MD))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    for line in &app.trace {
                        ui.label(egui::RichText::new(line).monospace().size(T_META).color(ACCENT));
                    }
                });
            });
    }
}
