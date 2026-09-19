//! Optimise tab: inspect, apply and revert each tweak.

use eframe::egui;

use super::{App, FG, FG_DIM, GREEN, RED, YELLOW};
use crate::i18n;
use crate::optimize::{self, Risk};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new(i18n::opt_blurb())
        .size(12.0)
        .color(FG_DIM),
    );
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        if ui.button(i18n::btn_refresh()).clicked() {
            app.refresh_tweaks();
        }
        if ui
            .add_enabled(app.elevated, egui::Button::new(i18n::opt_btn_apply_all()).fill(super::ACCENT))
            .on_disabled_hover_text(i18n::opt_needs_admin())
            .clicked()
        {
            apply_all_safe(app, ui);
        }
        if !app.elevated {
            ui.label(
                egui::RichText::new(i18n::opt_read_only())
                    .size(11.0)
                    .color(YELLOW),
            );
        }
    });
    ui.add_space(10.0);

    let tweaks = optimize::all();
    let states = app.tweak_states.clone();

    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        egui::Grid::new("tweaks").num_columns(3).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
            ui.label(egui::RichText::new(i18n::opt_col_change()).size(11.0).color(FG_DIM));
            ui.label(egui::RichText::new(i18n::opt_col_state()).size(11.0).color(FG_DIM));
            ui.label(egui::RichText::new(i18n::opt_col_risk()).size(11.0).color(FG_DIM));
            ui.end_row();

            for (i, t) in tweaks.iter().enumerate() {
                let (_, text, optimal) = states
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| (t.id().into(), "…".into(), None));
                let colour = match optimal {
                    Some(true) => GREEN,
                    Some(false) => YELLOW,
                    None => FG_DIM,
                };
                let selected = app.selected_tweak == Some(i);
                if ui
                    .selectable_label(selected, egui::RichText::new(t.title()).color(colour).size(12.0))
                    .clicked()
                {
                    app.selected_tweak = Some(i);
                }
                ui.label(egui::RichText::new(text).size(12.0).color(FG_DIM));
                ui.label(
                    egui::RichText::new(t.risk().label())
                        .size(11.0)
                        .color(if t.risk() == Risk::Low { GREEN } else { YELLOW }),
                );
                ui.end_row();
            }
        });
    });

    ui.add_space(12.0);
    detail_panel(app, ui, &tweaks);
}

fn detail_panel(app: &mut App, ui: &mut egui::Ui, tweaks: &[Box<dyn optimize::Tweak>]) {
    egui::Frame::none()
        .fill(super::BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_min_height(150.0);
            let Some(i) = app.selected_tweak else {
                ui.label(egui::RichText::new(i18n::opt_select_tweak()).color(FG_DIM));
                return;
            };
            let Some(t) = tweaks.get(i) else { return };

            ui.label(egui::RichText::new(t.title()).size(15.0).strong().color(FG));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(i18n::opt_what_it_does(t.what())).size(12.0).color(FG));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(i18n::opt_why_it_helps(t.why())).size(12.0).color(FG_DIM));
            ui.add_space(6.0);

            let mut notes = vec![i18n::opt_risk_note(t.risk().label())];
            if t.needs_reboot() {
                notes.push(i18n::opt_needs_reboot().into());
            }
            if !t.reversible() {
                notes.push(i18n::opt_irreversible().into());
            }
            ui.label(egui::RichText::new(notes.join(" · ")).size(11.0).color(YELLOW));

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let can_apply = app.elevated || !t.needs_admin();
                if ui
                    .add_enabled(can_apply, egui::Button::new(i18n::btn_apply()).fill(super::ACCENT))
                    .on_disabled_hover_text(i18n::opt_needs_admin())
                    .clicked()
                {
                    let net = app.net.clone();
                    let now = ui.input(|inp| inp.time);
                    match optimize::apply(t.as_ref(), &net) {
                        Ok(msg) => {
                            app.store.log_tweak(t.id(), "apply", "", &msg);
                            app.toast(msg, GREEN, now);
                        }
                        Err(e) => {
                            app.store.log_tweak(t.id(), "apply", "", &e.to_string());
                            app.toast(e.to_string(), RED, now);
                        }
                    }
                    app.refresh_tweaks();
                }

                let can_revert =
                    t.reversible() && optimize::has_snapshot(t.id()) && (app.elevated || !t.needs_admin());
                if ui
                    .add_enabled(can_revert, egui::Button::new(i18n::btn_revert()))
                    .on_disabled_hover_text(i18n::opt_nothing_to_revert())
                    .clicked()
                {
                    let net = app.net.clone();
                    let now = ui.input(|inp| inp.time);
                    match optimize::revert(t.as_ref(), &net) {
                        Ok(msg) => {
                            app.store.log_tweak(t.id(), "revert", "", &msg);
                            app.toast(msg, GREEN, now);
                        }
                        Err(e) => app.toast(e.to_string(), RED, now),
                    }
                    app.refresh_tweaks();
                }
            });
        });
}

/// Applies every low-risk, reversible tweak that is not already in place.
fn apply_all_safe(app: &mut App, ui: &mut egui::Ui) {
    let net = app.net.clone();
    let now = ui.input(|i| i.time);
    let mut applied = 0;
    let mut failed = 0;

    for t in optimize::all() {
        if t.risk() != Risk::Low || !t.reversible() {
            continue;
        }
        if t.read(&net).optimal != Some(false) {
            continue;
        }
        match optimize::apply(t.as_ref(), &net) {
            Ok(msg) => {
                app.store.log_tweak(t.id(), "apply", "", &msg);
                applied += 1;
            }
            Err(e) => {
                app.store.log_tweak(t.id(), "apply", "", &e.to_string());
                failed += 1;
            }
        }
    }
    app.refresh_tweaks();

    let text = match (applied, failed) {
        (0, 0) => i18n::opt_all_ok().to_string(),
        (n, 0) => i18n::opt_applied(n),
        (n, f) => i18n::opt_applied_partial(n, f),
    };
    app.toast(text, if failed == 0 { GREEN } else { YELLOW }, now);
}
