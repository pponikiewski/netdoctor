//! Diagnose tab: the verdict first, the evidence behind it second.
//!
//! The scan used to hand the user a flat list of a dozen findings, most of
//! them green, and leave the reading of it to them. Ordering by severity is
//! not the same as answering the question, so the tab now leads with the one
//! thing the scan concluded — which segment of the chain is at fault, what it
//! costs, what to do — and keeps the findings underneath as the evidence for
//! that conclusion. Everything that came back healthy is collapsed: it is
//! worth being able to confirm, and not worth reading past.

use std::sync::Arc;

use eframe::egui;

use super::{S_LG, S_MD, S_SM, T_BODY, T_HEAD, T_TITLE, App, Job, FG, FG_DIM, GREEN, RED, YELLOW};
use crate::diagnose::{self, Segment, Severity, Verdict};
use crate::i18n;

fn severity_colour(s: Severity) -> egui::Color32 {
    match s {
        Severity::Critical => RED,
        Severity::Warn => YELLOW,
        Severity::Info => super::ACCENT,
        Severity::Good => GREEN,
    }
}

fn segment_colour(seg: Segment) -> egui::Color32 {
    match seg {
        Segment::Healthy => GREEN,
        Segment::Config => super::ACCENT,
        _ => RED,
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!app.scanning, egui::Button::new(i18n::diag_btn_scan()).fill(super::ACCENT))
            .clicked()
        {
            start_scan(app);
        }
        ui.add_enabled(
            !app.scanning,
            egui::Checkbox::new(&mut app.deep_scan, i18n::diag_deep()),
        )
        .on_hover_text(i18n::diag_deep_hint());
        ui.label(egui::RichText::new(&app.scan_label).size(T_BODY).color(FG_DIM));
    });

    if app.scanning {
        ui.add(egui::ProgressBar::new(app.scan_progress).desired_height(6.0));
    }
    ui.add_space(S_SM);

    if app.findings.is_empty() {
        ui.label(
            egui::RichText::new(i18n::diag_no_scan_yet())
            .size(T_HEAD)
            .color(FG_DIM),
        );
        return;
    }

    let mut jump_to: Option<String> = None;
    verdict_card(app, ui, &mut jump_to);
    if let Some(id) = jump_to {
        open_tweak(app, &id);
        return;
    }
    ui.add_space(S_MD);

    // Which row was clicked is decided first and applied afterwards, so the
    // list can read `app.findings` while the selection is written back.
    let mut picked: Option<usize> = None;
    let mut fix: Option<String> = None;
    let findings = &app.findings;
    let selected_finding = app.selected_finding;

    ui.columns(2, |cols| {
        egui::ScrollArea::vertical().id_salt("findings").show(&mut cols[0], |ui| {
            let mut row = |ui: &mut egui::Ui, i: usize, f: &crate::diagnose::Finding| {
                let hit = ui
                    .push_id(&f.key, |ui| {
                        ui.selectable_label(
                            selected_finding == Some(i),
                            egui::RichText::new(format!("{:>8}  {}", f.severity.label(), f.title))
                                .color(severity_colour(f.severity))
                                .size(T_HEAD),
                        )
                    })
                    .inner;
                if hit.clicked() {
                    picked = Some(i);
                }
            };

            // Anything that needs attention stays in the open.
            let mut healthy = Vec::new();
            for (i, f) in findings.iter().enumerate() {
                if f.severity == Severity::Good {
                    healthy.push(i);
                } else {
                    row(ui, i, f);
                }
            }

            // The rest is not a result, it is a reassurance. Worth being able
            // to open, never worth pushing the real findings off the screen.
            if !healthy.is_empty() {
                ui.add_space(S_SM);
                egui::CollapsingHeader::new(
                    egui::RichText::new(i18n::diag_checked_ok(healthy.len()))
                        .size(T_BODY)
                        .color(FG_DIM),
                )
                .id_salt("healthy")
                .show(ui, |ui| {
                    for i in healthy {
                        row(ui, i, &findings[i]);
                    }
                });
            }
        });

        let detail_col = &mut cols[1];
        egui::Frame::none()
            .fill(super::BG2)
            .rounding(6.0)
            .inner_margin(egui::Margin::same(S_MD))
            .show(detail_col, |ui| {
                ui.set_min_height(240.0);
                match selected_finding.and_then(|i| findings.get(i)) {
                    None => {
                        ui.label(
                            egui::RichText::new(i18n::diag_select_finding()).color(FG_DIM),
                        );
                    }
                    Some(f) => {
                        ui.label(
                            egui::RichText::new(&f.title)
                                .size(T_TITLE)
                                .strong()
                                .color(severity_colour(f.severity)),
                        );
                        ui.add_space(S_SM);
                        if !f.detail.is_empty() {
                            ui.label(egui::RichText::new(&f.detail).size(T_BODY).color(FG));
                            ui.add_space(S_SM);
                        }
                        if !f.advice.is_empty() {
                            ui.label(egui::RichText::new(&f.advice).size(T_BODY).color(FG_DIM));
                        }
                        if let Some(id) = &f.tweak_id {
                            ui.add_space(S_MD);
                            if ui.button(i18n::diag_btn_fix()).clicked() {
                                fix = Some(id.clone());
                            }
                        }
                    }
                }
            });
    });

    if let Some(i) = picked {
        app.selected_finding = Some(i);
    }
    if let Some(id) = fix {
        open_tweak(app, &id);
    }
}

/// The verdict: one segment, how sure the scan is, what it costs, and the
/// short ordered plan. This is the part most users will read and nothing else.
fn verdict_card(app: &App, ui: &mut egui::Ui, jump_to: &mut Option<String>) {
    let v: &Verdict = &app.verdict;
    let colour = segment_colour(v.segment);

    egui::Frame::none()
        .fill(super::BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(S_MD))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::verdict_heading()).size(T_BODY).color(FG_DIM),
            );
            ui.label(
                egui::RichText::new(i18n::verdict_confident(
                    v.segment.label(),
                    v.confidence.label(),
                ))
                .size(T_TITLE)
                .strong()
                .color(colour),
            );

            if let Some(split) = &v.split {
                ui.add_space(S_SM);
                ui.label(egui::RichText::new(split).size(T_BODY).color(FG_DIM));
            }

            ui.add_space(S_MD);
            ui.label(egui::RichText::new(i18n::verdict_cost_heading()).size(T_BODY).color(FG_DIM));
            ui.label(egui::RichText::new(&v.cost).size(T_HEAD).color(FG));

            if v.actions.is_empty() {
                return;
            }
            ui.add_space(S_LG);
            ui.label(
                egui::RichText::new(i18n::verdict_actions_heading()).size(T_BODY).color(FG_DIM),
            );
            for (n, action) in v.actions.iter().enumerate() {
                ui.add_space(S_SM);
                ui.horizontal_top(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{}.", n + 1))
                            .size(T_HEAD)
                            .strong()
                            .color(colour),
                    );
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&action.text).size(T_BODY).color(FG));
                        if let Some(id) = &action.tweak_id {
                            if ui.button(i18n::diag_btn_fix()).clicked() {
                                *jump_to = Some(id.clone());
                            }
                        }
                    });
                });
            }
        });
}

fn open_tweak(app: &mut App, id: &str) {
    app.tab = super::Tab::Optimise;
    app.refresh_tweaks();
    app.selected_tweak = app.tweak_states.iter().position(|(tid, _, _)| tid == id);
}

fn start_scan(app: &mut App) {
    app.scanning = true;
    app.scan_progress = 0.0;
    app.scan_label = i18n::diag_scan_starting().into();
    // The load test saturates the line. Anything the monitor sampled during it
    // would be measuring the test rather than the connection, and would then
    // pollute the very baseline the next scan compares against.
    app.monitor_was_paused = app.monitor.is_paused();
    if app.deep_scan {
        app.monitor.set_paused(true);
    }

    let tx = app.tx.clone();
    let store = Arc::clone(&app.store);
    let settings = app.settings.clone();
    let net = app.net.clone();
    let deep = app.deep_scan;

    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress: diagnose::Progress = Arc::new(move |label: &str, frac: f32| {
            let _ = progress_tx.send(Job::ScanProgress(label.to_string(), frac));
        });
        let scan = diagnose::scan(&net, &store, &settings, deep, Some(progress));
        let _ = tx.send(Job::ScanDone(Box::new(scan)));
    });
}
