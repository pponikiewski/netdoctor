//! Optimise tab: inspect, apply and revert each tweak.

use eframe::egui;

use super::{App, Job, FG, FG_DIM, GREEN, RED, YELLOW};
use crate::advise::{self, Candidate, Priority};
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
/// The same panel with a ranking verdict in it, which is two more lines.
const DETAIL_RANKED: f32 = 225.0;
const DETAIL_EMPTY: f32 = 44.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(i18n::opt_blurb()).size(12.0).color(FG_DIM));
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
            ui.label(egui::RichText::new(i18n::opt_read_only()).size(11.0).color(YELLOW));
        }

        // The tally sits on the same line, right-aligned: it answers "is
        // there anything left to do here" without reading a single row.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (set, todo, na) = tally(app);
            ui.label(egui::RichText::new(i18n::opt_summary(set, todo, na)).size(11.0).color(FG_DIM));
            if na > 0 {
                ui.checkbox(&mut app.show_unavailable, i18n::opt_show_unavailable())
                    .on_hover_text(i18n::opt_show_unavailable_hint());
            }
        });
    });
    ui.add_space(10.0);

    rank_panel(app, ui);
    ui.add_space(10.0);

    air_panel(app, ui);
    ui.add_space(10.0);

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
    let detail_height = match app.selected_tweak.and_then(|i| tweaks.get(i)) {
        Some(t) if app.priorities.contains_key(t.id()) => DETAIL_RANKED,
        Some(_) => DETAIL_OPEN,
        None => DETAIL_EMPTY,
    };
    let list_height = (ui.available_height() - detail_height - 40.0).max(160.0);
    egui::ScrollArea::vertical().max_height(list_height).show(ui, |ui| {
        for category in optimize::Category::ALL {
            section(app, ui, &tweaks, category);
        }
    });

    ui.add_space(12.0);
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
    let status_at = |i: usize| {
        Status::of(states.get(i).and_then(|(_, _, optimal)| *optimal))
    };
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
    // cannot be touched here last. Within the pending rows the ranking
    // decides the order when there is one. Declaration order is the tie
    // break, so the list is stable between frames and between runs.
    let scores = &app.priorities;
    rows.sort_by(|a, b| {
        let rank = |i: &usize| match status_at(*i) {
            Status::Todo => 0,
            Status::Set => 1,
            Status::Unavailable => 2,
        };
        let score = |i: &usize| {
            tweaks
                .get(*i)
                .and_then(|t| scores.get(t.id()))
                .filter(|p| p.is_recommendation())
                .map_or(f64::MIN, |p| p.score)
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| score(b).total_cmp(&score(a)))
            .then(a.cmp(b))
    });

    egui::Frame::none()
        .fill(super::BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .outer_margin(egui::Margin { bottom: 8.0, right: 4.0, ..Default::default() })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(category.title()).size(13.0).strong().color(FG));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let summary = if available == 0 {
                        i18n::opt_section_none().to_string()
                    } else if set == available {
                        i18n::opt_section_all_set().to_string()
                    } else {
                        i18n::opt_section_count(set, available)
                    };
                    let colour = if available > 0 && set == available { GREEN } else { FG_DIM };
                    ui.label(egui::RichText::new(summary).size(11.0).color(colour));
                });
            });
            ui.label(egui::RichText::new(category.blurb()).size(11.0).color(FG_DIM));
            ui.add_space(6.0);

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
                            egui::RichText::new(status.label()).size(11.0).color(status.colour()),
                        );

                        let selected = app.selected_tweak == Some(i);
                        let title = egui::RichText::new(t.title()).size(12.0).color(
                            if status == Status::Unavailable { FG_DIM } else { FG },
                        );
                        // The badge shares the title cell rather than taking
                        // a column of its own, which would sit empty on every
                        // row until a ranking has been asked for.
                        ui.horizontal(|ui| {
                            if let Some(p) = scores.get(t.id()) {
                                if let Some((text, colour)) = badge(p) {
                                    ui.label(
                                        egui::RichText::new(text).size(10.0).strong().color(colour),
                                    )
                                    .on_hover_text(i18n::adv_badge_hint(&p.level, p.confidence));
                                }
                            }
                            if ui.selectable_label(selected, title).clicked() {
                                app.selected_tweak = Some(i);
                            }
                        });

                        ui.label(egui::RichText::new(text).size(12.0).color(FG_DIM));

                        // A tweak nobody can apply has no risk to report.
                        if status == Status::Unavailable {
                            ui.label(egui::RichText::new("\u{2014}").size(11.0).color(FG_DIM));
                        } else {
                            ui.label(
                                egui::RichText::new(t.risk().label()).size(11.0).color(
                                    match t.risk() {
                                        Risk::Low => GREEN,
                                        Risk::Medium => YELLOW,
                                        Risk::High => RED,
                                    },
                                ),
                            );
                        }
                        ui.end_row();
                    }
                });
        });
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

/// The tweak ranking: one button, its result, and what it costs.
///
/// Off unless a key is configured, and it says so rather than showing a
/// button that cannot work. The privacy line is not a footnote — this is the
/// one feature in the app that sends anything anywhere, so it says what
/// leaves the machine on the same screen as the button that sends it.
fn rank_panel(app: &mut App, ui: &mut egui::Ui) {
    let configured = advise::key(&app.settings).is_some();
    let pending = candidates(app).len();

    ui.horizontal(|ui| {
        let enabled = configured && !app.advising && pending > 0;
        let button = ui
            .add_enabled(enabled, egui::Button::new(i18n::adv_btn_rank()))
            .on_hover_text(i18n::adv_sends_note());
        let button = if !configured {
            button.on_disabled_hover_text(i18n::adv_no_key())
        } else if pending == 0 {
            button.on_disabled_hover_text(i18n::adv_nothing_pending())
        } else {
            button
        };
        if button.clicked() {
            start_rank(app, ui.ctx().clone());
        }

        if app.advising {
            ui.spinner();
            ui.label(egui::RichText::new(i18n::adv_working()).size(11.0).color(FG_DIM));
        } else if !app.priorities.is_empty() {
            ui.label(
                egui::RichText::new(i18n::adv_ranked(app.priorities.len()))
                    .size(11.0)
                    .color(FG_DIM),
            );
            if ui.small_button(i18n::adv_btn_clear()).clicked() {
                app.priorities.clear();
                app.advice_error = None;
            }
        } else if !configured {
            ui.label(egui::RichText::new(i18n::adv_no_key()).size(11.0).color(FG_DIM));
        }
    });

    if let Some(err) = &app.advice_error {
        ui.label(egui::RichText::new(i18n::adv_failed(err)).size(11.0).color(YELLOW));
    }
}

/// The tweaks worth ranking: readable, not already applied, and applicable
/// here. Ranking a tweak that is already set, or one the driver does not
/// expose, would spend a question on an answer nobody can act on.
fn candidates(app: &App) -> Vec<Candidate> {
    let tweaks = optimize::all();
    let mut out = Vec::new();
    for (i, t) in tweaks.iter().enumerate() {
        let Some((_, text, optimal)) = app.tweak_states.get(i) else { continue };
        if *optimal != Some(false) {
            continue;
        }
        if t.needs_admin() && !app.elevated {
            continue;
        }
        out.push(Candidate {
            id: t.id().to_string(),
            title: t.title().to_string(),
            changes: t.what().to_string(),
            rationale: t.why().to_string(),
            risk: advise::risk_word(t.risk()).to_string(),
            current: text.clone(),
        });
    }
    advise::cap(out)
}

/// Builds the state on the UI thread, where the store and the settings are,
/// then hands the request to a worker. Nothing about the machine is read
/// after this point, so the ranking describes the moment the button was
/// pressed rather than whenever the reply happened to arrive.
fn start_rank(app: &mut App, ctx: egui::Context) {
    let Some(key) = advise::key(&app.settings) else { return };
    let candidates = candidates(app);
    if candidates.is_empty() {
        return;
    }
    let state = advise::state(
        &app.net,
        &app.last,
        &app.store,
        &app.settings,
        &app.air,
        &candidates,
    );

    app.advising = true;
    app.advice_error = None;
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let result = advise::rank(&key, &state, &candidates).map_err(|e| e.to_string());
        let _ = tx.send(Job::AdviceDone(result));
        ctx.request_repaint();
    });
}

/// The badge that marks a ranked row. Only the rows the model actually
/// recommends get one: a badge on every row would rank the list without
/// saying anything, which is how a priority column becomes decoration.
fn badge(p: &Priority) -> Option<(String, egui::Color32)> {
    if !p.is_recommendation() {
        return None;
    }
    let colour = if p.is_risky() { YELLOW } else { GREEN };
    Some((i18n::adv_badge(p.score), colour))
}

/// The channel advice. Collapsed by default: it is a different question from
/// the rest of the tab — everything else here changes this machine, and this
/// one produces a sentence to type into the router.
fn air_panel(app: &mut App, ui: &mut egui::Ui) {
    egui::CollapsingHeader::new(egui::RichText::new(i18n::air_title()).size(13.0).color(FG))
        .id_salt("air_scan")
        .show(ui, |ui| {
            ui.label(egui::RichText::new(i18n::air_blurb()).size(11.0).color(FG_DIM));
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!app.air_scanning, egui::Button::new(i18n::air_btn_scan()))
                    .on_hover_text(i18n::air_scan_cost())
                    .clicked()
                {
                    start_air_scan(app, ui.ctx().clone());
                }
                if app.air_scanning {
                    ui.label(egui::RichText::new(i18n::air_scanning()).size(11.0).color(FG_DIM));
                }
            });
            ui.add_space(6.0);

            if let Some(err) = &app.air.error {
                ui.label(egui::RichText::new(i18n::air_failed(err)).size(12.0).color(YELLOW));
                return;
            }
            if app.air.aps.is_empty() {
                ui.label(egui::RichText::new(i18n::air_empty()).size(12.0).color(FG_DIM));
                return;
            }

            air_advice(app, ui);
            ui.add_space(8.0);
            air_table(app, ui);
        });
}

fn air_advice(app: &App, ui: &mut egui::Ui) {
    let air = &app.air;
    ui.label(
        egui::RichText::new(i18n::air_seen(air.aps.len(), air.co_channel()))
            .size(12.0)
            .color(FG),
    );
    if let Some(cur) = air.current {
        let noise = air.load_24.iter().chain(air.load_5.iter()).find(|l| l.channel == cur);
        ui.label(
            egui::RichText::new(i18n::air_current_line(cur, noise.and_then(|l| l.noise_dbm)))
                .size(12.0)
                .color(FG_DIM),
        );
    }
    ui.add_space(6.0);

    if air.worth_moving_24() {
        // `worth_moving_24` has already established both numbers exist.
        let best = air.best_24.unwrap_or(1);
        let gain = gain_24(app, best);
        ui.label(egui::RichText::new(i18n::air_best_24(best, gain)).size(13.0).strong().color(GREEN));
        ui.label(egui::RichText::new(i18n::air_router_note()).size(11.0).color(FG_DIM));
    } else if air.current.map(|c| (1..=14).contains(&c)).unwrap_or(false) {
        ui.label(egui::RichText::new(i18n::air_quiet_here()).size(12.0).color(GREEN));
    }

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(
            crate::probe::airscan::CLEAN_24
                .iter()
                .filter_map(|c| air.load_24.iter().find(|l| l.channel == *c))
                .map(|l| i18n::air_load_cell(l.channel, l.aps, l.noise_dbm))
                .collect::<Vec<_>>()
                .join("   ·   "),
        )
        .size(11.0)
        .color(FG_DIM),
    );

    if air.on_dfs() {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(i18n::air_on_dfs(air.current.unwrap_or(0)))
                .size(13.0)
                .strong()
                .color(YELLOW),
        );
        ui.label(egui::RichText::new(i18n::air_dfs_move()).size(11.0).color(FG_DIM));
    }

    if let Some(best5) = air.best_5 {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(i18n::air_best_5(best5)).size(12.0).color(FG));
        ui.label(egui::RichText::new(i18n::air_dfs_note()).size(11.0).color(FG_DIM));
    }
}

/// How much quieter the recommended channel is than the current one, in dB.
fn gain_24(app: &App, best: u32) -> f64 {
    let noise = |ch: u32| {
        app.air.load_24.iter().find(|l| l.channel == ch).and_then(|l| l.noise_dbm)
    };
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
            ui.label(egui::RichText::new(label).size(11.0).color(FG_DIM));
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
            ui.label(
                egui::RichText::new(name)
                    .size(12.0)
                    .color(if ap.ours { GREEN } else { FG }),
            )
            // Mesh nodes share an SSID, so the only thing telling two rows
            // apart is the radio's own address.
            .on_hover_text(&ap.bssid);
            ui.label(
                egui::RichText::new(i18n::air_channel_cell(ap.channel, ap.band.label()))
                    .size(12.0)
                    .color(FG_DIM),
            );
            ui.label(
                egui::RichText::new(format!("{} dBm", ap.rssi_dbm))
                    .size(12.0)
                    .color(if ap.rssi_dbm > -70 { FG } else { FG_DIM }),
            );
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
    egui::Frame::none()
        .fill(super::BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
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

            ui.label(egui::RichText::new(t.title()).size(15.0).strong().color(FG));
            ui.add_space(4.0);

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
                ui.label(egui::RichText::new(status.label()).size(11.0).color(status.colour()));
                ui.label(egui::RichText::new(state_text).size(11.0).color(FG_DIM));
            });
            if optimize::has_snapshot(t.id()) {
                ui.label(
                    egui::RichText::new(i18n::opt_revert_available()).size(11.0).color(super::ACCENT),
                );
            }
            verdict(app, ui, t.id());
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
        });
}

/// What the ranking said about the selected tweak, if it was in the batch.
///
/// Shown as the level's own words rather than as a number: "2.4 out of 3"
/// invites the reader to treat a probability-weighted position as a
/// measurement of this machine, which it is not. The confidence sits beside
/// it because a split answer and a firm one should not read the same.
fn verdict(app: &App, ui: &mut egui::Ui, id: &str) {
    let Some(p) = app.priorities.get(id) else { return };
    ui.add_space(8.0);

    let colour = if p.is_recommendation() { GREEN } else { FG_DIM };
    ui.label(egui::RichText::new(i18n::adv_verdict_heading()).size(11.0).color(FG_DIM));
    ui.label(egui::RichText::new(&p.level).size(12.0).color(colour));
    ui.label(
        egui::RichText::new(i18n::adv_confidence(p.confidence)).size(10.0).color(FG_DIM),
    );

    if p.is_risky() {
        ui.label(egui::RichText::new(i18n::adv_breakage_warning()).size(11.0).color(YELLOW));
    }
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
