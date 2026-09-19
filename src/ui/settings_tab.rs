//! Settings: probing cadence, thresholds, extra targets, autostart.

use eframe::egui;

use super::{App, FG, FG_DIM, GREEN, RED, YELLOW};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.columns(2, |cols| {
            let left = &mut cols[0];
            section(left, "Probing", |ui| {
                num_u64(ui, "Interval between sweeps (ms)", &mut app.draft.probe_interval_ms);
                hint(ui, "Lower is more detailed but adds traffic. The floor is 300 ms.");
                num_u32(ui, "Ping timeout (ms)", &mut app.draft.ping_timeout_ms);
                num_u32(ui, "Failed sweeps before an alarm", &mut app.draft.outage_after_fails);
                hint(ui, "Guards against logging a single dropped packet as an outage.");
                num_usize(ui, "Chart width (samples)", &mut app.draft.history_points);
                num_i64(ui, "Days of history to keep", &mut app.draft.keep_days);
            });

            section(left, "Extra ping targets", |ui| {
                hint(
                    ui,
                    "One host per line — for example the game server you play on. Names are \
                     resolved when settings are saved.",
                );
                ui.add(
                    egui::TextEdit::multiline(&mut app.draft_targets)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            });

            let right = &mut cols[1];
            section(right, "Thresholds", |ui| {
                num_f64(ui, "Latency still good up to (ms)", &mut app.draft.ping_ok_ms);
                num_f64(ui, "Latency bad above (ms)", &mut app.draft.ping_bad_ms);
                num_f64(ui, "Jitter good up to (ms)", &mut app.draft.jitter_good_ms);
                num_f64(ui, "Jitter acceptable up to (ms)", &mut app.draft.jitter_ok_ms);
                num_f64(ui, "Acceptable packet loss (%)", &mut app.draft.loss_ok_pct);
            });

            section(right, "Behaviour", |ui| {
                ui.checkbox(&mut app.draft.notify_on_outage, "Announce outages in the app");
                ui.checkbox(&mut app.draft.start_minimised, "Start minimised");

                let mut autostart = app.autostart_on;
                if ui.checkbox(&mut autostart, "Start with Windows").changed() {
                    let now = ui.input(|i| i.time);
                    match crate::autostart::set(autostart) {
                        Ok(msg) => {
                            app.autostart_on = autostart;
                            app.toast(msg, GREEN, now);
                        }
                        Err(e) => app.toast(e.to_string(), RED, now),
                    }
                }
                hint(
                    ui,
                    "Autostart only matters together with background monitoring: without it the \
                     app cannot record an outage that happens while it is closed.",
                );
                if crate::autostart::is_stale() {
                    ui.label(
                        egui::RichText::new(
                            "The autostart entry points at an executable that no longer exists. \
                             Toggle it off and on again to repoint it here.",
                        )
                        .size(11.0)
                        .color(YELLOW),
                    );
                }
            });
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Save settings").clicked() {
                save(app, ui);
            }
            if ui.button("Restore defaults").clicked() {
                app.draft = crate::settings::Settings::default();
                app.draft_targets.clear();
                save(app, ui);
            }
        });
    });
}

fn save(app: &mut App, ui: &mut egui::Ui) {
    let now = ui.input(|i| i.time);

    if app.draft.probe_interval_ms < 300 {
        app.toast("An interval below 300 ms loads the network more than it measures.", RED, now);
        return;
    }
    if app.draft.ping_ok_ms >= app.draft.ping_bad_ms {
        app.toast("\"Good\" latency must be lower than \"bad\" latency.", RED, now);
        return;
    }
    app.draft.history_points = app.draft.history_points.max(30);

    app.draft.extra_targets = app
        .draft_targets
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    // Name anything that will silently never be probed, rather than letting
    // the user believe a typo is being monitored.
    let unresolved: Vec<String> = app
        .draft
        .extra_targets
        .iter()
        .filter(|t| crate::settings::resolve_target(t).is_none())
        .cloned()
        .collect();

    app.settings = app.draft.clone();
    match app.settings.save() {
        Ok(()) => {
            app.monitor.update_settings(app.settings.clone());
            if unresolved.is_empty() {
                app.toast("Settings saved.", GREEN, now);
            } else {
                app.toast(
                    format!("Saved, but these could not be resolved: {}", unresolved.join(", ")),
                    YELLOW,
                    now,
                );
            }
        }
        Err(e) => app.toast(format!("Could not save: {e}"), RED, now),
    }
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(super::BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(title).size(14.0).strong().color(FG));
            ui.add_space(8.0);
            body(ui);
        });
    ui.add_space(10.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(10.0).color(FG_DIM));
    ui.add_space(4.0);
}

fn row(ui: &mut egui::Ui, label: &str, widget: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(12.0).color(FG));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), widget);
    });
}

fn num_u64(ui: &mut egui::Ui, label: &str, value: &mut u64) {
    row(ui, label, |ui| {
        ui.add(egui::DragValue::new(value).speed(50));
    });
}

fn num_u32(ui: &mut egui::Ui, label: &str, value: &mut u32) {
    row(ui, label, |ui| {
        ui.add(egui::DragValue::new(value).speed(1));
    });
}

fn num_usize(ui: &mut egui::Ui, label: &str, value: &mut usize) {
    row(ui, label, |ui| {
        ui.add(egui::DragValue::new(value).speed(10));
    });
}

fn num_i64(ui: &mut egui::Ui, label: &str, value: &mut i64) {
    row(ui, label, |ui| {
        ui.add(egui::DragValue::new(value).speed(1));
    });
}

fn num_f64(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    row(ui, label, |ui| {
        ui.add(egui::DragValue::new(value).speed(1.0));
    });
}
