//! The Statistics tab: the recorded history added up over a period, from
//! [`crate::tally`]. The figures that count events open the matching entries
//! in Outage history, so a number is never the end of the trail.

use eframe::egui;
use egui_plot::{Bar, BarChart, Plot};

use super::history::{Filter, Kinds};
use super::report::Range;
use super::{
    card, App, Job, Tab, FG, FG_DIM, GREEN, RED, S_MD, S_SM, S_XS, T_BODY, T_META, T_TITLE, YELLOW,
};
use crate::i18n;
use crate::monitor::Status;
use crate::store;
use crate::tally::{Bucket, HourShare, Period, Ping, Totals};
use std::sync::Arc;

/// How often the figures are counted again while the tab is open. The count
/// scans every sample in the period, so not every frame.
const REFRESH_S: f64 = 60.0;

/// Same threshold as the Live tab's cards: below it the row drops to fewer.
const CARD_MIN: f32 = 172.0;

/// Starts a count when the figures on hand are missing, stale or for another
/// period, and none is running.
fn request(app: &mut App) {
    let now = store::now();
    let days = app.stats_range.days();
    let fresh = app.stats.as_ref().is_some_and(|t| t.days == days && now - t.built_at < REFRESH_S);
    if fresh || app.stats_busy {
        return;
    }
    // Cleared when the count arrives. A panic in the count would leave it set
    // and the tab on "Counting…", but only in a debug build: release builds
    // abort on panic (Cargo.toml), taking the window with them.
    app.stats_busy = true;
    let (store, tx, ctx) = (app.store.clone(), app.tx.clone(), app.ctx.clone());
    let keep_days = app.settings.keep_days;
    std::thread::spawn(move || {
        let side = store.side_reader();
        let totals =
            crate::tally::count(side.as_ref().unwrap_or(&store), days, keep_days, store::now());
        let _ = tx.send(Job::Stats(Box::new(totals)));
        ctx.request_repaint();
    });
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    request(app);
    // Counted again every minute while the tab is open, with no input to
    // wake the window for it.
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(REFRESH_S));

    let range = app.stats_range.label();
    let mut open: Option<Filter> = None;
    egui::ScrollArea::vertical().id_salt("stats_page").auto_shrink([false, false]).show(ui, |ui| {
        header(app, ui);
        // Figures for another period are not shown under this one's name.
        let days = app.stats_range.days();
        let Some(t) = app.stats.as_ref().filter(|t| t.days == days) else {
            ui.label(egui::RichText::new(i18n::stats_counting()).size(T_BODY).color(FG_DIM));
            return;
        };
        let a = cards(ui, t, &range);
        prev_line(ui, t);
        if t.now.measured_from > t.now.from + 1.0 {
            meta(ui, &i18n::stats_samples_kept(t.keep_days, t.outage_keep_days()));
        }
        ui.add_space(S_MD);
        let b = causes(ui, t, &range);
        ping(ui, t);
        let c = when(ui, t);
        if let Some(hours) = &t.by_hour {
            time_of_day(ui, hours);
        }
        open = a.or(b).or(c);
    });
    if let Some(filter) = open {
        app.history_filter = Some(filter);
        app.selected_outage = None;
        app.tab = Tab::History;
    }
}

fn meta(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(T_META).color(FG_DIM));
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(i18n::stats_heading()).size(T_TITLE).strong().color(FG));
    ui.add_space(S_SM);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(i18n::stats_range()).size(T_BODY).color(FG_DIM));
        ui.spacing_mut().interact_size.y = super::BTN_H;
        egui::ComboBox::from_id_salt("stats_range")
            .selected_text(egui::RichText::new(app.stats_range.label()).size(T_BODY))
            .show_ui(ui, |ui| {
                for r in Range::ALL {
                    ui.selectable_value(&mut app.stats_range, r, r.label());
                }
            });
    });
    ui.add_space(S_MD);
}

fn pct(p: f64) -> String {
    format!("{p:.2} %")
}

/// A filter over the whole period for `kinds`, named `what`.
fn over_period(p: &Period, kinds: Kinds, what: &str, range: &str) -> Filter {
    Filter { from: p.from, to: p.to, kinds, label: format!("{what} · {range}") }
}

struct CardSpec {
    label: &'static str,
    value: String,
    sub: String,
    colour: egui::Color32,
    tip: &'static str,
    opens: Option<Kinds>,
}

fn cards(ui: &mut egui::Ui, t: &Totals, range: &str) -> Option<Filter> {
    let p = &t.now;
    let tally = &p.tally;
    let window = i18n::span(p.to - p.measured_from);
    let ok_or = |bad: bool, colour| if bad { colour } else { GREEN };

    let (uptime, uptime_sub, uptime_colour) = match p.availability() {
        Some(a) => {
            let colour = if a >= 99.9 {
                GREEN
            } else if a >= 99.0 {
                YELLOW
            } else {
                RED
            };
            (pct(a), i18n::stats_watched(&i18n::span(p.watched_s), &window), colour)
        }
        None => ("—".to_string(), i18n::stats_too_little().to_string(), FG_DIM),
    };
    let breaks_sub = if tally.ongoing {
        i18n::stats_ongoing().to_string()
    } else if tally.breaks > 0 {
        i18n::stats_longest(&i18n::span(tally.longest_s))
    } else {
        i18n::stats_no_breaks().to_string()
    };
    let (clear, clear_sub, clear_colour) = match p.longest_clear_s {
        Some(s) => (
            i18n::span(s),
            p.mean_between_breaks().map_or_else(
                || i18n::stats_no_drop_watched().to_string(),
                |m| i18n::stats_every(&i18n::span(m)),
            ),
            FG,
        ),
        None => ("—".to_string(), i18n::stats_too_little().to_string(), FG_DIM),
    };
    let (router, router_sub, router_colour) =
        match t.router.and_then(|r| r.restarts.map(|n| (n, r))) {
            Some((n, r)) => (
                n.to_string(),
                r.new_ips.map(i18n::stats_new_ips).unwrap_or_default(),
                ok_or(n > 0, YELLOW),
            ),
            None => ("—".to_string(), i18n::stats_router_silent().to_string(), FG_DIM),
        };

    let specs = [
        CardSpec {
            label: i18n::stats_offline(),
            value: i18n::span(tally.down_s),
            sub: if tally.breaks > 0 {
                i18n::stats_offline_sub(tally.breaks)
            } else {
                String::new()
            },
            colour: ok_or(tally.breaks > 0, RED),
            tip: i18n::stats_offline_tip(),
            opens: (tally.breaks > 0).then_some(Kinds::Breaks),
        },
        CardSpec {
            label: i18n::stats_uptime(),
            value: uptime,
            sub: uptime_sub,
            colour: uptime_colour,
            tip: i18n::stats_uptime_tip(),
            opens: None,
        },
        CardSpec {
            label: i18n::stats_breaks(),
            value: tally.breaks.to_string(),
            sub: breaks_sub,
            colour: ok_or(tally.breaks > 0, RED),
            tip: i18n::stats_breaks_tip(),
            opens: (tally.breaks > 0).then_some(Kinds::Breaks),
        },
        CardSpec {
            label: i18n::stats_slow(),
            value: tally.slow.to_string(),
            sub: if tally.slow > 0 {
                i18n::stats_slow_sub(&i18n::span(tally.slow_s))
            } else {
                String::new()
            },
            colour: ok_or(tally.slow > 0, YELLOW),
            tip: i18n::stats_slow_tip(),
            opens: (tally.slow > 0).then_some(Kinds::Slow),
        },
        CardSpec {
            label: i18n::stats_clear(),
            value: clear,
            sub: clear_sub,
            colour: clear_colour,
            tip: i18n::stats_clear_tip(),
            opens: None,
        },
        CardSpec {
            label: i18n::stats_router(),
            value: router,
            sub: router_sub,
            colour: router_colour,
            tip: i18n::stats_router_tip(),
            opens: None,
        },
    ];

    // Every row full, as on the Live tab: six, three or two across.
    let w = ui.available_width();
    let per_row = if w >= 6.0 * CARD_MIN {
        6
    } else if w >= 3.0 * CARD_MIN {
        3
    } else {
        2
    };
    let mut open = None;
    for (row, chunk) in specs.chunks(per_row).enumerate() {
        if row > 0 {
            ui.add_space(S_SM);
        }
        ui.columns(per_row, |cols| {
            for (i, c) in chunk.iter().enumerate() {
                let width = (cols[i].available_width() - 2.0 * S_MD).max(0.0);
                let resp = super::stat_card(
                    &mut cols[i],
                    c.label,
                    &c.value,
                    &c.sub,
                    c.colour,
                    Some(width),
                    c.tip,
                );
                if let Some(kinds) = &c.opens {
                    if super::card_clicked(&cols[i], &resp) {
                        open = Some(over_period(p, kinds.clone(), c.label, range));
                    }
                }
            }
        });
    }
    open
}

/// The period before, in one line, with how much of it was watched.
fn prev_line(ui: &mut egui::Ui, t: &Totals) {
    let (Some(prev), Some(days)) = (t.comparable_prev(), t.days) else {
        return;
    };
    ui.add_space(S_SM);
    let avail = prev.availability().map(pct);
    meta(
        ui,
        &i18n::stats_prev(
            days,
            &i18n::span(prev.watched_s),
            prev.tally.breaks,
            &i18n::span(prev.tally.down_s),
            prev.tally.slow,
            avail.as_deref(),
        ),
    );
}

/// Each kind of trouble, with a bar for its share of the troubled time.
fn causes(ui: &mut egui::Ui, t: &Totals, range: &str) -> Option<Filter> {
    let tally = &t.now.tally;
    let mut open = None;
    card(ui, i18n::stats_causes(), |ui| {
        if tally.by_kind.is_empty() {
            ui.label(egui::RichText::new(i18n::stats_nothing()).size(T_BODY).color(FG_DIM));
            return;
        }
        meta(ui, i18n::stats_causes_hint());
        ui.add_space(S_SM);
        let total: f64 = tally.by_kind.iter().map(|(_, _, s)| s).sum();
        egui::Grid::new("stats_causes").num_columns(4).spacing([S_MD, S_SM]).show(ui, |ui| {
            for (kind, n, secs) in &tally.by_kind {
                let colour = if kind == Status::Degraded.key() { YELLOW } else { RED };
                let name = i18n::event_kind(kind);
                let row = ui
                    .horizontal(|ui| {
                        super::status_dot(ui, colour, 4.0);
                        ui.add_space(S_XS);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&name).size(T_BODY).color(FG).underline(),
                            )
                            .sense(egui::Sense::click()),
                        )
                    })
                    .inner
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if row.clicked() {
                    open = Some(over_period(&t.now, Kinds::One(kind.clone()), &name, range));
                }
                ui.label(egui::RichText::new(format!("{n}×")).size(T_BODY).color(FG));
                ui.label(egui::RichText::new(i18n::span(*secs)).size(T_BODY).color(FG));
                let share = if total > 0.0 { (*secs / total) as f32 } else { 0.0 };
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(140.0, 6.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 3.0, super::BG3);
                let mut filled = rect;
                filled.set_width(rect.width() * share);
                ui.painter().rect_filled(filled, 3.0, colour);
                ui.end_row();
            }
        });
    });
    open
}

fn ping(ui: &mut egui::Ui, t: &Totals) {
    card(ui, i18n::stats_ping(), |ui| {
        egui::Grid::new("stats_ping").num_columns(6).spacing([S_MD * 2.0, S_SM]).show(ui, |ui| {
            ui.label("");
            for head in [
                i18n::stats_col_avg(),
                i18n::stats_col_p95(),
                i18n::stats_col_max(),
                i18n::stats_col_loss_up(),
                i18n::stats_col_loss_all(),
            ] {
                ui.label(egui::RichText::new(head).size(T_META).color(FG_DIM));
            }
            ui.end_row();
            for (name, p) in [
                (i18n::stats_ping_internet(), &t.internet),
                (i18n::stats_ping_router(), &t.router_ping),
            ] {
                ping_row(ui, name, p);
            }
        });
        ui.add_space(S_SM);
        meta(ui, i18n::stats_ping_note());
    });
}

fn ping_row(ui: &mut egui::Ui, name: &str, p: &Ping) {
    ui.label(egui::RichText::new(name).size(T_BODY).color(FG));
    let Some(all) = p.totals.loss_pct() else {
        ui.label(egui::RichText::new(i18n::stats_not_measured()).size(T_BODY).color(FG_DIM));
        ui.end_row();
        return;
    };
    let text = |ui: &mut egui::Ui, s: String, colour| {
        ui.label(egui::RichText::new(s).size(T_BODY).color(colour));
    };
    let ms = |v: Option<f64>| v.map_or("—".to_string(), |v| format!("{v:.0} ms"));
    text(ui, ms(p.totals.avg), FG);
    text(ui, ms(p.p95), FG);
    text(ui, ms(p.totals.max), FG);
    match p.outside_loss_pct() {
        // Loss while the line is up is what a provider is asked about.
        Some(up) => text(ui, pct(up), if up >= 1.0 { RED } else { FG }),
        None => text(ui, "—".into(), FG_DIM),
    }
    text(ui, pct(all), FG_DIM);
    ui.end_row();
}

/// What hovering a bar says about it.
type BarTip = Box<dyn Fn(&Bar, &BarChart) -> String>;

/// The hover text for one series: the bar's name, looked up from its
/// position only when it is hovered, and its value.
fn tip(
    series: &'static str,
    unit: &'static str,
    name: impl Fn(usize) -> String + 'static,
) -> BarTip {
    Box::new(move |bar, _| {
        let at = name(bar.argument.round().max(0.0) as usize);
        format!(
            "{at}
{series}: {:.1}{unit}",
            bar.value
        )
    })
}

/// Minutes down and slow per hour or per day, stacked. A click on a bar opens
/// what happened in it.
///
/// Nothing here allocates per bar per frame: the names were made once, with
/// the count, and the formatters hold the shared buckets rather than copies.
fn when(ui: &mut egui::Ui, t: &Totals) -> Option<Filter> {
    let mut open = None;
    card(ui, i18n::stats_when(), |ui| {
        ui.horizontal_wrapped(|ui| {
            meta(ui, if t.hourly { i18n::stats_when_hours() } else { i18n::stats_when_days() });
            for (name, colour) in [(i18n::stats_offline(), RED), (i18n::stats_slow(), YELLOW)] {
                ui.add_space(S_MD);
                super::status_dot(ui, colour, 4.0);
                meta(ui, name);
            }
        });
        meta(ui, i18n::stats_when_hint());
        ui.add_space(S_SM);

        let bars = |pick: fn(&Bucket) -> f64| -> Vec<Bar> {
            t.buckets.iter().enumerate().map(|(i, b)| Bar::new(i as f64, pick(b) / 60.0)).collect()
        };
        let label = |buckets: Arc<Vec<Bucket>>| {
            move |i: usize| buckets.get(i).map(|b| b.label.clone()).unwrap_or_default()
        };
        let down = BarChart::new(bars(|b| b.down_s)).color(RED).width(0.7).element_formatter(tip(
            i18n::stats_offline(),
            " min",
            label(t.buckets.clone()),
        ));
        let slow = BarChart::new(bars(|b| b.slow_s))
            .color(YELLOW)
            .width(0.7)
            .element_formatter(tip(i18n::stats_slow(), " min", label(t.buckets.clone())))
            .stack_on(&[&down]);

        let ticks = t.buckets.clone();
        let tick = move |i: usize| ticks.get(i).map(|b| b.tick.clone()).unwrap_or_default();
        let resp = bar_plot(("stats_when", t.hourly), t.buckets.len(), tick, "").show(ui, |plot| {
            plot.bar_chart(down);
            plot.bar_chart(slow);
            plot.pointer_coordinate()
        });
        if resp.response.clicked() {
            let hit = resp.inner.and_then(|p| {
                let i = p.x.round();
                ((p.x - i).abs() <= 0.35 && i >= 0.0).then_some(i as usize)
            });
            if let Some(b) = hit.and_then(|i| t.buckets.get(i)) {
                open = Some(Filter {
                    from: b.from,
                    to: b.to,
                    kinds: Kinds::Any,
                    label: b.label.clone(),
                });
            }
        }
    });
    open
}

/// A bar chart with the look the tab's charts share: fixed, no zoom, whole
/// positions labelled by `tick`.
fn bar_plot(
    id: impl std::hash::Hash,
    n: usize,
    tick: impl Fn(usize) -> String + 'static,
    unit: &'static str,
) -> Plot<'static> {
    Plot::new(id)
        .height(180.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .include_y(0.0)
        .include_y(1.0)
        .include_x(-0.5)
        .include_x(n as f64 - 0.5)
        .y_axis_min_width(44.0)
        .y_axis_formatter(move |mark, _| format!("{:.0}{unit}", mark.value))
        .x_axis_formatter(move |mark, _| {
            // Only whole positions have a bar under them.
            let i = mark.value.round();
            if (mark.value - i).abs() > 1e-6 || i < 0.0 || i as usize >= n {
                return String::new();
            }
            tick(i as usize)
        })
        .label_formatter(|_, _| String::new())
}

/// The share of each hour of the day that was down or slow, over the period.
fn time_of_day(ui: &mut egui::Ui, hours: &[HourShare; 24]) {
    card(ui, i18n::stats_hours(), |ui| {
        meta(ui, i18n::stats_hours_sub());
        let worst = hours
            .iter()
            .enumerate()
            .filter_map(|(h, s)| s.shares().map(|(d, sl)| (h, d + sl)))
            .filter(|(_, share)| *share > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((h, share)) = worst {
            ui.add_space(S_XS);
            ui.label(
                egui::RichText::new(i18n::stats_worst_hour(h, &format!("{share:.1} %")))
                    .size(T_BODY)
                    .color(FG),
            );
        }
        ui.add_space(S_SM);

        // An hour watched too little gets no bar at all, rather than a zero.
        let bars = |pick: fn((f64, f64)) -> f64| -> Vec<Bar> {
            hours
                .iter()
                .enumerate()
                .filter_map(|(h, s)| s.shares().map(|v| Bar::new(h as f64, pick(v))))
                .collect()
        };
        let hour = |h: usize| format!("{h}:00–{}:00", (h + 1) % 24);
        let down = BarChart::new(bars(|(d, _)| d)).color(RED).width(0.7).element_formatter(tip(
            i18n::stats_offline(),
            " %",
            hour,
        ));
        let slow = BarChart::new(bars(|(_, s)| s))
            .color(YELLOW)
            .width(0.7)
            .element_formatter(tip(i18n::stats_slow(), " %", hour))
            .stack_on(&[&down]);
        bar_plot("stats_hours", 24, |h| h.to_string(), " %").show(ui, |plot| {
            plot.bar_chart(down);
            plot.bar_chart(slow);
        });
    });
}
