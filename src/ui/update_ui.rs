//! The update flow's two surfaces: a banner across the top when a new version
//! is waiting, and a section in settings for checking on purpose.
//!
//! All the work happens on a worker thread; this module only starts it and
//! draws whatever state came back.

use eframe::egui;

use super::{
    button, App, Emphasis, Job, ACCENT, FG, FG_DIM, GREEN, RED, S_MD, S_SM, T_BODY, T_META,
};
use crate::i18n;
use crate::update::{self, State};

/// Ask GitHub what the latest release is.
///
/// `manual` separates the check the user asked for from the one that runs at
/// start. A failed automatic check goes quiet: there is no point reporting
/// "GitHub was unreachable" to someone who only opened the app to look at
/// their latency.
pub fn start_check(app: &mut App, manual: bool) {
    if matches!(app.update, State::Checking | State::Downloading(..)) {
        return;
    }
    app.update = State::Checking;
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let state = match update::check() {
            Ok(Some(rel)) => State::Available(Box::new(rel)),
            Ok(None) => State::Current,
            Err(e) if manual => State::Failed(e.to_string()),
            Err(_) => State::Idle,
        };
        let _ = tx.send(Job::UpdateState(Box::new(state)));
    });
}

/// Download the waiting release and put it in place. The restart is a separate
/// step the user takes when they are ready for it.
pub fn start_install(app: &mut App) {
    let State::Available(rel) = &app.update else { return };
    let rel = rel.clone();
    let tx = app.tx.clone();
    app.update = State::Downloading(rel.clone(), None);

    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let result = update::install(&rel, |frac| {
            let _ = progress_tx.send(Job::UpdateProgress(frac));
        });
        let state = match result {
            Ok(()) => State::Installed(rel),
            Err(e) => State::Failed(e.to_string()),
        };
        let _ = tx.send(Job::UpdateState(Box::new(state)));
    });
}

fn restart(app: &mut App, ui: &mut egui::Ui) {
    match update::restart() {
        // Closing the window is what ends this process, and ending this
        // process is what unlocks the replaced binary for the next start.
        Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
        Err(e) => {
            let now = ui.input(|i| i.time);
            app.toast(i18n::upd_failed(&e.to_string()), RED, now);
        }
    }
}

/// Whether the banner has anything to say. Checked before the panel is built,
/// so an empty strip is never reserved at the top of the window.
pub fn banner_wanted(app: &App) -> bool {
    app.update_banner
        && matches!(app.update, State::Available(_) | State::Installed(_) | State::Failed(_))
}

pub fn banner(app: &mut App, ui: &mut egui::Ui) {
    // The banner's own vocabulary: what it says, what colour it says it in,
    // and which single action it leads with.
    enum Lead {
        Install,
        Restart,
        None,
    }
    let (text, colour, lead) = match &app.update {
        State::Available(rel) => (i18n::upd_available(&rel.version), ACCENT, Lead::Install),
        State::Installed(rel) => (i18n::upd_installed(&rel.version), GREEN, Lead::Restart),
        // An update the user asked for and did not get has to say so here. It
        // used to fall through to nothing, so clicking Update and watching the
        // banner vanish was indistinguishable from success.
        State::Failed(detail) => (i18n::upd_failed(detail), RED, Lead::None),
        _ => return,
    };

    ui.horizontal(|ui| {
        super::status_dot(ui, colour, 5.0);
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text).size(T_BODY).color(FG));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Dismiss sits furthest right and stays quiet: the banner is there
            // to be acted on, and the way to ignore it should not be the
            // loudest thing in it.
            if button(ui, i18n::upd_btn_later(), Emphasis::Ghost).clicked() {
                app.update_banner = false;
            }
            match lead {
                Lead::Install => {
                    if button(ui, i18n::upd_btn_install(), Emphasis::Primary).clicked() {
                        start_install(app);
                    }
                    if button(ui, i18n::upd_btn_page(), Emphasis::Ghost).clicked() {
                        if let State::Available(rel) = &app.update {
                            update::open_in_browser(&rel.page);
                        }
                    }
                }
                Lead::Restart => {
                    if button(ui, i18n::upd_btn_restart(), Emphasis::Primary).clicked() {
                        restart(app, ui);
                    }
                }
                // Nothing to retry in place: the release page is where a failed
                // update gets finished by hand.
                Lead::None => {
                    if button(ui, i18n::upd_btn_page(), Emphasis::Secondary).clicked() {
                        update::open_in_browser(&update::releases_page());
                    }
                }
            }
        });
    });
}

/// The settings tab's update section: what is running, and a way to check.
pub fn section(app: &mut App, ui: &mut egui::Ui) {
    ui.checkbox(&mut app.draft.check_updates, i18n::upd_auto_check());
    ui.label(egui::RichText::new(i18n::upd_auto_check_hint()).size(T_META).color(FG_DIM));
    ui.add_space(S_SM);

    ui.label(egui::RichText::new(i18n::upd_running(crate::VERSION)).size(T_BODY).color(FG_DIM));
    ui.add_space(S_SM);

    match app.update.clone() {
        State::Checking => {
            ui.label(egui::RichText::new(i18n::upd_checking()).size(T_META).color(FG_DIM));
        }
        State::Current => {
            ui.label(
                egui::RichText::new(i18n::upd_up_to_date(crate::VERSION)).size(T_META).color(GREEN),
            );
        }
        State::Failed(detail) => {
            ui.label(egui::RichText::new(i18n::upd_failed(&detail)).size(T_META).color(RED));
        }
        State::Downloading(rel, frac) => {
            ui.label(egui::RichText::new(i18n::upd_available(&rel.version)).size(T_BODY).color(FG));
            ui.label(egui::RichText::new(i18n::upd_downloading()).size(T_META).color(FG_DIM));
            ui.add_space(4.0);
            // An unknown total still gets a bar, animated rather than filled:
            // "working, length unknown" reads better than a bar frozen at zero.
            let bar = match frac {
                Some(f) => egui::ProgressBar::new(f).show_percentage(),
                None => egui::ProgressBar::new(0.0).animate(true),
            };
            ui.add(bar.desired_width(f32::INFINITY));
        }
        State::Available(rel) => {
            ui.label(
                egui::RichText::new(i18n::upd_available(&rel.version)).size(T_BODY).color(ACCENT),
            );
            if !rel.notes.is_empty() {
                ui.add_space(S_SM);
                ui.label(
                    egui::RichText::new(i18n::upd_whats_new()).size(T_META).strong().color(FG),
                );
                // Scrolled rather than truncated: release notes are as long as
                // they are, and a section that grows without limit pushes the
                // buttons under it off the screen.
                egui::ScrollArea::vertical().max_height(140.0).id_salt("upd_notes").show(
                    ui,
                    |ui| {
                        ui.label(egui::RichText::new(&rel.notes).size(T_META).color(FG_DIM));
                    },
                );
            }
        }
        State::Installed(rel) => {
            ui.label(
                egui::RichText::new(i18n::upd_installed(&rel.version)).size(T_BODY).color(GREEN),
            );
            ui.label(egui::RichText::new(i18n::upd_restart_hint()).size(T_META).color(FG_DIM));
        }
        State::Idle => {}
    }

    ui.add_space(S_MD);
    ui.horizontal(|ui| {
        let busy = matches!(app.update, State::Checking | State::Downloading(..));
        if super::button_ex(ui, i18n::upd_btn_check(), Emphasis::Secondary, !busy, 0.0).clicked() {
            start_check(app, true);
        }
        // The one action the section leads with, if there is one: install what
        // was found, or restart into what was installed.
        let lead = match &app.update {
            State::Available(_) => Some((i18n::upd_btn_install(), true)),
            State::Installed(_) => Some((i18n::upd_btn_restart(), false)),
            _ => None,
        };
        if let Some((label, installs)) = lead {
            if button(ui, label, Emphasis::Primary).clicked() {
                if installs {
                    start_install(app);
                } else {
                    restart(app, ui);
                }
            }
        }
        if button(ui, i18n::upd_btn_page(), Emphasis::Ghost).clicked() {
            let url = match &app.update {
                State::Available(rel) => rel.page.clone(),
                _ => update::releases_page(),
            };
            update::open_in_browser(&url);
        }
    });
}
