//! Outage history: when, how long, and whose fault.

use eframe::egui;

use super::{App, FG, FG_DIM, GREEN, RED, YELLOW};
use crate::diagnose::format_datetime;
use crate::i18n;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let day = app.store.events_since(24.0 * 3600.0);

    if day.is_empty() {
        ui.label(
            egui::RichText::new(i18n::hist_none_24h())
                .size(16.0)
                .strong()
                .color(GREEN),
        );
    } else {
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for e in &day {
            *counts.entry(e.scope.as_str()).or_default() += 1;
        }
        let worst = counts.iter().max_by_key(|(_, n)| **n).map(|(s, _)| *s).unwrap_or("");
        let where_text = match worst {
            "lan" => i18n::hist_where_lan(),
            "adapter" => i18n::hist_where_adapter(),
            "isp" => i18n::hist_where_isp(),
            "dns" => i18n::hist_where_dns(),
            _ => i18n::hist_where_other(),
        };
        ui.label(
            egui::RichText::new(i18n::hist_summary(day.len(), where_text))
            .size(16.0)
            .strong()
            .color(RED),
        );
    }

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(i18n::hist_blurb())
        .size(12.0)
        .color(FG_DIM),
    );
    ui.add_space(10.0);

    let events = app.store.recent_events(300);
    if events.is_empty() {
        ui.label(egui::RichText::new(i18n::hist_nothing_logged()).color(FG_DIM));
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
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
                    ui.label(egui::RichText::new(h).size(11.0).color(FG_DIM));
                }
                ui.end_row();

                for e in &events {
                    let colour = match e.scope.as_str() {
                        "lan" | "adapter" | "isp" => RED,
                        _ => YELLOW,
                    };
                    ui.label(
                        egui::RichText::new(format_datetime(e.ts_start))
                            .size(11.0)
                            .monospace()
                            .color(FG),
                    );
                    ui.label(
                        egui::RichText::new(match e.duration_s() {
                            Some(d) => format!("{d:.0} s"),
                            None => i18n::hist_ongoing().into(),
                        })
                        .size(11.0)
                        .color(colour),
                    );
                    ui.label(egui::RichText::new(i18n::event_kind(&e.kind)).size(11.0).color(colour));
                    ui.label(egui::RichText::new(&e.detail).size(11.0).color(FG_DIM));
                    ui.end_row();
                }
            });
    });
}
