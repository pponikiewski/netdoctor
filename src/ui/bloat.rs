//! Load test tab: latency under saturation (bufferbloat).

use std::sync::Arc;

use eframe::egui;

use super::{stat_card, App, Job, FG, FG_DIM, GREEN, RED, YELLOW};
use crate::bandwidth::{self, Grade};
use crate::i18n;

fn grade_colour(g: Grade) -> egui::Color32 {
    match g {
        Grade::A | Grade::B => GREEN,
        Grade::C => YELLOW,
        Grade::D | Grade::F => RED,
        Grade::Unknown => FG_DIM,
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(i18n::bloat_title()).size(18.0).strong().color(FG));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(i18n::bloat_blurb())
        .size(12.0)
        .color(FG_DIM),
    );
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        if ui
            .add_enabled(!app.bloat_running, egui::Button::new(i18n::bloat_btn_run()).fill(super::ACCENT))
            .clicked()
        {
            start(app);
        }
        ui.label(egui::RichText::new(&app.bloat_label).size(12.0).color(FG_DIM));
    });
    if app.bloat_running {
        ui.add(egui::ProgressBar::new(app.bloat_progress).desired_height(6.0));
    }
    ui.add_space(12.0);

    let r = app.bloat.clone();
    ui.horizontal(|ui| {
        match r.idle_avg {
            Some(v) => stat_card(ui, i18n::bloat_card_idle(), &format!("{v:.0} ms"), "", FG),
            None => stat_card(ui, i18n::bloat_card_idle(), "—", "", FG_DIM),
        }
        let bump = r.bump_ms.unwrap_or(0.0);
        let colour = if bump < 60.0 { GREEN } else if bump < 150.0 { YELLOW } else { RED };
        match r.loaded_avg {
            Some(v) => stat_card(ui, i18n::bloat_card_loaded(), &format!("{v:.0} ms"), "", colour),
            None => stat_card(ui, i18n::bloat_card_loaded(), "—", "", FG_DIM),
        }
        match r.bump_ms {
            Some(v) => stat_card(ui, i18n::bloat_card_increase(), &format!("+{v:.0} ms"), "", colour),
            None => stat_card(ui, i18n::bloat_card_increase(), "—", "", FG_DIM),
        }
        match r.mbps {
            Some(v) => stat_card(
                ui,
                i18n::bloat_card_throughput(),
                &format!("{v:.0} Mbps"),
                i18n::bloat_card_throughput_sub(),
                FG,
            ),
            None => stat_card(ui, i18n::bloat_card_throughput(), "—", "", FG_DIM),
        }
        let g = r.grade_or_unknown();
        stat_card(ui, i18n::bloat_card_grade(), g.letter(), "", grade_colour(g));
    });

    ui.add_space(14.0);
    egui::Frame::none()
        .fill(super::BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let g = r.grade_or_unknown();
                if g != Grade::Unknown {
                    ui.label(egui::RichText::new(g.verdict()).size(13.0).strong().color(grade_colour(g)));
                    ui.add_space(6.0);
                }
                if let (Some(max), true) = (r.loaded_max, r.loaded_avg.is_some()) {
                    ui.label(
                        egui::RichText::new(i18n::bloat_worst(max, r.loaded_loss_pct))
                        .size(12.0)
                        .color(FG),
                    );
                    ui.add_space(6.0);
                }
                ui.label(egui::RichText::new(bandwidth::advice(&r)).size(12.0).color(FG_DIM));
            });
        });
}

fn start(app: &mut App) {
    app.bloat_running = true;
    app.bloat_progress = 0.0;
    app.bloat_label = i18n::bloat_starting().into();
    // Our own probes would otherwise count as part of the load.
    app.monitor.set_paused(true);

    let tx = app.tx.clone();
    let timeout = app.settings.ping_timeout_ms;
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress: bandwidth::Progress = Arc::new(move |label: &str, frac: f32| {
            let _ = progress_tx.send(Job::BloatProgress(label.to_string(), frac));
        });
        let res = bandwidth::run(
            std::net::Ipv4Addr::new(1, 1, 1, 1),
            std::time::Duration::from_secs(6),
            std::time::Duration::from_secs(12),
            timeout,
            Some(progress),
        );
        let _ = tx.send(Job::BloatDone(Box::new(res)));
    });
}
