//! Outage history: when, how long, and whose fault.

use eframe::egui;

use super::{App, FG, FG_DIM, GREEN, RED, YELLOW};
use crate::diagnose::format_datetime;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let day = app.store.events_since(24.0 * 3600.0);

    if day.is_empty() {
        ui.label(
            egui::RichText::new("No outages in the last 24 hours.")
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
            "lan" => "between this PC and the router",
            "adapter" => "on the network adapter",
            "isp" => "on the provider side",
            "dns" => "in DNS",
            _ => "as degraded quality",
        };
        ui.label(
            egui::RichText::new(format!(
                "{} outage(s) in the last 24 hours, mostly {where_text}.",
                day.len()
            ))
            .size(16.0)
            .strong()
            .color(RED),
        );
    }

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(
            "Each entry records the connection state at that moment — signal, channel, access \
             point and whether the router was still answering. That last detail is what decides \
             whether it was your laptop or your provider.",
        )
        .size(12.0)
        .color(FG_DIM),
    );
    ui.add_space(10.0);

    let events = app.store.recent_events(300);
    if events.is_empty() {
        ui.label(egui::RichText::new("Nothing logged yet.").color(FG_DIM));
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("history")
            .num_columns(4)
            .striped(true)
            .spacing([16.0, 5.0])
            .show(ui, |ui| {
                for h in ["Started", "Duration", "Kind", "Detail"] {
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
                            None => "ongoing".into(),
                        })
                        .size(11.0)
                        .color(colour),
                    );
                    ui.label(egui::RichText::new(&e.kind).size(11.0).color(colour));
                    ui.label(egui::RichText::new(&e.detail).size(11.0).color(FG_DIM));
                    ui.end_row();
                }
            });
    });
}
