//! Settings: probing cadence, thresholds, extra targets, autostart.
//!
//! Split into pages rather than one long column. The column held six
//! sections, the save button sat under the last of them, and reaching it meant
//! scrolling past everything you had not touched. Now the page list sits on
//! the left, the page on the right, and the save bar is pinned to the bottom
//! where it stays in reach from every page.

use eframe::egui;

use super::{
    button, button_ex, App, Emphasis, ACCENT, BG, BG2, FG, FG_DIM, GREEN, RED, S_LG, S_MD, S_SM,
    S_XS, T_BODY, T_HEAD, T_META, YELLOW,
};
use crate::i18n;
use crate::settings::Settings;

/// The settings pages, in the order the list shows them.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Measure,
    Targets,
    Overlay,
    General,
    Ai,
    Updates,
}

impl Page {
    const ALL: [Page; 6] =
        [Page::Measure, Page::Targets, Page::Overlay, Page::General, Page::Ai, Page::Updates];

    fn label(self) -> &'static str {
        match self {
            Page::Measure => i18n::set_page_measure(),
            Page::Targets => i18n::set_page_targets(),
            Page::Overlay => i18n::set_page_overlay(),
            Page::General => i18n::set_page_general(),
            Page::Ai => i18n::set_page_ai(),
            Page::Updates => i18n::upd_section(),
        }
    }
}

/// Width of the page list, and the most a page's content is allowed to span.
/// A label on the left and its value 1400 px away on the right is a row the
/// eye cannot follow.
const NAV_W: f32 = 190.0;
const CONTENT_MAX_W: f32 = 680.0;
/// The label column in a form row.
const LABEL_W: f32 = 280.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let page_id = egui::Id::new("settings_page");
    let mut page: Page = ui.data(|d| d.get_temp(page_id)).unwrap_or_default();

    // The bar goes in first so the pages get what is left above it.
    egui::TopBottomPanel::bottom("settings_actions")
        .frame(
            egui::Frame::none()
                .fill(BG)
                .inner_margin(egui::Margin { top: S_MD, ..Default::default() }),
        )
        .show_inside(ui, |ui| action_bar(app, ui));

    if super::is_narrow(ui) {
        ui.horizontal_wrapped(|ui| {
            for p in Page::ALL {
                let selected = page == p;
                let text = egui::RichText::new(nav_text(app, p)).size(T_BODY).color(if selected {
                    FG
                } else {
                    FG_DIM
                });
                if ui.selectable_label(selected, text).clicked() {
                    page = p;
                }
            }
        });
        ui.add_space(S_MD);
        page_body(app, ui, page);
    } else {
        egui::SidePanel::left("settings_nav")
            .resizable(false)
            .exact_width(NAV_W)
            .show_separator_line(false)
            .frame(
                egui::Frame::none()
                    .inner_margin(egui::Margin { right: S_LG, ..Default::default() }),
            )
            .show_inside(ui, |ui| {
                for p in Page::ALL {
                    if nav_item(app, ui, p, page == p).clicked() {
                        page = p;
                    }
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show_inside(ui, |ui| page_body(app, ui, page));
    }

    ui.data_mut(|d| d.insert_temp(page_id, page));
}

/// A page's name in the narrow strip, where there is no room for a dot.
fn nav_text(app: &App, page: Page) -> String {
    if page_marker(app, page).is_some() {
        format!("{} •", page.label())
    } else {
        page.label().to_string()
    }
}

/// A dot beside a page that needs looking at: yellow for changes not saved
/// yet, the accent for an update waiting.
fn page_marker(app: &App, page: Page) -> Option<egui::Color32> {
    if page_dirty(app, page) {
        return Some(YELLOW);
    }
    let update_waiting = matches!(
        app.update,
        crate::update::State::Available(_) | crate::update::State::Installed(_)
    );
    (page == Page::Updates && update_waiting).then_some(ACCENT)
}

/// One entry in the page list. Drawn the way the main tabs mark theirs: the
/// selected one in full colour with an accent bar, the rest dimmed.
fn nav_item(app: &App, ui: &mut egui::Ui, page: Page, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected || response.hovered() {
            painter.rect_filled(rect, super::BTN_R, BG2);
        }
        if selected {
            let bar = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(0.0, 7.0),
                egui::vec2(3.0, rect.height() - 14.0),
            );
            painter.rect_filled(bar, 1.5, ACCENT);
        }
        let colour = if selected || response.hovered() { FG } else { FG_DIM };
        let font = egui::FontId::new(T_BODY, egui::FontFamily::Proportional);
        painter.text(
            rect.left_center() + egui::vec2(S_MD, 0.0),
            egui::Align2::LEFT_CENTER,
            page.label(),
            font,
            colour,
        );
        if let Some(dot) = page_marker(app, page) {
            painter.circle_filled(rect.right_center() - egui::vec2(S_MD, 0.0), 4.0, dot);
        }
    }

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn page_body(app: &mut App, ui: &mut egui::Ui, page: Page) {
    // One scroll position per page: shared, a page opened after a long one
    // starts scrolled past its own top.
    let scroll = egui::ScrollArea::vertical().id_salt(("settings_scroll", page as u8));
    scroll.auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(CONTENT_MAX_W);

        // Silently resetting to defaults looks like the app forgot the
        // settings; saying the file was damaged points at the real cause and
        // at the copy that was kept.
        if let Some(detail) = crate::settings::load_issue() {
            ui.label(egui::RichText::new(i18n::set_load_failed(&detail)).size(T_META).color(RED));
            ui.add_space(S_SM);
        }

        match page {
            Page::Measure => measure_page(app, ui),
            Page::Targets => targets_page(app, ui),
            Page::Overlay => overlay_page(app, ui),
            Page::General => general_page(app, ui),
            Page::Ai => ai_page(app, ui),
            Page::Updates => {
                section(ui, i18n::upd_section(), |ui| super::update_ui::section(app, ui))
            }
        }
    });
}

fn measure_page(app: &mut App, ui: &mut egui::Ui) {
    let d = &mut app.draft;
    section(ui, i18n::set_sec_probing(), |ui| {
        row(ui, i18n::set_interval(), |ui| {
            ui.add(
                egui::DragValue::new(&mut d.probe_interval_ms)
                    .range(300..=crate::settings::MAX_PROBE_INTERVAL_MS)
                    .speed(50)
                    .suffix(" ms"),
            );
        });
        hint(ui, i18n::set_interval_hint());
        row(ui, i18n::set_ping_timeout(), |ui| {
            ui.add(
                egui::DragValue::new(&mut d.ping_timeout_ms)
                    .range(100..=10_000)
                    .speed(10)
                    .suffix(" ms"),
            );
        });
        gap(ui);
        row(ui, i18n::set_fails_before_alarm(), |ui| {
            ui.add(egui::DragValue::new(&mut d.outage_after_fails).range(1..=100).speed(0.1));
        });
        hint(ui, i18n::set_fails_hint());
        row(ui, i18n::set_keep_days(), |ui| {
            let unit = format!(" {}", i18n::set_unit_days());
            ui.add(egui::DragValue::new(&mut d.keep_days).range(1..=3650).speed(0.2).suffix(unit));
        });
    });

    section(ui, i18n::set_sec_thresholds(), |ui| {
        ms_row(ui, i18n::set_lat_good(), &mut d.ping_ok_ms);
        gap(ui);
        ms_row(ui, i18n::set_lat_bad(), &mut d.ping_bad_ms);
        gap(ui);
        ms_row(ui, i18n::set_jitter_good(), &mut d.jitter_good_ms);
        gap(ui);
        ms_row(ui, i18n::set_jitter_ok(), &mut d.jitter_ok_ms);
        gap(ui);
        row(ui, i18n::set_loss_ok(), |ui| {
            ui.add(
                egui::DragValue::new(&mut d.loss_ok_pct).range(0.0..=100.0).speed(0.1).suffix(" %"),
            );
        });
    });
}

fn targets_page(app: &mut App, ui: &mut egui::Ui) {
    section(ui, i18n::set_sec_targets(), |ui| {
        hint(ui, i18n::set_targets_hint());
        ui.add(
            egui::TextEdit::multiline(&mut app.draft_targets)
                .desired_rows(8)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace)
                .hint_text("192.168.1.10\nexample.com"),
        );
    });
}

fn overlay_page(app: &mut App, ui: &mut egui::Ui) {
    instant_note(ui);
    section(ui, i18n::set_page_overlay(), |ui| {
        let d = &mut app.draft;
        ui.checkbox(
            &mut d.game_overlay,
            egui::RichText::new(i18n::set_game_overlay()).size(T_BODY),
        );
        ui.add_space(S_MD);
        ui.add_enabled_ui(d.game_overlay, |ui| {
            row(ui, i18n::set_overlay_corner(), |ui| {
                egui::ComboBox::from_id_salt("overlay_corner")
                    .selected_text(d.game_overlay_corner.label())
                    .show_ui(ui, |ui| {
                        for c in crate::settings::Corner::ALL {
                            ui.selectable_value(&mut d.game_overlay_corner, c, c.label());
                        }
                    });
            });
            gap(ui);
            row(ui, i18n::set_overlay_content(), |ui| {
                egui::ComboBox::from_id_salt("overlay_content")
                    .selected_text(d.overlay_content.label())
                    .show_ui(ui, |ui| {
                        for c in crate::settings::OverlayContent::ALL {
                            ui.selectable_value(&mut d.overlay_content, c, c.label());
                        }
                    });
            });
            gap(ui);
            row(ui, i18n::set_overlay_size(), |ui| {
                for s in crate::settings::OverlaySize::ALL {
                    ui.selectable_value(&mut d.overlay_size, s, s.label());
                }
            });
            gap(ui);
            row(ui, i18n::set_overlay_opacity(), |ui| {
                ui.add(
                    egui::Slider::new(
                        &mut d.overlay_opacity,
                        crate::settings::OVERLAY_OPACITY_MIN..=100,
                    )
                    .suffix(" %"),
                );
            });
        });
    });

    // Applied on click, like the language: the overlay is watched while
    // it is set, and a Save between a click and its effect hides which
    // click did what. Written to disk once the mouse is up, so dragging
    // the slider does not write the file every frame.
    apply_overlay(app, ui);
}

/// Carries the overlay's fields from the draft into effect. Called from the
/// overlay page and after "Restore defaults", the two places they change.
fn apply_overlay(app: &mut App, ui: &mut egui::Ui) {
    if app.settings.take_overlay(&app.draft) {
        app.monitor.update_settings(app.settings.clone());
        app.overlay_unsaved = true;
    }
    if app.overlay_unsaved && !ui.input(|i| i.pointer.any_down()) {
        app.overlay_unsaved = false;
        if let Err(e) = app.settings.save() {
            let now = ui.input(|i| i.time);
            app.toast(i18n::set_save_failed(&e.to_string()), RED, now);
        }
    }
}

fn general_page(app: &mut App, ui: &mut egui::Ui) {
    section(ui, i18n::set_language(), |ui| {
        // Applied on click rather than on save: a language picker that needs
        // a second confirmation is hard to undo once the labels are in a
        // language you cannot read.
        let mut chosen = app.draft.effective_lang();
        ui.horizontal(|ui| {
            for lang in i18n::Lang::ALL {
                let text = egui::RichText::new(lang.native_name()).size(T_BODY);
                if ui.selectable_label(chosen == lang, text).clicked() {
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
        ui.add_space(S_XS);
        hint(ui, i18n::set_language_hint());
    });

    section(ui, i18n::set_sec_behaviour(), |ui| {
        ui.checkbox(
            &mut app.draft.notify_on_outage,
            egui::RichText::new(i18n::set_notify()).size(T_BODY),
        );
        gap(ui);
        ui.checkbox(
            &mut app.draft.start_minimised,
            egui::RichText::new(i18n::set_start_min()).size(T_BODY),
        );
        gap(ui);

        let mut autostart = app.autostart_on;
        let label = egui::RichText::new(i18n::set_autostart()).size(T_BODY);
        if ui.checkbox(&mut autostart, label).changed() {
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
}

fn ai_page(app: &mut App, ui: &mut egui::Ui) {
    section(ui, i18n::set_sec_ai(), |ui| {
        hint(ui, i18n::set_ai_hint());
        ui.add_space(S_XS);
        row(ui, i18n::set_ai_key(), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.draft.ai_key)
                    .password(true)
                    .desired_width(ui.available_width())
                    .hint_text("sk-or-…"),
            );
        });
        gap(ui);
        row(ui, i18n::set_ai_model(), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.draft.ai_model)
                    .desired_width(ui.available_width())
                    .hint_text(crate::ai::DEFAULT_MODEL),
            );
        });
        hint(ui, &i18n::set_ai_model_hint(crate::ai::DEFAULT_MODEL));
    });
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

fn parsed_targets(text: &str) -> Vec<String> {
    text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

/// Whether the draft differs from what is in effect. The targets are held as
/// text while edited, so they are compared as the list they will be saved as.
pub fn is_dirty(app: &App) -> bool {
    app.draft != app.settings || parsed_targets(&app.draft_targets) != app.settings.extra_targets
}

/// Whether a page holds a change not saved yet.
fn page_dirty(app: &App, page: Page) -> bool {
    let (d, s) = (&app.draft, &app.settings);
    match page {
        Page::Measure => {
            d.probe_interval_ms != s.probe_interval_ms
                || d.ping_timeout_ms != s.ping_timeout_ms
                || d.outage_after_fails != s.outage_after_fails
                || d.keep_days != s.keep_days
                || d.ping_ok_ms != s.ping_ok_ms
                || d.ping_bad_ms != s.ping_bad_ms
                || d.jitter_good_ms != s.jitter_good_ms
                || d.jitter_ok_ms != s.jitter_ok_ms
                || d.loss_ok_pct != s.loss_ok_pct
        }
        Page::Targets => parsed_targets(&app.draft_targets) != s.extra_targets,
        // Applied as it is set, so there is never anything waiting here.
        Page::Overlay => false,
        Page::General => {
            d.notify_on_outage != s.notify_on_outage || d.start_minimised != s.start_minimised
        }
        Page::Ai => d.ai_key != s.ai_key || d.ai_model != s.ai_model,
        Page::Updates => d.check_updates != s.check_updates,
    }
}

/// The strip pinned under every page: whether anything is waiting to be
/// saved, and the three things you can do about it.
fn action_bar(app: &mut App, ui: &mut egui::Ui) {
    let dirty = is_dirty(app);

    // Ctrl+S, because a form with a save button gets that keystroke tried on
    // it whether or not it answers.
    let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
    let save_key = ui.input_mut(|i| i.consume_shortcut(&shortcut));

    ui.horizontal(|ui| {
        let (dot, text, colour) = if dirty {
            (YELLOW, i18n::set_unsaved(), FG)
        } else {
            (GREEN, i18n::set_all_saved(), FG_DIM)
        };
        super::status_dot(ui, dot, 4.0);
        ui.add_space(S_XS);
        ui.label(egui::RichText::new(text).size(T_BODY).color(colour));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save = button_ex(ui, i18n::set_btn_save(), Emphasis::Primary, dirty, 0.0)
                .on_hover_text("Ctrl+S");
            if save.clicked() || (save_key && dirty) {
                save_draft(app, ui);
            }
            if button_ex(ui, i18n::set_btn_discard(), Emphasis::Secondary, dirty, 0.0).clicked() {
                app.draft = app.settings.clone();
                app.draft_targets = app.settings.extra_targets.join("\n");
            }
            if button(ui, i18n::set_btn_defaults(), Emphasis::Ghost).clicked() {
                restore_defaults(app, ui);
            }
        });
    });
}

/// Fills the draft with defaults and leaves saving to the user, like any
/// other edit. It used to save at once, so one stray click wiped every
/// threshold with no way back.
///
/// The language and the AI key are kept. The language is not a measurement
/// setting and flipping it back to the Windows default mid-click leaves the
/// user on a screen they may not read; the key is something they pasted in
/// from elsewhere and would have to go and fetch again.
fn restore_defaults(app: &mut App, ui: &mut egui::Ui) {
    app.draft = Settings {
        lang: app.draft.lang,
        ai_key: std::mem::take(&mut app.draft.ai_key),
        ..Settings::default()
    };
    app.draft_targets.clear();
    // The overlay is applied as it is set, and defaults are no exception.
    apply_overlay(app, ui);
    let now = ui.input(|i| i.time);
    app.toast(i18n::set_defaults_loaded().to_string(), ACCENT, now);
}

fn save_draft(app: &mut App, ui: &mut egui::Ui) {
    let now = ui.input(|i| i.time);

    if let Some(why) = app.draft.refusal() {
        app.toast(why, RED, now);
        return;
    }

    app.draft.extra_targets = parsed_targets(&app.draft_targets);

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

// ---------------------------------------------------------------------------
// Layout pieces
// ---------------------------------------------------------------------------

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none().fill(BG2).rounding(6.0).inner_margin(egui::Margin::same(S_LG)).show(
        ui,
        |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(egui::RichText::new(title).size(T_HEAD).strong().color(FG));
            ui.add_space(S_MD);
            body(ui);
        },
    );
    ui.add_space(S_MD);
}

/// A line above a page whose changes need no save, so the save bar under it
/// is not mistaken for part of the deal.
fn instant_note(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        super::status_dot(ui, ACCENT, 4.0);
        ui.add_space(S_XS);
        ui.label(egui::RichText::new(i18n::set_instant_hint()).size(T_META).color(FG_DIM));
    });
    ui.add_space(S_SM);
}

/// The line under a control that says what it is for. The gap below it is
/// what ties it to the control above rather than the one below.
fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(T_META).color(FG_DIM));
    ui.add_space(S_MD);
}

/// Space between two rows that have no hint between them.
fn gap(ui: &mut egui::Ui) {
    ui.add_space(S_SM);
}

/// A form row: the label in a fixed column, the control after it, so every
/// control on a page starts on the same vertical line.
fn row(ui: &mut egui::Ui, label: &str, widget: impl FnOnce(&mut egui::Ui)) {
    let label_w = LABEL_W.min(ui.available_width() * 0.5);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(label_w, super::BTN_H),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(label_w);
                ui.add(egui::Label::new(egui::RichText::new(label).size(T_BODY).color(FG)).wrap());
            },
        );
        widget(ui);
    });
}

fn ms_row(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    row(ui, label, |ui| {
        ui.add(egui::DragValue::new(value).range(0.0..=10_000.0).speed(1.0).suffix(" ms"));
    });
}
