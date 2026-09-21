//! Optimise tab: inspect, apply and revert each tweak.

use eframe::egui;

use super::{
    figure, App, FG, FG_DIM, GREEN, RED, S_LG, S_MD, S_SM, S_XS, T_BODY, T_HEAD, T_META, T_TITLE,
    YELLOW,
};
use crate::i18n;
use crate::optimize::{self, Risk};

/// How a row reads at a glance. The colour alone was doing this job before,
/// which meant the list could only be understood by someone who already knew
/// the colour scheme; the word is what people actually read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Already the way the tweak wants it.
    Set,
    /// Readable, changeable, and currently not what the tweak wants.
    Todo,
    /// Nothing to decide: wrong medium, driver does not expose it, or it
    /// cannot be read. The state column says which.
    Unavailable,
}

impl Status {
    fn of(optimal: Option<bool>) -> Status {
        match optimal {
            Some(true) => Status::Set,
            Some(false) => Status::Todo,
            None => Status::Unavailable,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Status::Set => i18n::st_set(),
            Status::Todo => i18n::st_todo(),
            Status::Unavailable => i18n::st_na(),
        }
    }

    fn colour(&self) -> egui::Color32 {
        match self {
            Status::Set => GREEN,
            Status::Todo => YELLOW,
            Status::Unavailable => FG_DIM,
        }
    }
}

/// Height of the detail panel with a tweak open, and with nothing
/// selected. Kept here because the list above is sized against them.
const DETAIL_OPEN: f32 = 185.0;
const DETAIL_EMPTY: f32 = 44.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(i18n::opt_blurb()).size(T_BODY).color(FG_DIM));
    ui.add_space(S_SM);

    // A damaged snapshot file used to just make every Revert button vanish,
    // which reads as "nothing was ever applied" rather than as a fault.
    if let Some(err) = optimize::snapshots_error() {
        ui.label(egui::RichText::new(i18n::tw_snapshots_unreadable_hint()).size(T_META).color(RED));
        ui.label(egui::RichText::new(err).size(T_META).color(FG_DIM));
        ui.add_space(S_SM);
    }

    // Reading the twenty-one states takes a third of a second of `netsh` and
    // `powercfg`, so it runs on a worker thread. While it does, the rows on
    // screen are whatever the last pass found, and the two buttons that act
    // on that reading are held back rather than acting on a stale one.
    let loading = app.tweaks_loading();

    ui.horizontal(|ui| {
        if ui.add_enabled(!loading, egui::Button::new(i18n::btn_refresh())).clicked() {
            app.refresh_tweaks();
        }
        if ui
            .add_enabled(
                app.elevated && !loading,
                egui::Button::new(i18n::opt_btn_apply_all()).fill(super::ACCENT),
            )
            .on_disabled_hover_text(if app.elevated {
                i18n::opt_reading()
            } else {
                i18n::opt_needs_admin()
            })
            .clicked()
        {
            apply_all_safe(app, ui);
        }
        if loading {
            ui.label(egui::RichText::new(i18n::opt_reading()).size(T_META).color(super::ACCENT));
        }
        if !app.elevated {
            ui.label(egui::RichText::new(i18n::opt_read_only()).size(T_META).color(YELLOW));
        }

        // The tally sits on the same line, right-aligned: it answers "is
        // there anything left to do here" without reading a single row.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (set, todo, na) = tally(app);
            ui.label(
                egui::RichText::new(i18n::opt_summary(set, todo, na)).size(T_META).color(FG_DIM),
            );
            if na > 0 {
                ui.checkbox(&mut app.show_unavailable, i18n::opt_show_unavailable())
                    .on_hover_text(i18n::opt_show_unavailable_hint());
            }
        });
    });
    ui.add_space(S_MD);

    air_panel(app, ui);
    ui.add_space(S_MD);

    // Escape backs out of a selection. The detail panel is the only thing on
    // this tab that holds a mode, and reaching for the mouse to leave it is
    // the kind of friction nobody reports and everybody feels.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.selected_tweak = None;
    }

    let tweaks = optimize::all();

    // The list takes whatever the detail panel below does not need, rather
    // than a hardcoded height that overflows on a short window. With nothing
    // selected that panel is one line of text, so it shrinks and the list
    // gets the difference instead of leaving a dead grey box on screen.
    let detail_height = if app.selected_tweak.is_some() { DETAIL_OPEN } else { DETAIL_EMPTY };
    let list_height = (ui.available_height() - detail_height - 40.0).max(160.0);
    egui::ScrollArea::vertical().max_height(list_height).show(ui, |ui| {
        for category in optimize::Category::ALL {
            section(app, ui, &tweaks, category);
        }
    });

    ui.add_space(S_MD);
    detail_panel(app, ui, &tweaks, detail_height);
}

/// How many tweaks are set, worth changing, and not on offer at all.
fn tally(app: &App) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    for (_, _, optimal) in &app.tweak_states {
        match Status::of(*optimal) {
            Status::Set => counts.0 += 1,
            Status::Todo => counts.1 += 1,
            Status::Unavailable => counts.2 += 1,
        }
    }
    counts
}

/// One category, with its own heading, count and table.
fn section(
    app: &mut App,
    ui: &mut egui::Ui,
    tweaks: &[Box<dyn optimize::Tweak>],
    category: optimize::Category,
) {
    let all_rows: Vec<usize> =
        (0..tweaks.len()).filter(|i| tweaks[*i].category() == category).collect();
    if all_rows.is_empty() {
        return;
    }

    let states = app.tweak_states.clone();
    let status_at = |i: usize| Status::of(states.get(i).and_then(|(_, _, optimal)| *optimal));
    // The counts describe the whole section, whether or not every row of it
    // is on screen, so hiding rows never changes what the heading claims.
    let set = all_rows.iter().filter(|i| status_at(**i) == Status::Set).count();
    let available = all_rows.iter().filter(|i| status_at(**i) != Status::Unavailable).count();

    let mut rows: Vec<usize> = all_rows
        .into_iter()
        .filter(|i| app.show_unavailable || status_at(*i) != Status::Unavailable)
        .collect();
    if rows.is_empty() {
        return;
    }

    // What is left to do comes first, what is already set after it, and what
    // cannot be touched here last. Declaration order breaks ties, so the list
    // is stable between frames and between runs. Before this the three kinds
    // were interleaved in declaration order, which meant finding the pending
    // ones was a read of every row in the section.
    rows.sort_by_key(|i| {
        (
            match status_at(*i) {
                Status::Todo => 0,
                Status::Set => 1,
                Status::Unavailable => 2,
            },
            *i,
        )
    });

    egui::Frame::none()
        .fill(super::BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(S_MD, S_MD))
        .outer_margin(egui::Margin { bottom: 8.0, right: 4.0, ..Default::default() })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(category.title()).size(T_HEAD).strong().color(FG));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let summary = if available == 0 {
                        i18n::opt_section_none().to_string()
                    } else if set == available {
                        i18n::opt_section_all_set().to_string()
                    } else {
                        i18n::opt_section_count(set, available)
                    };
                    let colour = if available > 0 && set == available { GREEN } else { FG_DIM };
                    ui.label(egui::RichText::new(summary).size(T_META).color(colour));
                });
            });
            ui.label(egui::RichText::new(category.blurb()).size(T_META).color(FG_DIM));
            ui.add_space(S_SM);

            egui::Grid::new(("tweaks", category.title()))
                .num_columns(4)
                .striped(true)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    for i in rows {
                        let (_, text, optimal) = states
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| (tweaks[i].id().into(), "\u{2026}".into(), None));
                        let status = Status::of(optimal);
                        let t = &tweaks[i];

                        ui.label(
                            egui::RichText::new(status.label()).size(T_META).color(status.colour()),
                        );

                        let selected = app.selected_tweak == Some(i);
                        let title = egui::RichText::new(t.title())
                            .size(T_BODY)
                            .color(if status == Status::Unavailable { FG_DIM } else { FG });
                        if ui.selectable_label(selected, title).clicked() {
                            app.selected_tweak = Some(i);
                        }

                        ui.label(egui::RichText::new(text).size(T_BODY).color(FG_DIM));

                        // A tweak nobody can apply has no risk to report.
                        if status == Status::Unavailable {
                            ui.label(egui::RichText::new("\u{2014}").size(T_META).color(FG_DIM));
                        } else {
                            ui.label(egui::RichText::new(t.risk().label()).size(T_META).color(
                                match t.risk() {
                                    Risk::Low => GREEN,
                                    Risk::Medium => YELLOW,
                                    Risk::High => RED,
                                },
                            ));
                        }
                        ui.end_row();
                    }
                });
        });
}

/// The channel advice. Collapsed by default: it is a different question from
/// the rest of the tab — everything else here changes this machine, and this
/// one produces a sentence to type into the router.
fn air_panel(app: &mut App, ui: &mut egui::Ui) {
    egui::CollapsingHeader::new(egui::RichText::new(i18n::air_title()).size(T_HEAD).color(FG))
        .id_salt("air_scan")
        .show(ui, |ui| {
            ui.label(egui::RichText::new(i18n::air_blurb()).size(T_META).color(FG_DIM));
            ui.add_space(S_SM);

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!app.air_scanning, egui::Button::new(i18n::air_btn_scan()))
                    .on_hover_text(i18n::air_scan_cost())
                    .clicked()
                {
                    start_air_scan(app, ui.ctx().clone());
                }
                if app.air_scanning {
                    ui.label(egui::RichText::new(i18n::air_scanning()).size(T_META).color(FG_DIM));
                }
            });
            ui.add_space(S_SM);

            if let Some(err) = &app.air.error {
                ui.label(egui::RichText::new(i18n::air_failed(err)).size(T_BODY).color(YELLOW));
                return;
            }
            if app.air.aps.is_empty() {
                ui.label(egui::RichText::new(i18n::air_empty()).size(T_BODY).color(FG_DIM));
                return;
            }

            air_advice(app, ui);
            ui.add_space(S_SM);
            air_table(app, ui);
        });
}

fn air_advice(app: &App, ui: &mut egui::Ui) {
    let air = &app.air;
    ui.label(
        egui::RichText::new(i18n::air_seen(air.aps.len(), air.co_channel())).size(T_BODY).color(FG),
    );
    if let Some(cur) = air.current {
        let noise = air.load_24.iter().chain(air.load_5.iter()).find(|l| l.channel == cur);
        ui.label(
            egui::RichText::new(i18n::air_current_line(cur, noise.and_then(|l| l.noise_dbm)))
                .size(T_BODY)
                .color(FG_DIM),
        );
    }
    ui.add_space(S_SM);

    if air.worth_moving_24() {
        // `worth_moving_24` has already established both numbers exist.
        let best = air.best_24.unwrap_or(1);
        let gain = gain_24(app, best);
        ui.label(
            egui::RichText::new(i18n::air_best_24(best, gain)).size(T_HEAD).strong().color(GREEN),
        );
        ui.label(egui::RichText::new(i18n::air_router_note()).size(T_META).color(FG_DIM));
    } else if air.current.map(|c| (1..=14).contains(&c)).unwrap_or(false) {
        ui.label(egui::RichText::new(i18n::air_quiet_here()).size(T_BODY).color(GREEN));
    }

    ui.add_space(S_XS);
    ui.label(
        egui::RichText::new(
            crate::probe::airscan::CLEAN_24
                .iter()
                .filter_map(|c| air.load_24.iter().find(|l| l.channel == *c))
                .map(|l| i18n::air_load_cell(l.channel, l.aps, l.noise_dbm))
                .collect::<Vec<_>>()
                .join("   ·   "),
        )
        .size(T_META)
        .color(FG_DIM),
    );

    if air.on_dfs() {
        ui.add_space(S_SM);
        ui.label(
            egui::RichText::new(i18n::air_on_dfs(air.current.unwrap_or(0)))
                .size(T_HEAD)
                .strong()
                .color(YELLOW),
        );
        ui.label(egui::RichText::new(i18n::air_dfs_move()).size(T_META).color(FG_DIM));
    }

    if let Some(best5) = air.best_5 {
        ui.add_space(S_XS);
        ui.label(egui::RichText::new(i18n::air_best_5(best5)).size(T_BODY).color(FG));
        ui.label(egui::RichText::new(i18n::air_dfs_note()).size(T_META).color(FG_DIM));
    }
}

/// How much quieter the recommended channel is than the current one, in dB.
fn gain_24(app: &App, best: u32) -> f64 {
    let noise =
        |ch: u32| app.air.load_24.iter().find(|l| l.channel == ch).and_then(|l| l.noise_dbm);
    match (app.air.current.and_then(noise), noise(best)) {
        (Some(cur), Some(b)) => cur - b,
        (Some(cur), None) => cur + 100.0,
        _ => 0.0,
    }
}

fn air_table(app: &App, ui: &mut egui::Ui) {
    egui::Grid::new("air_aps").num_columns(3).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
        for (label, _) in [
            (i18n::air_col_network(), ()),
            (i18n::air_col_channel(), ()),
            (i18n::air_col_signal(), ()),
        ] {
            ui.label(egui::RichText::new(label).size(T_META).color(FG_DIM));
        }
        ui.end_row();

        for ap in app.air.aps.iter().take(12) {
            let name = if ap.ssid.is_empty() {
                i18n::air_hidden_ssid().to_string()
            } else if ap.ours {
                format!("{} ({})", ap.ssid, i18n::air_yours())
            } else {
                ap.ssid.clone()
            };
            ui.label(egui::RichText::new(name).size(T_BODY).color(if ap.ours {
                GREEN
            } else {
                FG
            }))
            // Mesh nodes share an SSID, so the only thing telling two rows
            // apart is the radio's own address.
            .on_hover_text(&ap.bssid);
            ui.label(
                egui::RichText::new(i18n::air_channel_cell(ap.channel, ap.band.label()))
                    .size(T_BODY)
                    .color(FG_DIM),
            );
            // Signal is the column this table is sorted and read by, so its
            // digits line up rather than drifting with the glyph widths.
            ui.label(figure(
                format!("{} dBm", ap.rssi_dbm),
                T_BODY,
                if ap.rssi_dbm > -70 { FG } else { FG_DIM },
            ));
            ui.end_row();
        }
    });
}

/// The sweep itself blocks for about four seconds while the card retunes, so
/// it never happens on the UI thread.
///
/// It also costs about a second of connectivity: the card has to leave the
/// channel it is associated on to listen to the others. Left alone, the
/// monitor sees that as an outage and files it, so a history kept to explain
/// real failures would fill up with failures this app caused. The load test
/// pauses the monitor for the same reason.
fn start_air_scan(app: &mut App, ctx: egui::Context) {
    let tx = app.tx.clone();
    let net = app.net.clone();
    app.air_scanning = true;
    app.monitor.set_paused(true);
    std::thread::spawn(move || {
        let result = crate::probe::airscan::rescan(&net);
        let _ = tx.send(crate::ui::Job::AirDone(Box::new(result)));
        ctx.request_repaint();
    });
}

fn detail_panel(
    app: &mut App,
    ui: &mut egui::Ui,
    tweaks: &[Box<dyn optimize::Tweak>],
    height: f32,
) {
    egui::Frame::none().fill(super::BG2).rounding(6.0).inner_margin(egui::Margin::same(S_LG)).show(
        ui,
        |ui| {
            ui.set_min_height(height);
            // Without this the empty state shrinks to the width of its one
            // line of text and reads as a rendering fault rather than as a
            // panel waiting for a selection.
            ui.set_min_width(ui.available_width());
            let Some(i) = app.selected_tweak else {
                ui.label(egui::RichText::new(i18n::opt_select_tweak()).color(FG_DIM));
                return;
            };
            let Some(t) = tweaks.get(i) else { return };

            ui.label(egui::RichText::new(t.title()).size(T_TITLE).strong().color(FG));
            ui.add_space(S_XS);

            // Repeat the status here. The row that was clicked scrolls out of
            // sight on a short window, and "what is it now" is the first
            // thing anyone asks before pressing Apply.
            let (state_text, optimal) = app
                .tweak_states
                .get(i)
                .map(|(_, text, optimal)| (text.clone(), *optimal))
                .unwrap_or_default();
            let status = Status::of(optimal);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(status.label()).size(T_META).color(status.colour()));
                ui.label(egui::RichText::new(state_text).size(T_META).color(FG_DIM));
            });
            if optimize::has_snapshot(t.as_ref(), &app.net) {
                ui.label(
                    egui::RichText::new(i18n::opt_revert_available())
                        .size(T_META)
                        .color(super::ACCENT),
                );
            }
            ui.add_space(S_MD);
            ui.horizontal(|ui| {
                let can_apply = app.elevated || !t.needs_admin();
                if ui
                    .add_enabled(
                        can_apply,
                        egui::Button::new(i18n::btn_apply()).fill(super::ACCENT),
                    )
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

                let can_revert = t.reversible()
                    && optimize::has_snapshot(t.as_ref(), &app.net)
                    && (app.elevated || !t.needs_admin());
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
            ui.add_space(S_SM);
            ui.label(egui::RichText::new(i18n::opt_what_it_does(t.what())).size(T_BODY).color(FG));
            ui.add_space(S_XS);
            ui.label(
                egui::RichText::new(i18n::opt_why_it_helps(t.why())).size(T_BODY).color(FG_DIM),
            );
            ui.add_space(S_SM);

            let mut notes = vec![i18n::opt_risk_note(t.risk().label())];
            if t.needs_reboot() {
                notes.push(i18n::opt_needs_reboot().into());
            }
            if !t.reversible() {
                notes.push(i18n::opt_irreversible().into());
            }
            ui.label(egui::RichText::new(notes.join(" · ")).size(T_META).color(YELLOW));
        },
    );
}

/// Whether "apply all safe" may touch a tweak, given what the last read found.
///
/// `known` is `None` when the list has never been read, `Some(None)` when the
/// tweak was read and could not be made out — a value the process may not
/// read, or one of the wrong type. Neither is permission to write. A tweak
/// whose current value is unknown has no "before" worth recording, so
/// applying it leaves Revert with nothing to go back to.
fn is_safe_candidate(risk: Risk, reversible: bool, known: Option<Option<bool>>) -> bool {
    risk == Risk::Low && reversible && known == Some(Some(false))
}

/// Applies every low-risk, reversible tweak that is not already in place.
///
/// "Not already in place" comes from the states the worker thread just read,
/// not from a second `read` per tweak: that second pass cost another 369 ms on
/// the UI thread to learn what the app had learnt moments earlier. The button
/// is disabled while a read is in flight, so the states here are never the
/// ones from before an apply.
fn apply_all_safe(app: &mut App, ui: &mut egui::Ui) {
    let net = app.net.clone();
    let now = ui.input(|i| i.time);
    let mut applied = 0;
    let mut failed = 0;
    let states = app.tweak_states.clone();

    for t in optimize::all() {
        let known = states.iter().find(|(id, _, _)| id == t.id()).map(|(_, _, opt)| *opt);
        if !is_safe_candidate(t.risk(), t.reversible(), known) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_all_safe_skips_what_it_could_not_read() {
        // A tweak whose value could not be read shows as Unavailable, and an
        // Unavailable tweak has no recorded "before". Applying it would leave
        // Revert with nothing to go back to, so the bulk button steps over it
        // however low its risk.
        assert!(!is_safe_candidate(Risk::Low, true, Some(None)), "read and could not be made out");
        assert!(!is_safe_candidate(Risk::Low, true, None), "never read at all");

        assert!(is_safe_candidate(Risk::Low, true, Some(Some(false))), "read, and worth changing");
        assert!(!is_safe_candidate(Risk::Low, true, Some(Some(true))), "already set");
        assert!(!is_safe_candidate(Risk::Medium, true, Some(Some(false))), "not low risk");
        assert!(!is_safe_candidate(Risk::Low, false, Some(Some(false))), "cannot be undone");
    }
}
