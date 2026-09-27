//! The app's own widgets: buttons, cards, list rows, tooltips and the
//! layout helpers every tab shares. Colours and the type scale stay in
//! `ui/mod.rs`, which re-exports everything here.

use eframe::egui;

use super::{
    figure, ACCENT, BG, BG2, BG3, FG, FG_DIM, LINE, RED, S_LG, S_MD, S_SM, S_XS, T_BODY, T_HEAD,
    T_META, T_METRIC, T_MICRO,
};

/// Below this width a row of things laid out side by side stops fitting and
/// has to wrap or stack instead.
///
/// One number, shared, because a layout that breaks at a different width in
/// each tab reads as a bug rather than as a design. It is measured against
/// the width actually available for content, not the window, so a panel
/// inside a panel gets the same treatment.
pub const NARROW: f32 = 860.0;

pub fn is_narrow(ui: &egui::Ui) -> bool {
    ui.available_width() < NARROW
}

/// Draws `add` and holds the block at the tallest height it has needed at
/// this width.
///
/// The readings in the header, the summary, the cards and the legend change
/// every second, and in a small window a slightly longer one wraps onto
/// another line. The block then grew by a line and shrank back a second
/// later, and everything drawn under it jumped with it. Held, the block can
/// grow when something longer than ever before arrives, but it never shrinks,
/// so nothing under it moves back. A different width starts over, since the
/// old height says nothing about the new layout.
///
/// ponytail: the height is kept in memory, so each start of the app learns
/// it again; persist it per width if the first growth is a problem too.
pub fn steady<R>(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let id = ui.make_persistent_id(id_salt);
    let width = ui.available_width();
    let (was_width, held): (f32, f32) = ui.ctx().data(|d| d.get_temp(id)).unwrap_or((width, 0.0));
    let held = if (was_width - width).abs() < 0.5 { held } else { 0.0 };

    ui.vertical(|ui| {
        let top = ui.min_rect().top();
        let inner = add(ui);
        // The content's own height, measured before any padding: recording
        // the padded height would feed the padding back in and let the block
        // creep taller every frame.
        let used = ui.min_rect().bottom() - top;
        // Stretched to the exact edge, not padded with `add_space`: that
        // counts from the cursor, which already sits an item gap under the
        // content, so the short layout came out one gap taller than the tall
        // one and the block still jumped, by 8 px instead of a line.
        if held > used {
            let left = ui.min_rect().left();
            ui.expand_to_include_rect(egui::Rect::from_min_max(
                egui::pos2(left, top),
                egui::pos2(left, top + held),
            ));
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, (width, held.max(used))));
        inner
    })
    .inner
}

/// How wide a tooltip carrying prose is allowed to get.
///
/// egui lays a tooltip out on one line unless it is told not to, and these
/// are paragraphs. Around this width a line holds some sixty characters,
/// which is the span the eye can return from without losing its place.
pub const TIP_WIDTH: f32 = 380.0;

/// Prose in a tooltip, laid out to be read rather than scanned.
///
/// The text arrives as paragraphs separated by a blank line and is drawn as
/// paragraphs: a four-sentence explanation set as one block at the caption
/// size was technically present and practically unread. The opening paragraph
/// answers "what is this" and is set in the primary colour; what follows is
/// the detail, and is dimmer so the eye can stop after the first if that was
/// all it needed.
pub fn tip_prose(ui: &mut egui::Ui, text: &str) {
    ui.set_max_width(TIP_WIDTH);
    for (i, para) in text.split("\n\n").enumerate() {
        if i > 0 {
            ui.add_space(S_SM);
        }
        ui.label(egui::RichText::new(para).size(T_BODY).color(if i == 0 { FG } else { FG_DIM }));
    }
}

/// A tooltip's title: what the thing is called, over the prose about it.
pub fn tip_heading(ui: &mut egui::Ui, text: &str) {
    ui.set_max_width(TIP_WIDTH);
    ui.label(egui::RichText::new(text).size(T_HEAD).color(FG).strong());
    ui.add_space(S_XS);
    ui.separator();
    ui.add_space(S_SM);
}

/// A titled panel spanning the width it is given: the one container the
/// settings pages and the outage detail are built from, so a section looks
/// the same wherever it appears.
pub fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
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

/// Grid steps for a plot's value axis: four or five labelled lines
/// over the visible range, each a round 1, 2 or 5 times a power of ten.
///
/// egui_plot's own decimal grid only labels a line once the lines are far
/// enough apart, and on plots this short it often labelled none: a signal
/// plot with no numbers, and a latency plot showing only its zero once one
/// spike stretched the range. Shared by the outage lead-up and the load test.
pub fn y_steps(input: egui_plot::GridInput) -> [f64; 3] {
    let span = (input.bounds.1 - input.bounds.0).abs().max(1.0);
    let rough = span / 5.0;
    let magnitude = 10f64.powf(rough.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= rough)
        .unwrap_or(10.0 * magnitude);
    [step / 5.0, step, step * 5.0]
}

/// How a legend entry is drawn, matching the mark it names.
pub enum Key {
    Line,
    Dot,
    Span,
}

/// The legend over a plot rather than inside it. Inside, egui's legend sits
/// on a panel in a corner of the data, and the corner it covers is the
/// lead-up, which is what the plot is there to show.
pub fn legend_row(ui: &mut egui::Ui, entries: &[(&str, egui::Color32, Key)]) {
    ui.horizontal_wrapped(|ui| {
        for (label, colour, key) in entries {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, T_META), egui::Sense::hover());
            let painter = ui.painter();
            match key {
                Key::Line => {
                    painter.hline(rect.x_range(), rect.center().y, egui::Stroke::new(2.0, *colour));
                }
                Key::Dot => {
                    painter.circle_filled(rect.center(), 3.0, *colour);
                }
                Key::Span => {
                    painter.rect_filled(rect.shrink2(egui::vec2(2.0, 1.0)), 2.0, *colour);
                }
            }
            ui.label(egui::RichText::new(*label).size(T_META).color(FG_DIM));
            ui.add_space(S_SM);
        }
    });
    ui.add_space(S_XS);
}

/// One entry in a list beside a detail pane: a status dot, a title with an
/// optional figure at the far right, and a second line cut to the row.
///
/// The whole row is the click target. The history tab used to make only the
/// date clickable, which left most of the row looking clickable and doing
/// nothing. Shared so the history and optimise lists look and answer alike.
///
/// The second line is cut rather than wrapped: a list whose rows change
/// height with their text is not a list to scan. Its first piece lifts to
/// full strength when the row is hovered or selected, the rest stay dim.
#[allow(clippy::too_many_arguments)]
pub fn list_row(
    ui: &mut egui::Ui,
    height: f32,
    selected: bool,
    dot: egui::Color32,
    title: &str,
    title_mono: bool,
    trailing: Option<(&str, egui::Color32)>,
    sub: &[(&str, egui::Color32)],
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let hovered = response.hovered();
    let active = selected || hovered;
    let inner = rect.shrink2(egui::vec2(S_MD, S_SM));
    let top = inner.top() + T_BODY * 0.6;
    let bottom = inner.bottom() - T_META * 0.6;
    let text_x = inner.left() + 16.0;

    let body = egui::FontId::new(T_BODY, egui::FontFamily::Proportional);
    let trailing = trailing.map(|(text, colour)| {
        ui.fonts(|f| f.layout_no_wrap(text.to_string(), body.clone(), colour))
    });
    let trailing_w = trailing.as_ref().map_or(0.0, |g| g.size().x + S_SM);

    let title_font = if title_mono {
        egui::FontId::new(T_BODY, egui::FontFamily::Monospace)
    } else {
        body.clone()
    };
    let mut title_job = egui::text::LayoutJob::simple_singleline(title.to_string(), title_font, FG);
    title_job.wrap =
        egui::text::TextWrapping::truncate_at_width(inner.right() - text_x - trailing_w);
    let title_galley = ui.fonts(|f| f.layout_job(title_job));

    let meta = egui::FontId::new(T_META, egui::FontFamily::Proportional);
    let mut job = egui::text::LayoutJob::default();
    for (i, (text, colour)) in sub.iter().enumerate() {
        let colour = if i == 0 && !active { FG_DIM } else { *colour };
        job.append(text, 0.0, egui::TextFormat::simple(meta.clone(), colour));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(inner.right() - text_x);
    let sub_galley = ui.fonts(|f| f.layout_job(job));

    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, BTN_R, if selected { BG3 } else { BG2 });
    }
    if selected {
        let bar = egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(0.0, 8.0),
            egui::vec2(3.0, rect.height() - 16.0),
        );
        painter.rect_filled(bar, 1.5, ACCENT);
    }
    painter.circle_filled(egui::pos2(inner.left() + 4.0, top), 4.0, dot);
    painter.galley(egui::pos2(text_x, top - title_galley.size().y * 0.5), title_galley, FG);
    if let Some(g) = trailing {
        let pos = egui::pos2(inner.right() - g.size().x, top - g.size().y * 0.5);
        painter.galley(pos, g, FG);
    }
    painter.galley(egui::pos2(text_x, bottom - sub_galley.size().y * 0.5), sub_galley, FG_DIM);

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A stat card, used on the live tab: one headline number,
/// its name, and a line of context under it.
///
/// `width` pins the card's inner width, which is what a row of cards laid out
/// in columns needs: left to size themselves, cards came out different widths
/// depending on how many digits their value happened to have that second, and
/// the row stopped being a row.
///
/// `tip` is what the number means — not a repeat of the label. A card says
/// "Jitter, 2.3 ms" to someone who already knows what jitter is and nothing
/// at all to anyone else, and the second group is who this app is for. It
/// opens from the [`help_mark`] beside the label, not from the whole card.
pub fn stat_card(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    sub: &str,
    colour: egui::Color32,
    width: Option<f32>,
    tip: &str,
) -> egui::Response {
    let mut help_hovered = false;
    let response = egui::Frame::none()
        .fill(BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(S_MD))
        .show(ui, |ui| {
            match width {
                Some(w) => {
                    ui.set_min_width(w);
                    ui.set_max_width(w);
                }
                None => ui.set_min_width(140.0),
            }
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = S_XS;
                    ui.label(egui::RichText::new(label).size(T_META).color(FG_DIM));
                    if !tip.is_empty() {
                        let mark = help_mark(ui);
                        help_hovered = mark.hovered();
                        mark.on_hover_ui(|ui| {
                            tip_heading(ui, label);
                            tip_prose(ui, tip);
                        });
                    }
                });
                // The value is the reason the card exists and it changes every
                // second, so it is the one that most needs its digits to stay
                // in place between frames.
                ui.label(figure(value, T_METRIC, colour).strong());
                ui.add_space(S_XS * 0.5);
                // A card with nothing to say on the third line still keeps the
                // line. Without it that card is shorter than the ones beside
                // it, and a row of cards at different heights reads as a
                // layout fault rather than as a card with less to say.
                let sub = if sub.is_empty() { "\u{a0}" } else { sub };
                ui.label(egui::RichText::new(sub).size(T_MICRO).color(FG_DIM));
            });
        })
        .response;
    ui.data_mut(|d| d.insert_temp(response.id.with("help"), help_hovered));
    response
}

/// Whether a card from [`stat_card`] that opens something was clicked,
/// anywhere but on its help mark.
///
/// Not a click target laid over the card: registered after the card's
/// content, it sat on top of the help mark and took the pointer from it, so
/// the explanation never opened on the one card that also opens a tab.
pub fn card_clicked(ui: &egui::Ui, card: &egui::Response) -> bool {
    let on_help = ui.data(|d| d.get_temp::<bool>(card.id.with("help"))).unwrap_or(false);
    if on_help || !ui.rect_contains_pointer(card.rect) {
        return false;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    ui.input(|i| i.pointer.primary_clicked())
}

/// A small "?" that opens an explanation when the pointer is on it.
///
/// The explanation used to open on the whole card, so running the pointer
/// across the cards, or scrolling past them, threw paragraphs up over the
/// figures. On a mark of its own it opens for someone who asks.
pub fn help_mark(ui: &mut egui::Ui) -> egui::Response {
    let size = T_META + 2.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let colour = if response.hovered() { ACCENT } else { FG_DIM };
    let painter = ui.painter();
    painter.circle_stroke(rect.center(), size * 0.5 - 0.5, egui::Stroke::new(1.0_f32, colour));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "?",
        egui::FontId::new(T_MICRO, egui::FontFamily::Proportional),
        colour,
    );
    response.on_hover_cursor(egui::CursorIcon::Help)
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

// Every button in the app was `ui.button`, which means every button looked the
// same: the export sitting next to the diagnostic sitting next to the dismiss,
// all in the same grey, all claiming the same weight. A row of equals is a row
// with no entry point — the eye has to read all of it to find the one thing it
// came for. These four levels exist so a row can say which button that is.
//
// They are painted rather than configured through `Visuals`, because egui
// derives hover and pressed from the widget visuals, and an explicit `fill()`
// on a `Button` freezes it in every state — the button stops answering the
// pointer. A button that does not react to the cursor does not read as
// clickable, and that is the one thing it has to say.

/// One height for every button, so a row of them shares a baseline and a
/// centre line regardless of what each one is labelled.
pub const BTN_H: f32 = 30.0;
/// Matches the 6.0 the stat cards and the note panels already use.
pub const BTN_R: f32 = 6.0;
/// Horizontal breathing room inside a button, on the same spacing scale.
const BTN_PAD_X: f32 = S_MD;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    /// The action the row exists for. At most one per row — a second primary
    /// makes both of them secondary.
    Primary,
    /// A real action that is not the point of the row.
    Secondary,
    /// A side action that should stay legible without competing: dismiss,
    /// export, toggle a view.
    Ghost,
    /// Changes the machine, or cannot be taken back. Outlined rather than
    /// filled: it should be findable, not inviting.
    Danger,
}

/// Fill, stroke and text for a button in a given state.
fn btn_colours(
    emphasis: Emphasis,
    enabled: bool,
    hovered: bool,
    pressed: bool,
) -> (egui::Color32, egui::Color32, egui::Color32) {
    use egui::Color32 as C;
    const CLEAR: egui::Color32 = C::TRANSPARENT;

    if !enabled {
        // Dimmed rather than hidden. "Not now" is an answer, and a control
        // that vanishes when unavailable teaches the user it was never there.
        return (BG2, LINE.linear_multiply(0.6), FG_DIM.linear_multiply(0.55));
    }

    match emphasis {
        Emphasis::Primary => {
            let fill = if pressed {
                C::from_rgb(0x3d, 0x8a, 0xdb)
            } else if hovered {
                C::from_rgb(0x6c, 0xb4, 0xff)
            } else {
                ACCENT
            };
            // Dark text on the accent. White on `#4da3ff` sits near 2.4:1;
            // the page background against it clears 7:1.
            (fill, CLEAR, BG)
        }
        Emphasis::Secondary => {
            let fill = if pressed {
                C::from_rgb(0x3a, 0x41, 0x50)
            } else if hovered {
                C::from_rgb(0x32, 0x38, 0x46)
            } else {
                BG3
            };
            (fill, LINE, FG)
        }
        Emphasis::Ghost => {
            let fill = if pressed {
                BG3
            } else if hovered {
                BG2
            } else {
                CLEAR
            };
            // The label lifts to full strength on hover, which is most of what
            // tells you a ghost button is a button at all.
            (fill, CLEAR, if hovered || pressed { FG } else { FG_DIM })
        }
        Emphasis::Danger => {
            let fill = if pressed {
                C::from_rgb(0x3a, 0x23, 0x28)
            } else if hovered {
                C::from_rgb(0x2e, 0x1e, 0x22)
            } else {
                CLEAR
            };
            (fill, RED.linear_multiply(0.55), RED)
        }
    }
}

/// Width a button needs for a label, before any minimum is applied.
///
/// A toggle whose two labels are different lengths resizes as you click it,
/// and every button to its right slides. Measure both, pass the larger as
/// `min_w`, and the row holds still.
pub fn btn_width(ui: &egui::Ui, label: &str) -> f32 {
    let font = egui::FontId::new(T_BODY, egui::FontFamily::Proportional);
    let galley =
        ui.fonts(|f| f.layout_no_wrap(label.to_string(), font, egui::Color32::PLACEHOLDER));
    galley.size().x + BTN_PAD_X * 2.0
}

/// A button at a given emphasis. `min_w` of 0.0 means "fit the label".
pub fn button_ex(
    ui: &mut egui::Ui,
    label: &str,
    emphasis: Emphasis,
    enabled: bool,
    min_w: f32,
) -> egui::Response {
    let font = egui::FontId::new(T_BODY, egui::FontFamily::Proportional);
    // Laid out in PLACEHOLDER so the paint call can supply the colour once the
    // response says which state we are in.
    let galley =
        ui.fonts(|f| f.layout_no_wrap(label.to_string(), font, egui::Color32::PLACEHOLDER));

    let w = (galley.size().x + BTN_PAD_X * 2.0).max(min_w);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(w, BTN_H), sense);

    if ui.is_rect_visible(rect) {
        let (fill, stroke, text) = btn_colours(
            emphasis,
            enabled,
            response.hovered(),
            response.is_pointer_button_down_on(),
        );

        ui.painter().rect(rect, BTN_R, fill, egui::Stroke::new(1.0_f32, stroke));

        // Keyboard focus, drawn outside the button so it never eats into the
        // label or the fill.
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                BTN_R + 2.0,
                egui::Stroke::new(1.0_f32, ACCENT),
            );
        }

        let pos = rect.center() - galley.size() * 0.5;
        ui.painter().galley(pos, galley, text);
    }

    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The common case: enabled, sized to its label.
pub fn button(ui: &mut egui::Ui, label: &str, emphasis: Emphasis) -> egui::Response {
    button_ex(ui, label, emphasis, true, 0.0)
}
