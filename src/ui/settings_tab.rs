//! Settings: probing cadence, thresholds, extra targets, autostart.

use eframe::egui;

use super::{App, FG, FG_DIM, GREEN, RED, S_LG, S_MD, S_SM, T_BODY, T_HEAD, T_META, YELLOW};
use crate::i18n;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Silently resetting to defaults looks like the app forgot the
        // settings; saying the file was damaged points at the real cause and
        // at the copy that was kept.
        if let Some(detail) = crate::settings::load_issue() {
            ui.label(egui::RichText::new(i18n::set_load_failed(&detail)).size(T_META).color(RED));
            ui.add_space(S_SM);
        }

        // Side by side when there is room, stacked when there is not. A
        // labelled number field squeezed into a third of a narrow window is a
        // field whose label and value stop fitting on one line.
        if super::is_narrow(ui) {
            probing_and_targets(app, ui);
            language_thresholds_behaviour(app, ui);
        } else {
            ui.columns(2, |cols| {
                probing_and_targets(app, &mut cols[0]);
                language_thresholds_behaviour(app, &mut cols[1]);
            });
        }

        ui.add_space(S_MD);
        ui.horizontal(|ui| {
            if ui.button(i18n::set_btn_save()).clicked() {
                save(app, ui);
            }
            if ui.button(i18n::set_btn_defaults()).clicked() {
                app.draft = crate::settings::Settings::default();
                app.draft_targets.clear();
                save(app, ui);
            }
        });
    });
}

/// How often the app probes, and what it probes.
fn probing_and_targets(app: &mut App, ui: &mut egui::Ui) {
    section(ui, i18n::set_sec_probing(), |ui| {
        num_u64(ui, i18n::set_interval(), &mut app.draft.probe_interval_ms);
        hint(ui, i18n::set_interval_hint());
        num_u32(ui, i18n::set_ping_timeout(), &mut app.draft.ping_timeout_ms);
        num_u32(ui, i18n::set_fails_before_alarm(), &mut app.draft.outage_after_fails);
        hint(ui, i18n::set_fails_hint());
        num_i64(ui, i18n::set_keep_days(), &mut app.draft.keep_days);
    });

    section(ui, i18n::set_sec_targets(), |ui| {
        hint(ui, i18n::set_targets_hint());
        ui.add(
            egui::TextEdit::multiline(&mut app.draft_targets)
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );
    });
}

/// The choices that are about the app rather than about the measurement.
fn language_thresholds_behaviour(app: &mut App, ui: &mut egui::Ui) {
    section(ui, i18n::set_language(), |ui| {
        // Applied on click rather than on save: a language picker that needs
        // a second confirmation is hard to undo once the labels are in a
        // language you cannot read.
        let mut chosen = app.draft.effective_lang();
        ui.horizontal(|ui| {
            for lang in i18n::Lang::ALL {
                if ui.selectable_label(chosen == lang, lang.native_name()).clicked() {
                    chosen = lang;
                }
            }
        });
        if chosen != app.draft.effective_lang() {
            app.draft.lang = Some(chosen);
            app.settings.lang = Some(chosen);
            i18n::set(chosen);
            let _ = app.settings.save();
        }
        hint(ui, i18n::set_language_hint());
    });

    section(ui, i18n::set_sec_thresholds(), |ui| {
        num_f64(ui, i18n::set_lat_good(), &mut app.draft.ping_ok_ms);
        num_f64(ui, i18n::set_lat_bad(), &mut app.draft.ping_bad_ms);
        num_f64(ui, i18n::set_jitter_good(), &mut app.draft.jitter_good_ms);
        num_f64(ui, i18n::set_jitter_ok(), &mut app.draft.jitter_ok_ms);
        num_f64(ui, i18n::set_loss_ok(), &mut app.draft.loss_ok_pct);
    });

    section(ui, i18n::set_sec_behaviour(), |ui| {
        ui.checkbox(&mut app.draft.notify_on_outage, i18n::set_notify());
        ui.checkbox(&mut app.draft.start_minimised, i18n::set_start_min());

        let mut autostart = app.autostart_on;
        if ui.checkbox(&mut autostart, i18n::set_autostart()).changed() {
            let now = ui.input(|i| i.time);
            match crate::autostart::set(autostart) {
                Ok(msg) => {
                    app.autostart_on = autostart;
                    app.toast(msg, GREEN, now);
                }
                Err(e) => app.toast(e.to_string(), RED, now),
            }
        }
        hint(ui, i18n::set_autostart_hint());
        if crate::autostart::is_stale() {
            ui.label(egui::RichText::new(i18n::set_autostart_stale()).size(T_META).color(YELLOW));
        }
    });

    section(ui, i18n::upd_section(), |ui| super::update_ui::section(app, ui));
}

fn save(app: &mut App, ui: &mut egui::Ui) {
    let now = ui.input(|i| i.time);

    if app.draft.probe_interval_ms < 300 {
        app.toast(i18n::set_err_interval(), RED, now);
        return;
    }
    if app.draft.ping_ok_ms >= app.draft.ping_bad_ms {
        app.toast(i18n::set_err_thresholds(), RED, now);
        return;
    }

    app.draft.extra_targets =
        app.draft_targets.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();

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
                app.toast(i18n::set_saved(), GREEN, now);
            } else {
                app.toast(i18n::set_saved_unresolved(&unresolved.join(", ")), YELLOW, now);
            }
        }
        Err(e) => app.toast(i18n::set_save_failed(&e.to_string()), RED, now),
    }
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none().fill(super::BG2).rounding(6.0).inner_margin(egui::Margin::same(S_LG)).show(
        ui,
        |ui| {
            ui.label(egui::RichText::new(title).size(T_HEAD).strong().color(FG));
            ui.add_space(S_MD);
            body(ui);
        },
    );
    ui.add_space(S_MD);
}

/// The line under a control that says what it is for.
///
/// It was set one step below the smallest label on the screen, which made
/// the explanation harder to read than the setting it explains. It is now
/// the same step as the other secondary text, and the gap below it is what
/// ties it to the control rather than the size.
fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(T_META).color(FG_DIM));
    ui.add_space(S_SM);
}

fn row(ui: &mut egui::Ui, label: &str, widget: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(T_BODY).color(FG));
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
