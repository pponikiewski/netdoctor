//! Optimise tab: inspect, apply and revert each tweak.

use eframe::egui;

use super::{
    button, button_ex, card, figure, App, Emphasis, FG, FG_DIM, GREEN, RED, S_MD, S_SM, S_XS,
    T_BODY, T_HEAD, T_META, T_TITLE, YELLOW,
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

/// The widest a card is allowed to get before the grid adds a column, and
/// the gap between cards. Cards much wider than this turn a two-line title
/// into one long line and leave the toggle far from what it switches.
const CARD_MAX_W: f32 = 420.0;
const CARD_GAP: f32 = S_MD;
/// Space above and below a card's content, and the width kept for its switch.
const CARD_PAD_Y: f32 = S_SM + 2.0;
const SWITCH_W: f32 = 40.0;
/// A card under the pointer, one step lighter: the cue that it opens.
const CARD_HOVER: egui::Color32 = egui::Color32::from_rgb(0x21, 0x25, 0x2d);

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let tweaks = optimize::all();
    header(app, ui);

    egui::ScrollArea::vertical().id_salt("optimise_grid").auto_shrink([false, false]).show(
        ui,
        |ui| {
            air_card(app, ui);
            for category in optimize::Category::ALL {
                group(app, ui, &tweaks, category);
            }
        },
    );
}

/// How far along the machine is, the states that limit what can be done, and
/// the two actions that apply to the whole list.
fn header(app: &mut App, ui: &mut egui::Ui) {
    let (set, todo, na) = tally(app);
    let (dot, text) = if todo == 0 {
        (GREEN, i18n::opt_headline_done().to_string())
    } else {
        (YELLOW, format!("{}: {todo}", i18n::st_todo_heading()))
    };
    ui.horizontal_wrapped(|ui| {
        super::status_dot(ui, dot, 5.0);
        ui.add_space(S_XS);
        ui.label(egui::RichText::new(text).size(T_TITLE).strong().color(FG));
    });
    ui.add_space(S_XS);
    ui.label(egui::RichText::new(i18n::opt_blurb()).size(T_META).color(FG_DIM));

    // A damaged snapshot file used to just make every Revert button vanish,
    // which reads as "nothing was ever applied" rather than as a fault.
    if let Some(err) = optimize::snapshots_error() {
        ui.add_space(S_XS);
        ui.label(egui::RichText::new(i18n::tw_snapshots_unreadable_hint()).size(T_META).color(RED));
        ui.label(egui::RichText::new(err).size(T_META).color(FG_DIM));
    }

    // Reading the states takes a third of a second of `netsh` and
    // `powercfg`, so it runs on a worker thread. While it does, the rows on
    // screen are whatever the last pass found, and the buttons that act on
    // that reading are held back rather than acting on a stale one.
    let loading = app.tweaks_loading();
    if loading || !app.elevated {
        ui.add_space(S_XS);
        ui.horizontal_wrapped(|ui| {
            if loading {
                ui.label(
                    egui::RichText::new(i18n::opt_reading()).size(T_META).color(super::ACCENT),
                );
            }
            if !app.elevated {
                ui.label(egui::RichText::new(i18n::opt_read_only()).size(T_META).color(YELLOW));
            }
        });
    }
    ui.add_space(S_MD);

    ui.horizontal(|ui| {
        let apply_all = button_ex(
            ui,
            i18n::opt_btn_apply_all(),
            Emphasis::Primary,
            app.elevated && !loading && todo > 0,
            0.0,
        );
        let apply_all = if !app.elevated {
            apply_all.on_hover_text(i18n::opt_needs_admin())
        } else if loading {
            apply_all.on_hover_text(i18n::opt_reading())
        } else {
            apply_all
        };
        if apply_all.clicked() {
            apply_all_safe(app, ui);
        }
        if button_ex(ui, i18n::btn_refresh(), Emphasis::Secondary, !loading, 0.0).clicked() {
            app.refresh_tweaks();
        }

        // The tally answers "is there anything left to do here" without
        // reading a single row.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(i18n::opt_summary(set, todo, na)).size(T_META).color(FG_DIM),
            );
            if na > 0 {
                ui.checkbox(
                    &mut app.show_unavailable,
                    egui::RichText::new(i18n::opt_show_unavailable()).size(T_META),
                )
                .on_hover_text(i18n::opt_show_unavailable_hint());
            }
        });
    });
    ui.add_space(S_MD);
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

fn risk_colour(r: Risk) -> egui::Color32 {
    match r {
        Risk::Low => GREEN,
        Risk::Medium => YELLOW,
        Risk::High => RED,
    }
}

/// A category's name and how far along it is, over its cards. The blurb is
/// on hover: over every group it was more text than the cards held.
fn group_heading(ui: &mut egui::Ui, title: &str, count: &str, colour: egui::Color32, tip: &str) {
    ui.add_space(S_MD);
    let r = ui
        .horizontal(|ui| {
            ui.label(egui::RichText::new(title).size(T_HEAD).strong().color(FG));
            ui.add_space(S_SM);
            ui.label(egui::RichText::new(count).size(T_META).color(colour));
        })
        .response;
    if !tip.is_empty() {
        r.on_hover_ui(|ui| super::tip_prose(ui, tip));
    }
    ui.add_space(S_SM);
}

/// One category's cards, in rows of as many as fit.
fn group(
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

    let status_at =
        |app: &App, i: usize| Status::of(app.tweak_states.get(i).and_then(|(_, _, o)| *o));
    // The counts describe the whole category, whether or not every card of
    // it is on screen, so hiding cards never changes what the heading claims.
    let set = all_rows.iter().filter(|i| status_at(app, **i) == Status::Set).count();
    let available = all_rows.iter().filter(|i| status_at(app, **i) != Status::Unavailable).count();

    let mut cards: Vec<usize> = all_rows
        .into_iter()
        .filter(|i| app.show_unavailable || status_at(app, *i) != Status::Unavailable)
        .collect();
    if cards.is_empty() {
        return;
    }
    // What is left to do first, what is set after it, what cannot be touched
    // here last; declaration order breaks ties so nothing moves between frames.
    cards.sort_by_key(|i| {
        let rank = match status_at(app, *i) {
            Status::Todo => 0,
            Status::Set => 1,
            Status::Unavailable => 2,
        };
        (rank, *i)
    });

    let (count, colour) = if available == 0 {
        (i18n::opt_section_none().to_string(), FG_DIM)
    } else if set == available {
        (i18n::opt_section_all_set().to_string(), GREEN)
    } else {
        (i18n::opt_section_count(set, available), FG_DIM)
    };
    group_heading(ui, category.title(), &count, colour, category.blurb());

    let width = ui.available_width();
    let columns = ((width + CARD_GAP) / (CARD_MAX_W + CARD_GAP)).ceil().max(1.0) as usize;
    let columns = if super::is_narrow(ui) { columns.min(2) } else { columns };
    for (row_no, row) in cards.chunks(columns).enumerate() {
        // Cards in a row share a height. egui lays each out to its own
        // content, so each card reports how tall it wanted to be, and the
        // row is held to the tallest of those on the next frame.
        let row_id = egui::Id::new(("optimise_row", category.title(), row_no));
        let row_h: f32 = ui.data(|d| d.get_temp(row_id)).unwrap_or(0.0);
        let mut wanted = 0.0_f32;
        ui.columns(columns, |cols| {
            for (col, i) in row.iter().enumerate() {
                let natural = tweak_card(app, &mut cols[col], tweaks[*i].as_ref(), *i, row_h);
                wanted = wanted.max(natural);
            }
        });
        ui.data_mut(|d| d.insert_temp(row_id, wanted));
        ui.add_space(CARD_GAP);
    }
}

/// Pads a card's content down to `height` measured from `top`.
///
/// `Ui::set_min_height` counts from where the cursor is, not from the top of
/// the ui: called after the content it added the whole row height again
/// underneath it, which is where the empty half of every card came from.
/// Zero on a row's first frame, before any card has reported its height,
/// and egui asserts on a negative minimum, so nothing is added then.
fn level_to(ui: &mut egui::Ui, top: f32, height: f32) {
    // The cursor already sits one item gap below the content.
    let used = ui.cursor().top() - top;
    if height > used {
        ui.add_space(height - used);
    }
}

/// What a card's switch can do right now, and why not when it cannot.
enum Switch {
    /// Off, and switching it on applies the change.
    CanApply,
    /// On, and switching it off puts back what NetDoctor saved.
    CanRevert,
    /// Shown in its state but not clickable, with the reason on hover.
    Locked(&'static str),
    /// A one-off repair rather than a setting: a button, not a switch.
    Action,
}

fn switch_for(app: &App, t: &dyn optimize::Tweak, status: Status) -> Switch {
    if !t.reversible() {
        return Switch::Action;
    }
    if !app.elevated && t.needs_admin() {
        return Switch::Locked(i18n::opt_needs_admin());
    }
    match status {
        Status::Unavailable => Switch::Locked(i18n::st_na()),
        Status::Todo => Switch::CanApply,
        Status::Set if optimize::has_snapshot(t, &app.net) => Switch::CanRevert,
        // Already right, but not by this app's hand: there is no "before"
        // to go back to, so switching it off would be a guess.
        Status::Set => Switch::Locked(i18n::opt_nothing_to_revert()),
    }
}

fn open_id(t: &dyn optimize::Tweak) -> egui::Id {
    egui::Id::new(("optimise_open", t.id()))
}

fn confirm_id(t: &dyn optimize::Tweak) -> egui::Id {
    egui::Id::new(("optimise_confirm", t.id()))
}

/// One change as a card: its name and switch, what it is set to now, the
/// risk, and the explanation folded underneath. Returns the height the card
/// wanted before the row's shared height was applied.
fn tweak_card(
    app: &mut App,
    ui: &mut egui::Ui,
    t: &dyn optimize::Tweak,
    i: usize,
    row_h: f32,
) -> f32 {
    let (value, optimal) =
        app.tweak_states.get(i).map(|(_, v, o)| (v.clone(), *o)).unwrap_or_default();
    let status = Status::of(optimal);
    let loading = app.tweaks_loading();
    let mut natural = 0.0;

    // The history tab's "Open the fix" lands here with this change selected:
    // open its explanation, and scroll to it once the card is laid out.
    let asked_for = app.selected_tweak == Some(i);
    if asked_for {
        ui.data_mut(|d| d.insert_temp(open_id(t), true));
        app.selected_tweak = None;
    }

    // The whole card is the way into its explanation. It is sensed over
    // where the card was last frame, and before its contents, so the switch
    // and the buttons inside, registered after it, sit on top and keep their
    // own clicks.
    let rect_id = egui::Id::new(("optimise_card_rect", t.id()));
    let is_open = ui.data(|d| d.get_temp::<bool>(open_id(t))).unwrap_or(false);
    let card = ui.data(|d| d.get_temp::<egui::Rect>(rect_id)).map(|r| {
        ui.interact(r, rect_id.with("click"), egui::Sense::click()).on_hover_text(if is_open {
            i18n::opt_less()
        } else {
            i18n::opt_more()
        })
    });
    // Read off the pointer rather than `hovered()`: the switch sits on top
    // of the card, and the card is still the thing under it.
    let hovered = card.as_ref().is_some_and(|c| ui.rect_contains_pointer(c.rect));
    if card.as_ref().is_some_and(|c| c.clicked()) {
        ui.data_mut(|d| d.insert_temp(open_id(t), !is_open));
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let frame = egui::Frame::none()
        .fill(if hovered { CARD_HOVER } else { super::BG2 })
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(S_MD, CARD_PAD_Y))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // Text that can be selected takes the click for itself, and a
            // click on a card's text is the one most people make.
            ui.style_mut().interaction.selectable_labels = false;
            let top = ui.min_rect().top();

            // The switch sits in the card's top-right corner, in a child of
            // its own that takes no room in the flow. Laid out beside the
            // title in a right-to-left layout it took the whole remaining
            // height of the column, and every card came out as tall as the
            // tallest thing egui could imagine next to it.
            let switch_rect = egui::Rect::from_min_size(
                egui::pos2(ui.max_rect().right() - SWITCH_W, top),
                egui::vec2(SWITCH_W, 22.0),
            );
            let mut corner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(switch_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Min)),
            );
            card_switch(app, &mut corner, t, status, loading);

            ui.scope(|ui| {
                ui.set_max_width((ui.available_width() - SWITCH_W - S_SM).max(80.0));
                ui.horizontal_top(|ui| {
                    ui.add_space(1.0);
                    super::status_dot(ui, status.colour(), 4.0);
                    ui.add_space(S_XS);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(t.title())
                                .size(T_BODY)
                                .strong()
                                .color(if status == Status::Unavailable { FG_DIM } else { FG }),
                        )
                        .wrap(),
                    );
                });
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = S_XS;
                ui.label(egui::RichText::new(status.label()).size(T_META).color(status.colour()));
                if status != Status::Unavailable {
                    ui.label(egui::RichText::new("·").size(T_META).color(FG_DIM));
                    ui.label(
                        egui::RichText::new(i18n::opt_risk_note(t.risk().label()))
                            .size(T_META)
                            .color(risk_colour(t.risk())),
                    );
                }
                if t.needs_reboot() {
                    ui.label(egui::RichText::new("·").size(T_META).color(FG_DIM));
                    ui.label(
                        egui::RichText::new(i18n::opt_needs_reboot()).size(T_META).color(FG_DIM),
                    );
                }
            });
            let open = ui.data(|d| d.get_temp::<bool>(open_id(t))).unwrap_or(false);
            let confirm = ui.data(|d| d.get_temp::<bool>(confirm_id(t))).unwrap_or(false);
            ui.add(
                egui::Label::new(egui::RichText::new(value).size(T_META).color(FG_DIM)).truncate(),
            );
            // Measured before the explanation: the row is levelled to its
            // folded cards, and an opened one grows on its own. Measured
            // after, opening one card stretched its neighbours into empty
            // panels of the same height.
            natural = ui.min_rect().bottom() - top;
            if open || confirm {
                card_detail(app, ui, t, confirm);
            }

            level_to(ui, top, row_h - CARD_PAD_Y * 2.0);
        });
    ui.data_mut(|d| d.insert_temp(rect_id, frame.response.rect));
    if asked_for {
        frame.response.scroll_to_me(Some(egui::Align::TOP));
    }
    natural + CARD_PAD_Y * 2.0
}

/// The switch, or the action button for a one-off repair.
fn card_switch(
    app: &mut App,
    ui: &mut egui::Ui,
    t: &dyn optimize::Tweak,
    status: Status,
    loading: bool,
) {
    let on = status == Status::Set;
    match switch_for(app, t, status) {
        Switch::Action => {
            let enabled = (app.elevated || !t.needs_admin()) && !loading;
            let b = button_ex(ui, i18n::btn_apply(), Emphasis::Danger, enabled, 0.0);
            let b = if enabled { b } else { b.on_hover_text(i18n::opt_needs_admin()) };
            if b.clicked() {
                // Cannot be undone, so it never fires from the card alone.
                ui.data_mut(|d| {
                    d.insert_temp(confirm_id(t), true);
                    d.insert_temp(open_id(t), true);
                });
            }
        }
        Switch::Locked(why) => {
            toggle(ui, on, false).on_hover_text(why);
        }
        Switch::CanApply | Switch::CanRevert if loading => {
            toggle(ui, on, false).on_hover_text(i18n::opt_reading());
        }
        Switch::CanApply => {
            if toggle(ui, on, true).clicked() {
                if t.risk() == Risk::High {
                    // A switch is lighter to flick than the change is to
                    // live with, so a high-risk one asks first, with the
                    // explanation open above the question.
                    ui.data_mut(|d| {
                        d.insert_temp(confirm_id(t), true);
                        d.insert_temp(open_id(t), true);
                    });
                } else {
                    apply_one(app, ui, t);
                }
            }
        }
        Switch::CanRevert => {
            if toggle(ui, on, true).clicked() {
                revert_one(app, ui, t);
            }
        }
    }
}

/// What the change does and why, the notes, and what it did last time.
fn card_detail(app: &mut App, ui: &mut egui::Ui, t: &dyn optimize::Tweak, confirm: bool) {
    ui.add_space(S_XS);
    ui.separator();
    ui.add_space(S_XS);
    for (heading, text) in [(i18n::opt_card_what(), t.what()), (i18n::opt_card_why(), t.why())] {
        ui.label(egui::RichText::new(heading).size(T_META).strong().color(FG_DIM));
        ui.label(egui::RichText::new(text).size(T_META).color(FG));
        ui.add_space(S_XS);
    }
    if !t.reversible() {
        ui.label(egui::RichText::new(i18n::opt_irreversible()).size(T_META).color(RED));
    }
    if optimize::has_snapshot(t, &app.net) {
        ui.label(
            egui::RichText::new(i18n::opt_revert_available()).size(T_META).color(super::ACCENT),
        );
    }

    // What the line did in the day before and after this app last applied
    // it. Only there once it has been applied and measured on both sides.
    if let Some(e) = app.tweak_effects.get(t.id()) {
        ui.add_space(S_XS);
        let heading = i18n::opt_effect_heading(&crate::diagnose::format_datetime(e.applied));
        ui.label(egui::RichText::new(heading).size(T_META).strong().color(FG_DIM));
        for (label, side) in
            [(i18n::opt_effect_before(), e.before), (i18n::opt_effect_after(), e.after)]
        {
            let text = match side {
                Some(s) => i18n::opt_effect_side(
                    &label,
                    &i18n::span(s.span_s),
                    s.median_ms,
                    s.jitter_ms,
                    s.loss_pct,
                ),
                None => i18n::opt_effect_no_data(&label),
            };
            ui.label(egui::RichText::new(text).size(T_META).monospace().color(FG));
        }
        ui.label(egui::RichText::new(i18n::opt_effect_caveat()).size(T_META).color(FG_DIM));
    }

    if confirm {
        ui.add_space(S_SM);
        ui.label(egui::RichText::new(i18n::opt_confirm_risky()).size(T_META).color(YELLOW));
        ui.add_space(S_XS);
        ui.horizontal(|ui| {
            if button(ui, i18n::opt_btn_apply_anyway(), Emphasis::Danger).clicked() {
                ui.data_mut(|d| d.insert_temp(confirm_id(t), false));
                apply_one(app, ui, t);
            }
            if button(ui, i18n::hist_btn_cancel(), Emphasis::Ghost).clicked() {
                ui.data_mut(|d| d.insert_temp(confirm_id(t), false));
            }
        });
    }
}

fn apply_one(app: &mut App, ui: &mut egui::Ui, t: &dyn optimize::Tweak) {
    let net = app.net.clone();
    let now = ui.input(|inp| inp.time);
    match optimize::apply(t, &net) {
        Ok(msg) => {
            app.store.log_tweak(t.id(), "apply", "", &msg);
            app.toast(msg, GREEN, now);
        }
        Err(e) => {
            app.store.log_tweak(t.id(), "apply_failed", "", &e.to_string());
            app.toast(e.to_string(), RED, now);
        }
    }
    app.refresh_tweaks();
}

fn revert_one(app: &mut App, ui: &mut egui::Ui, t: &dyn optimize::Tweak) {
    let net = app.net.clone();
    let now = ui.input(|inp| inp.time);
    match optimize::revert(t, &net) {
        Ok(msg) => {
            app.store.log_tweak(t.id(), "revert", "", &msg);
            app.toast(msg, GREEN, now);
        }
        Err(e) => {
            app.store.log_tweak(t.id(), "revert_failed", "", &e.to_string());
            app.toast(e.to_string(), RED, now);
        }
    }
    app.refresh_tweaks();
}

/// An on/off switch. Painted, like the buttons, so it answers the pointer
/// and dims when it cannot be used rather than disappearing.
fn toggle(ui: &mut egui::Ui, on: bool, enabled: bool) -> egui::Response {
    let size = egui::vec2(40.0, 22.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, on);
        let track = if on { super::ACCENT } else { super::BG3 };
        let track = if enabled { track } else { track.gamma_multiply(0.45) };
        let track = if enabled && response.hovered() { track.gamma_multiply(1.15) } else { track };
        let radius = rect.height() / 2.0;
        ui.painter().rect(rect, radius, track, egui::Stroke::new(1.0_f32, super::LINE));
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        let knob = if enabled { FG } else { FG_DIM };
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), radius - 3.0, knob);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                radius + 2.0,
                egui::Stroke::new(1.0_f32, super::ACCENT),
            );
        }
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The channel advice, as one wide card over the changes. It answers a
/// different question from them (they change this machine; this produces a
/// setting to type into the router), so it is not one of the grid's cards.
///
/// Laid out answer first. It used to be a heading about "the air", a line
/// of how many networks were heard, the advice somewhere in the middle and
/// a run of "ch 1: 3 networks, -80 dBm" figures, which left the reader to
/// work out for themselves what to do.
fn air_card(app: &mut App, ui: &mut egui::Ui) {
    card(ui, i18n::air_title(), |ui| {
        ui.label(egui::RichText::new(i18n::air_blurb()).size(T_BODY).color(FG_DIM));
        ui.add_space(S_SM);
        ui.horizontal_wrapped(|ui| {
            let label =
                if app.air.aps.is_empty() { i18n::air_btn_scan() } else { i18n::air_btn_rescan() };
            let emphasis =
                if app.air.aps.is_empty() { Emphasis::Primary } else { Emphasis::Secondary };
            if button_ex(ui, label, emphasis, !app.air_scanning, 0.0).clicked() {
                start_air_scan(app, ui.ctx().clone());
            }
            // What pressing it costs, on screen rather than on hover: a
            // button that drops the Wi-Fi should say so before it is pressed.
            let note = if app.air_scanning { i18n::air_scanning() } else { i18n::air_scan_cost() };
            ui.label(egui::RichText::new(note).size(T_META).color(FG_DIM));
        });

        if let Some(err) = &app.air.error {
            ui.add_space(S_SM);
            ui.label(egui::RichText::new(i18n::air_failed(err)).size(T_BODY).color(YELLOW));
            return;
        }
        if app.air.aps.is_empty() {
            return;
        }

        ui.add_space(S_MD);
        air_verdict(app, ui);
        ui.add_space(S_MD);
        air_bars(app, ui);

        ui.add_space(S_SM);
        egui::CollapsingHeader::new(
            egui::RichText::new(i18n::air_networks(app.air.aps.len())).size(T_META).color(FG_DIM),
        )
        .id_salt("air_networks")
        .show(ui, |ui| air_table(app, ui));
    });
}

/// The one thing to do, or that there is nothing to do, in a panel of its
/// own with a coloured edge: the first thing the eye lands on after a scan.
fn air_verdict(app: &App, ui: &mut egui::Ui) {
    let air = &app.air;
    let cur = air.current;
    let (edge, headline, detail): (egui::Color32, String, Vec<String>) = if air.on_dfs() {
        let detail = vec![i18n::air_on_dfs(cur.unwrap_or(0)), i18n::air_dfs_move().to_string()];
        (YELLOW, i18n::air_verdict_dfs(cur.unwrap_or(0), air.best_5), detail)
    } else if air.worth_moving_24() {
        // `worth_moving_24` has already established both numbers exist.
        let best = air.best_24.unwrap_or(1);
        let gain = gain_24(app, best);
        (
            super::ACCENT,
            i18n::air_verdict_move(best),
            vec![
                i18n::air_verdict_move_why(cur.unwrap_or(0), gain),
                i18n::air_router_note().into(),
            ],
        )
    } else {
        let headline = match cur {
            Some(c) => i18n::air_verdict_fine(c),
            None => i18n::air_verdict_unknown().to_string(),
        };
        (GREEN, headline, vec![i18n::air_verdict_fine_why().to_string()])
    };

    let panel = egui::Frame::none()
        .fill(super::BG3)
        .rounding(super::BTN_R)
        .inner_margin(egui::Margin { left: S_MD + 4.0, right: S_MD, top: S_MD, bottom: S_MD })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(egui::RichText::new(headline).size(T_HEAD).strong().color(FG));
            for line in detail {
                ui.add_space(S_XS);
                ui.label(egui::RichText::new(line).size(T_META).color(FG_DIM));
            }
        });
    // Painted once the panel's size is known. Painted from inside it, the
    // edge ran to the bottom of everything that was left of the card.
    let r = panel.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(r.min, egui::vec2(4.0, r.height())),
        egui::Rounding { nw: super::BTN_R, sw: super::BTN_R, ne: 0.0, se: 0.0 },
        edge,
    );

    // The facts the verdict was read from, in one line under it.
    ui.add_space(S_XS);
    ui.label(
        egui::RichText::new(i18n::air_seen(air.aps.len(), air.co_channel()))
            .size(T_META)
            .color(FG_DIM),
    );
}

/// How crowded the channels worth comparing are, as bars. Longer is noisier.
/// The channel you are on and the one worth moving to are named on their
/// rows, so the comparison needs no reading of dBm.
///
/// Drawn for the band the card is on. It always showed 2.4 GHz, so someone
/// on 5 GHz was shown three channels that had nothing to do with them.
fn air_bars(app: &App, ui: &mut egui::Ui) {
    use crate::probe::airscan::{ChannelLoad, CLEAN_24, NON_DFS_5};
    let air = &app.air;
    let on_24 = air.current.is_none_or(|c| (1..=14).contains(&c));
    // The scan does not total up radar channels, so a card sitting on one has
    // no row of its own. It still counts the networks it heard there, and a
    // row from that is better than leaving out the channel the advice is about.
    let own_row: Option<ChannelLoad> = air
        .current
        .filter(|c| !on_24 && !air.load_5.iter().any(|l| l.channel == *c))
        .map(|c| ChannelLoad { channel: c, aps: air.co_channel(), noise_dbm: None });
    let rows: Vec<&ChannelLoad> = if on_24 {
        CLEAN_24.iter().filter_map(|c| air.load_24.iter().find(|l| l.channel == *c)).collect()
    } else {
        // Where you are, then the three quietest radar-free channels: the
        // choice the verdict is about, without a row for every channel.
        let mut free: Vec<&ChannelLoad> =
            air.load_5.iter().filter(|l| NON_DFS_5.contains(&l.channel)).collect();
        free.sort_by(|a, b| {
            let n = |l: &ChannelLoad| l.noise_dbm.unwrap_or(-120.0);
            n(a).total_cmp(&n(b)).then(a.channel.cmp(&b.channel))
        });
        let mut rows: Vec<&ChannelLoad> = air
            .load_5
            .iter()
            .filter(|l| Some(l.channel) == air.current)
            .chain(own_row.iter())
            .collect();
        rows.extend(free.into_iter().filter(|l| Some(l.channel) != air.current).take(3));
        rows
    };
    if rows.is_empty() {
        return;
    }
    let heading = if on_24 { i18n::air_bars_heading() } else { i18n::air_bars_heading_5() };
    ui.label(egui::RichText::new(heading).size(T_META).strong().color(FG_DIM));
    ui.add_space(S_XS);

    // -100 dBm is silence as far as a Wi-Fi card is concerned, -50 is a
    // neighbour's router on the other side of the wall.
    let fill = |noise: Option<f64>| noise.map_or(0.0, |n| ((n + 100.0) / 50.0).clamp(0.03, 1.0));
    let best = if !on_24 && (air.on_dfs() || air.best_5 != air.current) {
        air.best_5
    } else if air.worth_moving_24() {
        air.best_24
    } else {
        None
    };

    for l in rows {
        ui.horizontal(|ui| {
            let label_w = 70.0;
            ui.allocate_ui_with_layout(
                egui::vec2(label_w, 18.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(label_w);
                    ui.label(
                        egui::RichText::new(i18n::air_channel_no(l.channel)).size(T_BODY).color(FG),
                    );
                },
            );

            let bar_w = (ui.available_width() - 240.0).clamp(80.0, 360.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, 10.0), egui::Sense::hover());
            let f = fill(l.noise_dbm) as f32;
            let colour = if Some(l.channel) == best {
                GREEN
            } else if f > 0.6 {
                RED
            } else if f > 0.3 {
                YELLOW
            } else {
                GREEN
            };
            ui.painter().rect_filled(rect, 5.0, super::BG3);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * f, rect.height())),
                5.0,
                colour,
            );

            ui.add_space(S_SM);
            ui.label(
                egui::RichText::new(i18n::air_bar_note(l.aps, l.noise_dbm))
                    .size(T_META)
                    .color(FG_DIM),
            );
            if Some(l.channel) == air.current {
                ui.label(
                    egui::RichText::new(i18n::air_tag_yours()).size(T_META).strong().color(FG),
                );
            }
            if Some(l.channel) == best {
                ui.label(
                    egui::RichText::new(i18n::air_tag_best()).size(T_META).strong().color(GREEN),
                );
            }
        });
        ui.add_space(2.0);
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
    app.monitor.hold();
    std::thread::spawn(move || {
        let result = crate::probe::airscan::rescan(&net);
        let _ = tx.send(crate::ui::Job::AirDone(Box::new(result)));
        ctx.request_repaint();
    });
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
                app.store.log_tweak(t.id(), "apply_failed", "", &e.to_string());
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
    fn a_row_of_cards_is_as_tall_as_its_tallest_card_and_no_taller() {
        // Cards used to be padded with `set_min_height` after their content,
        // which counts from the cursor: every card came out the row's height
        // plus its own content, half of it empty.
        let ctx = egui::Context::default();
        let mut row_h = 0.0_f32;
        let mut drawn = Vec::new();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 800.0),
                )),
                ..Default::default()
            };
            drawn.clear();
            let mut wanted = 0.0_f32;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.columns(3, |cols| {
                        for (c, lines) in [3usize, 1, 2].iter().enumerate() {
                            let mut natural = 0.0;
                            let fr = egui::Frame::none()
                                .inner_margin(egui::Margin::symmetric(S_MD, CARD_PAD_Y))
                                .show(&mut cols[c], |ui| {
                                    let top = ui.min_rect().top();
                                    for _ in 0..*lines {
                                        ui.label("line");
                                    }
                                    natural = ui.min_rect().bottom() - top;
                                    level_to(ui, top, row_h - CARD_PAD_Y * 2.0);
                                });
                            drawn.push(fr.response.rect.height());
                            wanted = wanted.max(natural + CARD_PAD_Y * 2.0);
                        }
                    });
                });
            });
            row_h = wanted;
        }
        assert!(drawn.iter().all(|h| (h - row_h).abs() < 0.5), "{drawn:?} vs {row_h}");
    }

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
