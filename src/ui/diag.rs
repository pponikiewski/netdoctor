//! Diagnose tab: run the checks, browse findings, jump to the fix.

use std::sync::Arc;

use eframe::egui;

use super::{S_MD, S_SM, T_BODY, T_HEAD, T_TITLE, App, Job, FG, FG_DIM, GREEN, RED, YELLOW};
use crate::diagnose::{self, Severity};
use crate::i18n;

fn severity_colour(s: Severity) -> egui::Color32 {
    match s {
        Severity::Critical => RED,
        Severity::Warn => YELLOW,
        Severity::Info => super::ACCENT,
        Severity::Good => GREEN,
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

    let summary = diagnose::summarise(&app.findings);
    let has_critical = app.findings.iter().any(|f| f.severity == Severity::Critical);
    ui.label(
        egui::RichText::new(summary)
            .size(T_TITLE)
            .strong()
            .color(if has_critical { RED } else { FG }),
    );
    ui.add_space(S_MD);

    ui.columns(2, |cols| {
        egui::ScrollArea::vertical().id_salt("findings").show(&mut cols[0], |ui| {
            for (i, f) in app.findings.iter().enumerate() {
                let selected = app.selected_finding == Some(i);
                ui.push_id(&f.key, |ui| {
                let row = ui.selectable_label(
                    selected,
                    egui::RichText::new(format!("{:>8}  {}", f.severity.label(), f.title))
                        .color(severity_colour(f.severity))
                        .size(T_HEAD),
                );
                if row.clicked() {
                    app.selected_finding = Some(i);
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
                match app.selected_finding.and_then(|i| app.findings.get(i)).cloned() {
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
                                app.tab = super::Tab::Optimise;
                                app.refresh_tweaks();
                                app.selected_tweak =
                                    app.tweak_states.iter().position(|(tid, _, _)| tid == id);
                            }
                        }
                    }
                }
            });
    });
}

fn start_scan(app: &mut App) {
    app.scanning = true;
    app.scan_progress = 0.0;
    app.scan_label = i18n::diag_scan_starting().into();

    let tx = app.tx.clone();
    let store = Arc::clone(&app.store);
    let settings = app.settings.clone();
    let net = app.net.clone();

    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress: diagnose::Progress = Arc::new(move |label: &str, frac: f32| {
            let _ = progress_tx.send(Job::ScanProgress(label.to_string(), frac));
        });
        let findings = diagnose::scan(&net, &store, &settings, Some(progress));
        let _ = tx.send(Job::ScanDone(findings));
    });
}
