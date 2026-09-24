//! Load test tab: latency under saturation (bufferbloat).
//!
//! Read top to bottom it answers, in order: how bad is it (the grade), where
//! did the ping go (three cards, one per phase), what did it look like (the
//! course of every ping), and what to do about it. Before the first run the
//! same place says what the test will do and what it costs.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints, Points, Polygon};

use super::{
    button, button_ex, card, legend_row, stat_card_ex, status_dot, y_steps, App, Emphasis, Job,
    Key, ACCENT, BG, BG2, BG3, FG, FG_DIM, GREEN, LINE, RED, SERIES_COLOURS, S_LG, S_MD, S_SM,
    S_XS, T_BODY, T_LEAD, T_META, T_METRIC, T_TITLE, YELLOW,
};
use crate::bandwidth::{self, BloatResult, Grade, Phase};
use crate::i18n;

/// How long the tab's test holds each phase. Longer than the scan's, which
/// only needs the grade; here the course of the ping is the point.
const IDLE: Duration = Duration::from_secs(6);
const LOAD: Duration = Duration::from_secs(12);
/// Below this the three result cards stack instead of sharing a row.
const CARDS_MIN_W: f32 = 560.0;

fn grade_colour(g: Grade) -> egui::Color32 {
    match g {
        Grade::A | Grade::B => GREEN,
        Grade::C => YELLOW,
        Grade::D | Grade::F => RED,
        Grade::Unknown => FG_DIM,
    }
}

/// The same bands as the grades, for either direction's rise.
fn bump_colour(bump: f64) -> egui::Color32 {
    if bump < 60.0 {
        GREEN
    } else if bump < 150.0 {
        YELLOW
    } else {
        RED
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    header(app, ui);
    ui.add_space(S_MD);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if app.bloat_running {
            running(app, ui);
        }
        if app.bloat_at.is_some() {
            result(app, ui);
        } else if !app.bloat_running {
            empty(ui);
        }
    });
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(i18n::bloat_title()).size(T_LEAD).strong().color(FG));
    ui.add_space(S_XS);
    ui.label(egui::RichText::new(i18n::bloat_blurb()).size(T_BODY).color(FG_DIM));
    ui.add_space(S_MD);

    // The scan's own load step would fill the line at the same time, and
    // each would then be measuring the other.
    let blocked = app.scanning && !app.bloat_running;
    ui.horizontal(|ui| {
        if app.bloat_running {
            if button(ui, i18n::bloat_btn_stop(), Emphasis::Secondary).clicked() {
                app.bloat_cancel.store(true, Ordering::Relaxed);
            }
        } else {
            let label = if app.bloat_at.is_some() {
                i18n::bloat_btn_rerun()
            } else {
                i18n::bloat_btn_run()
            };
            if button_ex(ui, label, Emphasis::Primary, !blocked, 0.0).clicked() {
                start(app);
            }
        }
        ui.add_space(S_SM);
        // The test is the one thing in this app that spends the user's data
        // allowance, and on a phone hotspot it can spend a lot of it. Said
        // beside the button, not after the bill.
        let (dot, text) = if blocked {
            (FG_DIM, i18n::bloat_busy_scan())
        } else {
            (YELLOW, i18n::bloat_cost_warning())
        };
        status_dot(ui, dot, 3.5);
        ui.add(egui::Label::new(egui::RichText::new(text).size(T_META).color(FG_DIM)).wrap());
    });
}

/// Which of the three phases the running test is in.
fn phase_index(progress: f32) -> usize {
    if progress >= bandwidth::PROGRESS_UP {
        2
    } else if progress >= bandwidth::PROGRESS_DOWN {
        1
    } else {
        0
    }
}

fn running(app: &App, ui: &mut egui::Ui) {
    // The countdown moves with the clock, not only when the test reports in.
    ui.ctx().request_repaint_after(Duration::from_millis(250));
    let elapsed = app.bloat_started.map_or(0.0, |t| t.elapsed().as_secs_f64());
    let total = bandwidth::expected_secs(IDLE, LOAD);
    // ponytail: progress is read off the clock against the planned length,
    // so a slow join at the end holds at 98% rather than overshooting. A
    // per-ping report from `bandwidth::run` would make it exact.
    let frac = ((elapsed / total) as f32).max(app.bloat_progress).min(0.98);
    let left = total - elapsed;

    card(ui, i18n::bloat_running_title(), |ui| {
        stepper(ui, phase_index(app.bloat_progress));
        ui.add_space(S_MD);
        ui.add(egui::ProgressBar::new(frac).desired_height(6.0));
        ui.add_space(S_XS);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&app.bloat_label).size(T_META).color(FG_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let eta = if left > 0.5 {
                    i18n::bloat_remaining(left)
                } else {
                    i18n::bloat_finishing().to_string()
                };
                ui.label(egui::RichText::new(eta).size(T_META).color(FG_DIM));
            });
        });
        ui.add_space(S_SM);
        ui.label(egui::RichText::new(i18n::bloat_monitor_paused()).size(T_META).color(FG_DIM));
    });
}

/// The three phases as numbered steps: done ones green, the current one in
/// the accent, the rest outlined.
fn stepper(ui: &mut egui::Ui, current: usize) {
    let steps = [i18n::bloat_step_idle(), i18n::bloat_step_down(), i18n::bloat_step_up()];
    ui.horizontal(|ui| {
        for (i, name) in steps.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            let painter = ui.painter();
            let (fill, number, text) = match i.cmp(&current) {
                std::cmp::Ordering::Less => (GREEN, BG, FG_DIM),
                std::cmp::Ordering::Equal => (ACCENT, BG, FG),
                std::cmp::Ordering::Greater => (BG3, FG_DIM, FG_DIM),
            };
            painter.circle(rect.center(), 10.0, fill, egui::Stroke::new(1.0, LINE));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                egui::FontId::proportional(T_META),
                number,
            );
            ui.label(egui::RichText::new(*name).size(T_BODY).color(text));
            if i + 1 < steps.len() {
                let (line, _) =
                    ui.allocate_exact_size(egui::vec2(28.0, 20.0), egui::Sense::hover());
                ui.painter().hline(line.x_range(), line.center().y, egui::Stroke::new(1.0, LINE));
            }
        }
    });
}

fn result(app: &App, ui: &mut egui::Ui) {
    let r = &app.bloat;
    let g = r.grade_or_unknown();
    verdict(ui, r);
    ui.add_space(S_MD);
    cards(ui, r);
    ui.add_space(S_MD);
    if r.samples.iter().any(|s| s.phase != Phase::Idle) {
        card(ui, i18n::bloat_chart_title(), |ui| chart(ui, r));
    }
    // Unmeasured, the advice would only repeat the reason under the grade.
    if g != Grade::Unknown {
        let advice = bandwidth::advice(r);
        card(ui, i18n::bloat_advice_title(), |ui| {
            ui.label(egui::RichText::new(&advice.lead).size(T_BODY).color(FG));
            if !advice.steps.is_empty() {
                ui.add_space(S_MD);
                ui.label(
                    egui::RichText::new(i18n::bloat_advice_header())
                        .size(T_BODY)
                        .strong()
                        .color(FG_DIM),
                );
                ui.add_space(S_XS);
                for (i, step) in advice.steps.iter().enumerate() {
                    numbered(ui, i + 1, step);
                }
            }
            if let Some(note) = &advice.note {
                ui.add_space(S_MD);
                ui.label(egui::RichText::new(note).size(T_META).color(FG_DIM));
            }
        });
    }

    let mut foot = Vec::new();
    if let Some(at) = app.bloat_at {
        foot.push(i18n::bloat_measured_at(&crate::diagnose::format_clock(at)));
    }
    if r.total_bytes() > 0 {
        foot.push(i18n::bloat_data_used(r.total_bytes() as f64 / 1_000_000.0));
    }
    ui.label(egui::RichText::new(foot.join(" · ")).size(T_META).color(FG_DIM));
}

/// The grade, what it means, where the ping went, and anything that makes
/// the grade less than the whole story.
fn verdict(ui: &mut egui::Ui, r: &BloatResult) {
    let g = r.grade_or_unknown();
    let colour = grade_colour(g);

    let mut lines: Vec<(String, egui::Color32)> = Vec::new();
    if let Some(summary) = summary(r) {
        lines.push((summary, FG_DIM));
    }
    if !r.error.is_empty() {
        // With no grade the error is the reason; with one it is a caveat.
        lines.push((r.error.clone(), if g == Grade::Unknown { FG_DIM } else { YELLOW }));
    }
    if let Some(up) = r.upload.as_ref().filter(|u| !u.note.is_empty()) {
        lines.push((up.note.clone(), YELLOW));
    }

    let panel = egui::Frame::none()
        .fill(BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin { left: S_LG + 4.0, right: S_LG, top: S_LG, bottom: S_LG })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (badge, _) =
                    ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
                ui.painter().rect(
                    badge,
                    8.0,
                    colour.gamma_multiply(0.14),
                    egui::Stroke::new(1.0, colour.gamma_multiply(0.5)),
                );
                ui.painter().text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    g.letter(),
                    egui::FontId::proportional(T_METRIC * 1.3),
                    colour,
                );
                ui.add_space(S_MD);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(g.verdict()).size(T_TITLE).strong().color(FG));
                    for (text, colour) in &lines {
                        ui.add_space(S_XS);
                        ui.label(egui::RichText::new(text).size(T_BODY).color(*colour));
                    }
                    ui.add_space(S_SM);
                    scale(ui, g);
                });
            });
        });
    // Painted once the panel's size is known, like the Wi-Fi verdict's edge.
    let rect = panel.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(4.0, rect.height())),
        egui::Rounding { nw: 6.0, sw: 6.0, ne: 0.0, se: 0.0 },
        colour,
    );
}

/// "The ping goes from 12 ms to 85 ms", in whichever direction it went
/// furthest. `None` when either end was not measured.
fn summary(r: &BloatResult) -> Option<String> {
    let idle = r.idle_avg?;
    let down = r.loaded_avg.zip(r.bump_ms);
    let up = r.upload.as_ref().and_then(|u| u.loaded_avg.zip(u.bump_ms));
    let (loaded, upload) = match (down, up) {
        (Some(d), Some(u)) if u.1 > d.1 => (u.0, true),
        (Some(d), _) => (d.0, false),
        (None, Some(u)) => (u.0, true),
        (None, None) => return None,
    };
    Some(i18n::bloat_summary(idle, loaded, upload))
}

/// Every grade with the rise it covers, the one this line got picked out.
/// A letter alone says nothing to someone who has never seen the scale.
fn scale(ui: &mut egui::Ui, current: Grade) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = S_XS;
        let mut floor = None;
        for g in Grade::SCALE {
            let range = match (floor, g.ceiling_ms()) {
                (None, Some(c)) => format!("< {c:.0}"),
                (Some(f), Some(c)) => format!("{f:.0}-{c:.0}"),
                (Some(f), None) => format!("{f:.0}+"),
                (None, None) => String::new(),
            };
            floor = g.ceiling_ms();
            let on = g == current;
            let colour = grade_colour(g);
            egui::Frame::none()
                .fill(if on { colour.gamma_multiply(0.16) } else { BG3 })
                .stroke(egui::Stroke::new(1.0, if on { colour.gamma_multiply(0.6) } else { LINE }))
                .rounding(4.0)
                .inner_margin(egui::Margin::symmetric(S_SM, 2.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = S_XS;
                    ui.label(egui::RichText::new(g.letter()).size(T_META).strong().color(if on {
                        colour
                    } else {
                        FG_DIM
                    }));
                    ui.label(egui::RichText::new(range).size(T_META).color(if on {
                        FG
                    } else {
                        FG_DIM
                    }));
                });
        }
        ui.label(egui::RichText::new("ms").size(T_META).color(FG_DIM));
    });
}

/// One card per phase: the idle baseline, then the ping with the line full
/// each way, with the rise and the speed under it.
fn cards(ui: &mut egui::Ui, r: &BloatResult) {
    let none = || ("-".to_string(), FG_DIM);
    let loaded = |avg: Option<f64>, bump: Option<f64>, silent: bool| match (avg, bump) {
        (Some(v), b) => (format!("{v:.0} ms"), bump_colour(b.unwrap_or(0.0))),
        (None, _) if silent => (i18n::bloat_no_answer().to_string(), RED),
        (None, _) => none(),
    };
    let idle = r.idle_avg.map_or_else(none, |v| (format!("{v:.0} ms"), FG));
    let down = loaded(r.loaded_avg, r.bump_ms, r.grade == Some(Grade::F));
    let up = r
        .upload
        .as_ref()
        .map_or_else(none, |u| loaded(u.loaded_avg, u.bump_ms, u.grade == Some(Grade::F)));
    let up_sub =
        r.upload.as_ref().map_or_else(String::new, |u| i18n::bloat_card_sub(u.bump_ms, u.mbps));
    let items = [
        (i18n::bloat_card_idle(), idle, "1.1.1.1".to_string(), i18n::bloat_tip_idle()),
        (
            i18n::bloat_card_loaded(),
            down,
            i18n::bloat_card_sub(r.bump_ms, r.mbps),
            i18n::bloat_tip_loaded(),
        ),
        (i18n::bloat_card_loaded_up(), up, up_sub, i18n::bloat_tip_loaded_up()),
    ];

    let one = |ui: &mut egui::Ui, i: usize| {
        let (label, (value, colour), sub, tip) = &items[i];
        let w = ui.available_width() - S_MD * 2.0;
        stat_card_ex(ui, label, value, sub, *colour, Some(w), tip);
    };
    if ui.available_width() < CARDS_MIN_W {
        for i in 0..items.len() {
            one(ui, i);
            ui.add_space(S_SM);
        }
    } else {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = S_MD;
            ui.columns(items.len(), |cols| {
                for (i, col) in cols.iter_mut().enumerate() {
                    one(col, i);
                }
            });
        });
    }
}

/// Every ping of the test against time, a line per phase, with the two
/// loaded stretches shaded. An average hides the spikes a game feels.
fn chart(ui: &mut egui::Ui, r: &BloatResult) {
    let colour = |p: Phase| match p {
        Phase::Idle => SERIES_COLOURS[0],
        Phase::Down => SERIES_COLOURS[2],
        Phase::Up => SERIES_COLOURS[4],
    };
    let name = |p: Phase| match p {
        Phase::Idle => i18n::bloat_step_idle(),
        Phase::Down => i18n::bloat_step_down(),
        Phase::Up => i18n::bloat_step_up(),
    };
    let phases = [Phase::Idle, Phase::Down, Phase::Up];
    let top = r.samples.iter().filter_map(|s| s.rtt).fold(20.0_f64, f64::max);
    // Unanswered pings sit in a row just over the highest answer.
    let lost_y = top * 1.08;
    let lost: Vec<[f64; 2]> =
        r.samples.iter().filter(|s| s.rtt.is_none()).map(|s| [s.t, lost_y]).collect();

    ui.label(egui::RichText::new(i18n::bloat_chart_caption()).size(T_META).color(FG_DIM));
    ui.add_space(S_XS);
    let mut keys: Vec<(&str, egui::Color32, Key)> =
        phases.iter().map(|p| (name(*p), colour(*p), Key::Line)).collect();
    if !lost.is_empty() {
        keys.push((i18n::bloat_chart_lost(), RED, Key::Dot));
    }
    legend_row(ui, &keys);

    let lost_name = i18n::bloat_chart_lost();
    Plot::new("bloat_course")
        .height(200.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .include_x(0.0)
        .include_y(0.0)
        .include_y(lost_y * 1.04)
        .y_axis_min_width(44.0)
        .y_grid_spacer(egui_plot::uniform_grid_spacer(y_steps))
        .y_axis_formatter(|mark, _| format!("{:.0}", mark.value))
        .x_axis_formatter(|mark, _| format!("{:.0}", mark.value))
        .x_axis_label(i18n::bloat_chart_axis())
        .label_formatter(move |name, p| {
            if name.is_empty() {
                String::new()
            } else if name == lost_name {
                format!("{name}\n{:.1} s", p.x)
            } else {
                format!("{name}: {:.0} ms\n{:.1} s", p.y, p.x)
            }
        })
        .show(ui, |plot| {
            for p in phases {
                let points: Vec<[f64; 2]> = r
                    .samples
                    .iter()
                    .filter(|s| s.phase == p)
                    .filter_map(|s| s.rtt.map(|rtt| [s.t, rtt]))
                    .collect();
                let span = r.samples.iter().filter(|s| s.phase == p).fold(
                    None,
                    |acc: Option<(f64, f64)>, s| {
                        Some(acc.map_or((s.t, s.t), |(a, b)| (a.min(s.t), b.max(s.t))))
                    },
                );
                if let (Some((a, b)), true) = (span, p != Phase::Idle) {
                    plot.polygon(
                        Polygon::new(PlotPoints::from(vec![
                            [a, 0.0],
                            [b, 0.0],
                            [b, lost_y * 1.04],
                            [a, lost_y * 1.04],
                        ]))
                        .fill_color(colour(p).gamma_multiply(0.08))
                        .stroke(egui::Stroke::NONE),
                    );
                }
                plot.line(Line::new(PlotPoints::from(points)).name(name(p)).color(colour(p)));
            }
            if !lost.is_empty() {
                plot.points(
                    Points::new(PlotPoints::from(lost))
                        .radius(2.5)
                        .filled(true)
                        .color(RED)
                        .name(lost_name),
                );
            }
        });

    // The worst single ping each way, which the averages on the cards hide.
    ui.add_space(S_SM);
    if let (Some(max), true) = (r.loaded_max, r.loaded_avg.is_some()) {
        ui.label(
            egui::RichText::new(i18n::bloat_worst(max, r.loaded_loss_pct))
                .size(T_META)
                .color(FG_DIM),
        );
    }
    if let Some(up) = &r.upload {
        if let (Some(max), true) = (up.loaded_max, up.loaded_avg.is_some()) {
            ui.label(
                egui::RichText::new(i18n::bloat_worst_up(max, up.loaded_loss_pct))
                    .size(T_META)
                    .color(FG_DIM),
            );
        }
    }
}

/// A step in a list: its number in the accent, the text wrapping beside it.
fn numbered(ui: &mut egui::Ui, n: usize, text: &str) {
    ui.horizontal_top(|ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(18.0, T_BODY + 4.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_top() + egui::vec2(0.0, 1.0),
            egui::Align2::LEFT_TOP,
            format!("{n}."),
            egui::FontId::proportional(T_BODY),
            ACCENT,
        );
        ui.add(egui::Label::new(egui::RichText::new(text).size(T_BODY).color(FG)).wrap());
    });
    ui.add_space(S_XS);
}

/// Before the first run: what the test will do, and how it grades.
fn empty(ui: &mut egui::Ui) {
    card(ui, i18n::bloat_empty_title(), |ui| {
        for (i, step) in
            [i18n::bloat_empty_1(), i18n::bloat_empty_2(), i18n::bloat_empty_3()].iter().enumerate()
        {
            numbered(ui, i + 1, step);
        }
        ui.add_space(S_MD);
        ui.label(egui::RichText::new(i18n::bloat_empty_grade()).size(T_BODY).color(FG_DIM));
        ui.add_space(S_SM);
        scale(ui, Grade::Unknown);
    });
}

fn start(app: &mut App) {
    app.bloat_running = true;
    app.bloat_progress = 0.0;
    app.bloat_label = i18n::bloat_prog_idle().into();
    app.bloat_started = Some(Instant::now());
    app.bloat_cancel = Arc::default();
    // Our own probes would otherwise count as part of the load.
    app.monitor.hold();

    let tx = app.tx.clone();
    let timeout = app.settings.ping_timeout_ms;
    let cancel = Arc::clone(&app.bloat_cancel);
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress: bandwidth::Progress = Arc::new(move |label: &str, frac: f32| {
            let _ = progress_tx.send(Job::BloatProgress(label.to_string(), frac));
        });
        let res = bandwidth::run(
            std::net::Ipv4Addr::new(1, 1, 1, 1),
            IDLE,
            LOAD,
            timeout,
            Some(progress),
            &cancel,
        );
        let _ = tx.send(Job::BloatDone(Box::new(res)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_running_step_follows_the_progress_the_test_reports() {
        assert_eq!(phase_index(0.0), 0);
        assert_eq!(phase_index(bandwidth::PROGRESS_DOWN), 1);
        assert_eq!(phase_index(bandwidth::PROGRESS_UP), 2);
        assert_eq!(phase_index(1.0), 2);
    }

    #[test]
    fn the_summary_names_the_direction_the_ping_rose_most() {
        let _guard = i18n::test_lock();
        let mut r = BloatResult {
            idle_avg: Some(12.0),
            loaded_avg: Some(40.0),
            bump_ms: Some(28.0),
            upload: Some(bandwidth::Upload {
                loaded_avg: Some(90.0),
                bump_ms: Some(78.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        let up = summary(&r).unwrap_or_default();
        assert!(up.contains("90"), "{up}");
        r.upload = None;
        let down = summary(&r).unwrap_or_default();
        assert!(down.contains("40") && down != up, "{down}");
        r.idle_avg = None;
        assert!(summary(&r).is_none(), "no baseline, no claim about a rise");
    }
}
