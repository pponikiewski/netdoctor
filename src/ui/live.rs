//! Live tab: the latency plot and the headline numbers.

use eframe::egui;
use egui_plot::{HLine, Line, Plot, PlotBounds, PlotPoints, VLine};

use super::{btn_width, button, button_ex, is_narrow, latency_colour, stat_card, App, Emphasis, Job, ACCENT, FG, FG_DIM, GREEN, RED, SERIES_COLOURS, S_MD, S_SM, S_XS, T_BODY, T_META, T_MICRO, YELLOW};
use crate::i18n;
use crate::probe::icmp;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    // The tab is taller than a short window: a fixed-height plot, the cards,
    // a hop table whose length depends on the route, and the controls. Laid
    // out straight into the panel, whatever came last fell off the bottom
    // edge with nothing to say it was there. Scrolling costs nothing when
    // everything fits and is the difference between a hidden feature and a
    // visible one when it does not.
    egui::ScrollArea::vertical().id_salt("live_tab").auto_shrink([false, false]).show(ui, |ui| {
        body(app, ui);
    });
}

fn body(app: &mut App, ui: &mut egui::Ui) {
    plot(app, ui);
    ui.add_space(S_MD);
    cards(app, ui);
    ui.add_space(S_MD);
    // The controls stay above the hop table: they are the only things here a
    // person clicks, and a row of buttons that moves down the page whenever
    // the route grows a hop is a row of buttons nobody can find twice.
    controls(app, ui);
    ui.add_space(S_MD);
    // The hop table is five narrow columns and leaves the right half of the
    // window empty, which is exactly where a route dump wants to be: the two
    // answer the same question at different resolutions, and reading them
    // against each other is the point.
    if is_narrow(ui) {
        // Two columns in half a window is two unreadable columns. Stacked,
        // each one gets the width it needs and the page gets longer, which
        // is what scrolling is for.
        path_table(app, ui);
        ui.add_space(S_MD);
        trace_panel(app, ui);
    } else {
        ui.columns(2, |cols| {
            path_table(app, &mut cols[0]);
            trace_panel(app, &mut cols[1]);
        });
    }

    if !app.last.note.is_empty() || app.last.roamed {
        ui.add_space(S_SM);
        let mut note = app.last.note.clone();
        if app.last.roamed {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str(i18n::live_roamed());
        }
        egui::Frame::none()
            .fill(super::BG2)
            .rounding(6.0)
            .inner_margin(egui::Margin::same(S_MD))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(note).size(T_BODY).color(FG_DIM));
            });
    }
}

/// The legend's colour key, drawn to match the line it stands for.
///
/// This was a `▬` character tinted to the series colour, so its length and
/// weight came from the font rather than from the plot. A legend key should
/// look like a short piece of the line it names.
fn swatch(ui: &mut egui::Ui, colour: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 3.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 1.5, colour);
}

/// How long a window the chart covers, and what the buttons call it.
pub const RANGES: [f64; 4] = [60.0, 300.0, 900.0, 3600.0];

/// Roughly how many points per series the plot is given.
///
/// An hour at one sweep a second is 3600 readings per target; drawn straight
/// they are several thousand line segments a frame for a chart 900 pixels
/// wide, where most of them land on a pixel another one already covered.
/// Bucketing to about this many keeps every visible feature and stops the
/// frame time growing with the window the user picked.
const TARGET_POINTS: usize = 900;

/// One series, already reduced to what the plot will draw.
#[derive(Clone)]
pub struct ChartSeries {
    pub key: String,
    pub label: String,
    /// The drawn line. A `None` breaks it, whether because every probe in
    /// that slice was lost or because there was nothing recorded there at
    /// all; which of the two it was is in `outages`.
    pub points: Vec<(f64, Option<f64>)>,
    /// Buckets that lost some packets without losing most of them. A break in
    /// the line would overstate it and no mark at all would hide it, so it
    /// gets a mark of its own.
    pub losses: Vec<f64>,
    /// Breaks the probes are actually responsible for.
    ///
    /// Not every break is one. The chart reads the database, so it also
    /// covers stretches when the app was not running -- and those were drawn
    /// exactly like an outage, in red, which is the chart claiming a
    /// measurement it never took. Only these get the marker.
    pub outages: Vec<f64>,
}

/// The chart's data, held between frames.
///
/// The samples come from SQLite rather than from an in-memory ring, so the
/// window can be as long as the database and survives a restart. A query per
/// frame would be sixty of them a second, so it happens on the cadence
/// [`refresh_interval`] sets — at most as often as the data changes.
pub struct ChartCache {
    range_s: f64,
    smooth: bool,
    built_at: f64,
    pub series: Vec<ChartSeries>,
    spikes: crate::monitor::Spikes,
    newest: f64,
    oldest: f64,
}

/// How often the samples are fetched again, for a given window.
///
/// A one-second window of new data matters on a one-minute chart and is a
/// sixtieth of a pixel on an hour-long one — while the hour costs sixty times
/// as many rows to fetch. Scaling the interval with the window keeps the
/// query off the frame budget at every setting instead of only the cheap one.
fn refresh_interval(range_s: f64) -> f64 {
    (range_s / 300.0).clamp(1.0, 5.0)
}

/// Rebuilds the cache when the window, the smoothing or the clock says to.
fn refresh(app: &mut App) {
    let now = crate::store::now();
    let stale = match &app.chart_cache {
        Some(c) => {
            c.range_s != app.chart_range_s
                || c.smooth != app.chart_smooth
                || now - c.built_at >= refresh_interval(c.range_s)
        }
        None => true,
    };
    if !stale {
        return;
    }

    let range = app.chart_range_s;
    let rows = app.store.samples_between(now - range, now);

    let mut series = Vec::new();
    for t in app.settings.targets() {
        let raw: Vec<(f64, Option<f64>)> = rows
            .iter()
            .filter(|(_, target, _, _)| *target == t.key)
            .map(|(ts, _, rtt, ok)| (*ts, if *ok { *rtt } else { None }))
            .collect();
        if raw.is_empty() {
            continue;
        }
        let raw = mark_recording_gaps(raw);
        let (points, losses, outages) = reduce(&raw, range, app.chart_smooth);
        series.push(ChartSeries {
            key: t.key.clone(),
            label: t.label.clone(),
            points,
            losses,
            outages,
        });
    }

    // Spikes are counted on the raw readings, not on the reduced ones: a
    // bucket's maximum is a spike by construction, so counting after
    // reduction would report one for every bucket that contains any jitter.
    let raw_series: Vec<crate::monitor::Series> = app
        .settings
        .targets()
        .iter()
        .map(|t| {
            rows.iter()
                .filter(|(_, target, _, _)| *target == t.key)
                .map(|(ts, _, rtt, ok)| (*ts, if *ok { *rtt } else { None }))
                .collect()
        })
        .collect();
    let spikes = crate::monitor::find_spikes(&raw_series);

    let newest = rows.iter().map(|(ts, _, _, _)| *ts).fold(f64::NEG_INFINITY, f64::max);
    let oldest = rows.iter().map(|(ts, _, _, _)| *ts).fold(f64::INFINITY, f64::min);

    app.chart_cache = Some(ChartCache {
        range_s: range,
        smooth: app.chart_smooth,
        built_at: now,
        series,
        spikes,
        newest: if newest.is_finite() { newest } else { now },
        oldest: if oldest.is_finite() { oldest } else { now - range },
    });
}

/// Breaks the series wherever nothing was recorded for a while.
///
/// A lost packet leaves a row saying so. Time the app spent closed leaves no
/// row at all, and a line drawn straight from the last sample before the gap
/// to the first one after it asserts a steady latency across hours nobody
/// measured. The step is taken from the data rather than from the configured
/// interval, so it stays right when the interval is changed or the samples
/// come from an older run at a different cadence.
fn mark_recording_gaps(points: Vec<(f64, Option<f64>)>) -> Vec<(f64, Option<f64>)> {
    if points.len() < 3 {
        return points;
    }
    let mut deltas: Vec<f64> =
        points.windows(2).map(|w| w[1].0 - w[0].0).filter(|d| *d > 0.0).collect();
    if deltas.is_empty() {
        return points;
    }
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let step = deltas[deltas.len() / 2];
    // Three missed sweeps, and never less than a couple of seconds: ordinary
    // scheduling jitter must not be reported as the recorder stopping.
    let threshold = (step * 3.0).max(step + 2.0);

    let mut out = Vec::with_capacity(points.len() + 8);
    for (i, point) in points.iter().enumerate() {
        if i > 0 {
            let previous = points[i - 1].0;
            if point.0 - previous > threshold {
                out.push(((previous + point.0) * 0.5, None));
            }
        }
        out.push(*point);
    }
    out
}

/// Buckets a series down to something a chart can draw without lying about it.
///
/// Two modes, because they answer different questions. The envelope emits each
/// bucket's lowest and highest reading, in that order, so the drawn band is
/// the range the link actually covered in that slice — the same shape the
/// unsampled line would have had. Smoothed emits the mean, which throws the
/// spikes away on purpose: it is for reading the trend across an hour, and a
/// trend line with every outlier still attached is the thing nobody could
/// read in the first place.
type Reduced = (Vec<(f64, Option<f64>)>, Vec<f64>, Vec<f64>);

fn reduce(raw: &[(f64, Option<f64>)], range: f64, smooth: bool) -> Reduced {
    let mut losses = Vec::new();
    let mut outages = Vec::new();
    if raw.len() <= TARGET_POINTS && !smooth {
        // Unreduced, a `None` is either a lost probe or a gap this pass
        // inserted; only the first has a row of its own behind it, and a gap
        // inserted by `mark_recording_gaps` sits between two samples that are
        // further apart than the cadence. That is what tells them apart.
        for (i, (ts, v)) in raw.iter().enumerate() {
            if v.is_none() && i > 0 && i + 1 < raw.len() {
                let span = raw[i + 1].0 - raw[i - 1].0;
                if span < 4.0 {
                    outages.push(*ts);
                }
            }
        }
        return (raw.to_vec(), losses, outages);
    }

    let bucket_s = (range / TARGET_POINTS as f64).max(0.001);
    let mut out: Vec<(f64, Option<f64>)> = Vec::new();

    let mut i = 0;
    while i < raw.len() {
        let start = raw[i].0;
        let mut j = i;
        let mut vals: Vec<f64> = Vec::new();
        let mut lost = 0usize;
        while j < raw.len() && raw[j].0 - start < bucket_s {
            match raw[j].1 {
                Some(v) => vals.push(v),
                None => lost += 1,
            }
            j += 1;
        }
        let total = j - i;
        let mid = start + bucket_s * 0.5;

        if vals.is_empty() {
            // Nothing came back in this slice. A break either way, but only
            // a marker when probes were sent and went unanswered.
            out.push((mid, None));
            if lost > 0 {
                outages.push(mid);
            }
        } else if smooth {
            out.push((mid, Some(vals.iter().sum::<f64>() / vals.len() as f64)));
        } else {
            let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            out.push((start, Some(lo)));
            if hi > lo {
                out.push((start + bucket_s * 0.999, Some(hi)));
            }
        }

        // Losing some of a slice is not the same as losing all of it. Half or
        // more reads as an outage and breaks the line; less than half keeps
        // the line and gets a mark, because at an hour's zoom a single lost
        // packet would otherwise shred the chart into fragments.
        if vals.is_empty() {
            // already handled above
        } else if lost * 2 >= total {
            out.push((mid, None));
            outages.push(mid);
        } else if lost > 0 {
            losses.push(mid);
        }

        i = j;
    }

    (out, losses, outages)
}

fn plot(app: &mut App, ui: &mut egui::Ui) {
    refresh(app);

    range_controls(app, ui);
    ui.add_space(S_SM);

    let Some(cache) = app.chart_cache.as_ref() else {
        return;
    };
    let newest = cache.newest;
    let spikes = cache.spikes.clone();

    // Hidden series are dropped here rather than skipped while drawing, so
    // the scale, the spike markers and the readout all agree with what is on
    // screen: a y axis set by a line nobody can see is a scale with no
    // explanation.
    let visible: Vec<(ChartSeries, egui::Color32)> = cache
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| !app.hidden_series.contains(&s.key))
        .map(|(i, s)| (s.clone(), SERIES_COLOURS[i % SERIES_COLOURS.len()]))
        .collect();
    let all: Vec<(String, String, egui::Color32)> = cache
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| (s.key.clone(), s.label.clone(), SERIES_COLOURS[i % SERIES_COLOURS.len()]))
        .collect();

    let (y_top, above) = scale(&visible, &app.settings);
    let x_min = cache.oldest - newest;
    let x_min = if x_min < -1.0 { x_min } else { -app.chart_range_s };

    // The plot is the tab's centrepiece, so it takes a share of whatever
    // height there is rather than a fixed 280 px that is most of a laptop
    // screen and a fifth of a desktop one.
    let plot_height = (ui.ctx().screen_rect().height() * 0.32).clamp(170.0, 420.0);

    // No delay before the readout appears. egui waits a third of a second
    // before a tooltip, which is right for a hint attached to a button and
    // wrong for a value that is meant to track the pointer: by the time it
    // arrived the pointer had moved, so it felt like lag rather than like
    // reading the chart.
    ui.style_mut().interaction.tooltip_delay = 0.0;
    ui.style_mut().interaction.tooltip_grace_time = 0.0;

    let plotted = Plot::new("latency")
        .height(plot_height)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .show_axes([true, true])
        // No rotated "ms" down the side: two letters turned on end, drawn
        // hard against the tick labels, collided with them and bought
        // nothing. The unit is said once in the caption instead.
        .x_axis_formatter(|mark, _| {
            // x is seconds relative to now, so label it as age. Rounding to
            // whole minutes past 90 s printed "-2m" on six consecutive ticks,
            // which is not a time axis -- it is the same word six times. Past
            // a minute the labels are m:ss, so every tick is its own moment.
            let back = -mark.value;
            if back < 1.0 {
                i18n::live_x_now().to_string()
            } else if back < 60.0 {
                format!("-{back:.0}s")
            } else {
                let mins = (back / 60.0).floor();
                let secs = (back - mins * 60.0).round();
                format!("-{mins:.0}:{secs:02.0}")
            }
        })
        .label_formatter(|_, _| String::new())
        .show(ui, |plot_ui| {
            plot_ui.set_plot_bounds(PlotBounds::from_min_max([x_min, 0.0], [0.0, y_top]));

            for (level, colour) in
                [(app.settings.ping_ok_ms, GREEN), (app.settings.ping_bad_ms, RED)]
            {
                if level > 0.0 && level < y_top {
                    plot_ui.hline(HLine::new(level).color(colour.linear_multiply(0.25)));
                }
            }

            for (ts, n) in &spikes.correlated {
                let strength = if *n >= visible.len().max(2) { 0.42 } else { 0.22 };
                plot_ui.vline(VLine::new(ts - newest).color(YELLOW.linear_multiply(strength)));
            }

            let hovered = plot_ui.pointer_coordinate();
            if let Some(h) = hovered {
                plot_ui.vline(VLine::new(h.x).color(FG_DIM.linear_multiply(0.30)));
            }

            for (s, colour) in &visible {
                for ts in &s.losses {
                    plot_ui.vline(VLine::new(ts - newest).color(RED.linear_multiply(0.30)));
                }
                for ts in &s.outages {
                    plot_ui.vline(VLine::new(ts - newest).color(RED.linear_multiply(0.6)));
                }

                // Split at gaps so a lost packet breaks the line instead of
                // drawing a straight segment across the outage.
                let mut run: Vec<[f64; 2]> = Vec::new();
                for (ts, rtt) in &s.points {
                    let x = ts - newest;
                    match rtt {
                        Some(v) => run.push([x, *v]),
                        None => {
                            if run.len() > 1 {
                                plot_ui.line(
                                    Line::new(PlotPoints::from(std::mem::take(&mut run)))
                                        .color(*colour)
                                        .width(1.6)
                                        .name(&s.label),
                                );
                            } else {
                                run.clear();
                            }
                        }
                    }
                }
                if run.len() > 1 {
                    plot_ui.line(
                        Line::new(PlotPoints::from(run)).color(*colour).width(1.6).name(&s.label),
                    );
                }
            }

            hovered
        });

    if let Some(at) = plotted.inner {
        // Repaint while the pointer is over the chart. Without it the readout
        // only moves when the next sweep lands, which at a second a sweep
        // looks like the crosshair sticking to the last place it was.
        ui.ctx().request_repaint();
        let x = at.x;
        plotted.response.on_hover_ui(|ui| {
            readout(ui, &visible, &spikes, newest, x);
        });
    }

    // Row one: what each line is, and a switch for it. Row two: what the
    // markings mean. They used to share a row, with the key pushed right
    // against the window edge by a right-to-left layout while the series
    // names grew from the left -- the two collided the moment a target had a
    // long name. Two rows cost eight pixels and cannot collide.
    let mut toggled: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        for (key, label, colour) in &all {
            let on = !app.hidden_series.contains(key);
            let shown = if on { *colour } else { FG_DIM.linear_multiply(0.4) };
            let resp = ui
                .scope(|ui| {
                    ui.spacing_mut().item_spacing.x = S_XS;
                    ui.horizontal(|ui| {
                        swatch(ui, shown);
                        ui.label(
                            egui::RichText::new(label)
                                .size(T_BODY)
                                .color(if on { FG_DIM } else { FG_DIM.linear_multiply(0.45) }),
                        );
                    })
                    .response
                })
                .inner
                .interact(egui::Sense::click());

            // Four lines crossing each other is the state this chart is in
            // most of the time, and the question is usually about one of
            // them. Clicking its name takes the rest away.
            if resp.clicked() {
                toggled = Some(key.clone());
            }
            let tip = match app.last.results.get(key) {
                Some(sample) => match (&sample.rtt_ms, &sample.error) {
                    (Some(rtt), _) => format!("{label}: {rtt:.2} ms"),
                    (None, Some(err)) => format!("{label}: {err}"),
                    (None, None) => format!("{label}: {}", i18n::live_no_data()),
                },
                None => i18n::live_series_toggle().to_string(),
            };
            resp.on_hover_text(format!("{tip}\n{}", i18n::live_series_toggle()));
            ui.add_space(S_MD);
        }
    });

    ui.add_space(S_XS);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = S_SM;
        ui.label(egui::RichText::new(i18n::live_axis_unit()).size(T_MICRO).color(FG_DIM));
        dot(ui);
        ui.label(egui::RichText::new(i18n::live_red_line()).size(T_MICRO).color(FG_DIM));
        dot(ui);
        ui.label(egui::RichText::new(i18n::live_amber_line()).size(T_MICRO).color(FG_DIM))
            .on_hover_text(i18n::live_spike_explainer());
        dot(ui);
        ui.label(egui::RichText::new(i18n::live_scale_ok()).size(T_MICRO).color(GREEN));
        ui.label(egui::RichText::new("/").size(T_MICRO).color(FG_DIM));
        ui.label(egui::RichText::new(i18n::live_scale_bad()).size(T_MICRO).color(RED));
        dot(ui);
        ui.label(
            egui::RichText::new(i18n::live_hover_hint()).size(T_MICRO).italics().color(FG_DIM),
        );
        if above > 0 {
            dot(ui);
            ui.label(
                egui::RichText::new(i18n::live_above_scale(above, y_top))
                    .size(T_MICRO)
                    .color(YELLOW),
            );
        }
    });

    // The count is the part that reframes the chart: it says in one line how
    // much of what looks like instability was never about the connection. It
    // keeps its own line because it is a sentence, not a key.
    if spikes.single + spikes.correlated.len() > 0 {
        ui.add_space(S_XS);
        ui.label(
            egui::RichText::new(i18n::live_spike_tally(spikes.correlated.len(), spikes.single))
                .size(T_META)
                .color(FG_DIM),
        )
        .on_hover_text(i18n::live_spike_explainer());
    }

    if let Some(key) = toggled {
        if !app.hidden_series.remove(&key) {
            app.hidden_series.insert(key);
        }
    }
}

/// How far back the chart looks, and whether it draws the range or the trend.
fn range_controls(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = S_XS;
        ui.label(egui::RichText::new(i18n::live_range_label()).size(T_MICRO).color(FG_DIM));
        ui.add_space(S_XS);
        for secs in RANGES {
            let on = (app.chart_range_s - secs).abs() < 0.5;
            if ui.selectable_label(on, egui::RichText::new(i18n::range_name(secs)).size(T_META)).clicked() {
                app.chart_range_s = secs;
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Two readings of the same data. The envelope is what happened;
            // the trend is what it averaged out to. An hour of a jittery link
            // is unreadable as the former and meaningless as the latter, so
            // neither one can be the only option.
            if ui
                .selectable_label(
                    app.chart_smooth,
                    egui::RichText::new(i18n::live_smooth()).size(T_META),
                )
                .on_hover_text(i18n::live_smooth_hint())
                .clicked()
            {
                app.chart_smooth = !app.chart_smooth;
            }
        });
    });
}

/// The separator between two items in a key row: a painted dot, not a
/// character, so it keeps its size and colour whatever the font does with a
/// middot and never reads as punctuation belonging to the words beside it.
fn dot(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, T_META), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 1.5, FG_DIM.linear_multiply(0.6));
}

/// The top of the y axis, and how many readings are left above it.
///
/// The 95th percentile sets it, so the scale follows the data people actually
/// have rather than its worst moment. The floor stops a very fast link from
/// being drawn at enormous magnification, where ordinary jitter looks like a
/// catastrophe.
///
/// The "poor" threshold deliberately does *not* get a vote. Forcing it onto
/// the axis sounds principled and ruins the chart: on a 12 ms link it pins
/// the top at 138 ms and draws the entire connection as a flat line along
/// the bottom, which is the problem this function exists to solve. The
/// threshold lines are drawn when they fall inside the scale and are simply
/// absent when the link is nowhere near them -- which is itself the answer to
/// "is that bad".
fn scale(
    series: &[(ChartSeries, egui::Color32)],
    s: &crate::settings::Settings,
) -> (f64, usize) {
    let mut vals: Vec<f64> =
        series.iter().flat_map(|(d, _)| d.points.iter().filter_map(|(_, r)| *r)).collect();
    if vals.is_empty() {
        return (s.ping_ok_ms.max(50.0), 0);
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95 = vals[(vals.len() * 95 / 100).min(vals.len() - 1)];

    let top = (p95 * 1.5).max(s.ping_good_ms * 0.8).max(20.0);
    // Round up to something a person would choose, so the gridline labels are
    // whole numbers rather than 63.7 and 127.4.
    let step = if top <= 50.0 {
        10.0
    } else if top <= 200.0 {
        25.0
    } else {
        100.0
    };
    let top = (top / step).ceil() * step;

    let above = vals.iter().filter(|v| **v > top).count();
    (top, above)
}

/// Every probe's value at the moment under the pointer.
///
/// A latency chart with several lines on it answers "was something slow" at a
/// glance and "which of them, and by how much" not at all -- the lines cross,
/// the colours are three pixels wide, and the eye cannot read a value off a
/// y axis to within a millisecond. Naming all of them at one instant is also
/// what separates the two readings that matter: every line jumping together
/// is the connection, one line jumping alone is that responder.
fn readout(
    ui: &mut egui::Ui,
    series: &[(ChartSeries, egui::Color32)],
    spikes: &crate::monitor::Spikes,
    newest: f64,
    x: f64,
) {
    // The pointer lands between samples, so each series answers with its
    // closest one. Anything further away than this is not an answer about
    // that moment, and saying nothing beats inventing a value.
    const NEAREST_S: f64 = 3.0;

    let ts = newest + x;
    ui.label(
        egui::RichText::new(i18n::live_hover_when(&crate::diagnose::format_clock(ts), -x))
            .size(T_META)
            .strong()
            .color(FG),
    );
    ui.add_space(S_XS);

    for (s, colour) in series {
        let nearest = s
            .points
            .iter()
            .min_by(|a, b| {
                let da = (a.0 - newest - x).abs();
                let db = (b.0 - newest - x).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .filter(|(sample_ts, _)| (sample_ts - newest - x).abs() <= NEAREST_S);

        let Some((_, rtt)) = nearest else { continue };
        ui.horizontal(|ui| {
            swatch(ui, *colour);
            ui.add_space(S_XS);
            ui.label(egui::RichText::new(&s.label).size(T_META).color(FG_DIM));
            match rtt {
                Some(v) => ui.label(super::figure(format!("{v:.1} ms"), T_META, FG)),
                None => ui.label(
                    egui::RichText::new(i18n::live_hover_lost()).size(T_META).strong().color(RED),
                ),
            };
        });
    }

    // Whether this instant is one of the moments the app already decided was
    // evidence about the link rather than noise from one responder.
    if spikes.correlated.iter().any(|(spike_ts, _)| (spike_ts - ts).abs() <= NEAREST_S) {
        ui.add_space(S_XS);
        ui.label(egui::RichText::new(i18n::live_hover_spike()).size(T_META).color(YELLOW));
    }
}

fn cards(app: &mut App, ui: &mut egui::Ui) {
    let s = &app.settings;
    let cf = app.store.stats("cloudflare", 300.0);
    let gw = app.store.stats("gateway", 300.0);

    // Six cards at 140 px plus their margins need about 1100 px. Laid out in
    // a plain horizontal row, the ones past the edge were simply clipped --
    // and the last of them is the outage counter, which is the one worth
    // reading. Wrapping costs a second row and loses nothing.
    ui.horizontal_wrapped(|ui| {
        match cf.avg {
            Some(avg) => stat_card(
                ui,
                i18n::live_card_latency(),
                &format!("{avg:.0} ms"),
                &i18n::live_minmax(cf.min.unwrap_or(0.0), cf.max.unwrap_or(0.0)),
                latency_colour(avg, s),
            ),
            None => stat_card(ui, i18n::live_card_latency(), "—", i18n::live_card_latency_sub_none(), FG_DIM),
        }

        match cf.jitter {
            Some(j) => {
                let colour = if j < s.jitter_good_ms {
                    GREEN
                } else if j < s.jitter_ok_ms {
                    YELLOW
                } else {
                    RED
                };
                stat_card(ui, i18n::live_card_jitter(), &format!("{j:.1} ms"), i18n::live_card_jitter_sub(), colour)
            }
            None => stat_card(ui, i18n::live_card_jitter(), "—", "", FG_DIM),
        }

        let loss_colour = if cf.loss_pct < s.loss_good_pct {
            GREEN
        } else if cf.loss_pct < s.loss_ok_pct {
            YELLOW
        } else {
            RED
        };
        stat_card(
            ui,
            i18n::live_card_loss(),
            &format!("{:.1}%", cf.loss_pct),
            i18n::live_card_loss_sub(),
            loss_colour,
        );

        match gw.avg {
            Some(avg) => stat_card(
                ui,
                i18n::live_card_router(),
                &format!("{avg:.1} ms"),
                &i18n::live_router_loss(gw.loss_pct),
                if avg < 10.0 { GREEN } else { YELLOW },
            ),
            None => stat_card(ui, i18n::live_card_router(), "—", i18n::live_card_router_none(), RED),
        }

        if !app.last.dns_error.is_empty() {
            stat_card(ui, i18n::live_card_dns(), i18n::live_card_dns_err(), &app.last.dns_error, RED);
        } else {
            match app.last.dns_ms {
                Some(ms) => stat_card(
                    ui,
                    i18n::live_card_dns(),
                    &format!("{ms:.0} ms"),
                    i18n::live_card_dns_sub(),
                    if ms < 60.0 { GREEN } else if ms < 150.0 { YELLOW } else { RED },
                ),
                None => stat_card(ui, i18n::live_card_dns(), "—", "", FG_DIM),
            }
        }

        let events = app.store.events_since(24.0 * 3600.0);
        if events.is_empty() {
            stat_card(
                ui,
                i18n::live_card_uninterrupted(),
                i18n::live_card_uninterrupted_val(),
                i18n::live_card_uninterrupted_sub(),
                GREEN,
            );
        } else {
            let last = events.iter().map(|e| e.ts_start).fold(f64::NEG_INFINITY, f64::max);
            let mins = (crate::store::now() - last) / 60.0;
            stat_card(
                ui,
                i18n::live_card_since_outage(),
                &format!("{mins:.0} min"),
                &i18n::live_outages_24h(events.len()),
                if events.len() < 3 { YELLOW } else { RED },
            );
        }
    });
}

/// The path, hop by hop, with the verdict on top.
///
/// The plot above answers "is it bad"; this answers "whose". A verdict here
/// is deliberately rarer than a red figure in the table — see
/// [`crate::probe::path::blame`] for why a single lossy hop is usually a
/// router protecting itself rather than a fault.
fn path_table(app: &mut App, ui: &mut egui::Ui) {
    let reading = app.monitor.path();

    ui.label(egui::RichText::new(i18n::live_path_heading()).size(T_BODY).strong().color(FG_DIM));
    ui.add_space(S_XS);

    if reading.hops.is_empty() {
        ui.label(egui::RichText::new(i18n::live_path_waiting()).size(T_META).color(FG_DIM));
        return;
    }

    match &reading.blame {
        Some(b) => {
            let owner = i18n::path_owner(b.owner);
            let headline = match b.added_ms {
                Some(added) => {
                    i18n::path_blame_delay(b.ttl, &b.addr.to_string(), added, owner)
                }
                None => i18n::path_blame_loss(b.ttl, &b.addr.to_string(), b.loss_pct, owner),
            };
            ui.label(egui::RichText::new(headline).size(T_BODY).strong().color(RED));
            ui.label(
                egui::RichText::new(i18n::path_blame_advice(b.owner.is_mine()))
                    .size(T_META)
                    .color(FG_DIM),
            );
        }
        None => {
            ui.label(egui::RichText::new(i18n::live_path_clean()).size(T_BODY).color(GREEN));
        }
    }

    ui.add_space(S_SM);
    egui::Grid::new("path_hops").num_columns(5).striped(true).spacing([14.0, 3.0]).show(
        ui,
        |ui| {
            for h in [
                i18n::live_path_col_hop(),
                i18n::live_path_col_addr(),
                i18n::live_path_col_owner(),
                i18n::live_path_col_loss(),
                i18n::live_path_col_avg(),
            ] {
                ui.label(egui::RichText::new(h).size(T_META).color(FG_DIM));
            }
            ui.end_row();

            let blamed = reading.blame.as_ref().map(|b| b.ttl);
            for hop in &reading.hops {
                let accused = blamed == Some(hop.ttl);
                let name_colour = if accused { RED } else { FG_DIM };
                // Loss is coloured on its own merits, so a hop that is losing
                // packets still reads as losing them even when the verdict
                // above declined to blame it for anything. A silent hop is
                // the exception: it has no loss figure to colour, because it
                // was never measured.
                let loss_colour = match hop.loss_pct {
                    _ if hop.silent => FG_DIM,
                    l if l >= 8.0 => RED,
                    l if l > 0.0 => YELLOW,
                    _ => FG_DIM,
                };

                ui.label(super::figure(hop.ttl.to_string(), T_META, name_colour));
                ui.label(
                    egui::RichText::new(hop.addr.to_string())
                        .size(T_META)
                        .monospace()
                        .color(if accused { RED } else { FG_DIM }),
                );
                ui.label(
                    egui::RichText::new(i18n::path_owner(hop.owner)).size(T_META).color(FG_DIM),
                );
                if hop.silent {
                    ui.label(
                        egui::RichText::new(i18n::path_no_answer())
                            .size(T_META)
                            .italics()
                            .color(FG_DIM),
                    );
                    ui.label(egui::RichText::new("").size(T_META));
                } else {
                    ui.label(super::figure(format!("{:.0}%", hop.loss_pct), T_META, loss_colour));
                    ui.label(super::figure(
                        match hop.avg_ms {
                            Some(v) => format!("{v:.0} ms"),
                            None => "—".into(),
                        },
                        T_META,
                        latency_colour(hop.avg_ms.unwrap_or(0.0), &app.settings),
                    ));
                }
                ui.end_row();
            }
        },
    );

    ui.add_space(S_XS);
    ui.label(egui::RichText::new(i18n::live_path_note()).size(T_META).italics().color(FG_DIM));
}

fn controls(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        // A toggle whose two labels are different lengths resizes itself on
        // every click, and everything to its right slides with it. Reserve the
        // width of the longer label and the row holds still.
        let paused = app.monitor.is_paused();
        let toggle_label = if paused { i18n::live_btn_resume() } else { i18n::live_btn_pause() };
        let toggle_w =
            btn_width(ui, i18n::live_btn_pause()).max(btn_width(ui, i18n::live_btn_resume()));
        if button_ex(ui, toggle_label, Emphasis::Secondary, true, toggle_w).clicked() {
            app.monitor.set_paused(!paused);
        }

        // Traceroute is the only control here that goes and finds out
        // something the page is not already showing, so it carries the row.
        // While it runs it says so on its own face rather than greying out
        // with the same label and leaving the user to guess whether the click
        // registered.
        let trace_label = if app.tracing { i18n::live_tracing() } else { i18n::live_btn_trace() };
        let trace_w =
            btn_width(ui, i18n::live_btn_trace()).max(btn_width(ui, i18n::live_tracing()));
        if button_ex(ui, trace_label, Emphasis::Primary, !app.tracing, trace_w).clicked() {
            app.tracing = true;
            app.trace = vec![i18n::live_tracing().into()];
            let tx = app.tx.clone();
            std::thread::spawn(move || {
                let hops = icmp::traceroute(std::net::Ipv4Addr::new(1, 1, 1, 1), 20, 1000);
                let mut lines: Vec<String> = hops
                    .iter()
                    .map(|h| match h.addr {
                        Some(a) => format!(
                            "{:>2}  {:<16} {}",
                            h.hop,
                            a,
                            h.rtt_ms.map(|v| format!("{v:.1} ms")).unwrap_or_else(|| "*".into())
                        ),
                        None => format!("{:>2}  {:<16} *", h.hop, "*"),
                    })
                    .collect();
                lines.push(String::new());
                lines.push(i18n::live_trace_note_1().into());
                lines.push(i18n::live_trace_note_2().into());
                let _ = tx.send(Job::Traceroute(lines));
            });
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // An export. It should be findable and not much more — it is not
            // what anyone opened the live tab to do.
            if button(ui, i18n::live_btn_report(), Emphasis::Ghost).clicked() {
                match super::report::save(app) {
                    Ok(path) => {
                        let now = ui.input(|i| i.time);
                        app.toast(i18n::live_report_saved(&path), GREEN, now);
                    }
                    Err(e) => {
                        let now = ui.input(|i| i.time);
                        app.toast(i18n::set_save_failed(&e.to_string()), RED, now);
                    }
                }
            }
        });
    });

}

/// The one-off route dump, beside the continuous hop table rather than under
/// the buttons.
///
/// Sitting below the controls it pushed everything after it down the page
/// every time it was run, and it was the widest thing on the tab in the
/// narrowest place. Here it fills the space the hop table leaves and stays
/// where it is whether it has been run or not.
fn trace_panel(app: &App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(i18n::live_trace_heading()).size(T_BODY).strong().color(FG_DIM));
    ui.add_space(S_XS);

    if app.trace.is_empty() {
        ui.label(egui::RichText::new(i18n::live_trace_empty()).size(T_META).color(FG_DIM));
        return;
    }

    egui::Frame::none()
        .fill(super::BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(S_MD))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("trace_output")
                .max_height(260.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &app.trace {
                        ui.label(egui::RichText::new(line).monospace().size(T_META).color(ACCENT));
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn series(values: &[f64]) -> Vec<(ChartSeries, egui::Color32)> {
        vec![(
            ChartSeries {
                key: "t".into(),
                label: "t".into(),
                points: values.iter().enumerate().map(|(i, v)| (i as f64, Some(*v))).collect(),
                losses: Vec::new(),
                outages: Vec::new(),
            },
            egui::Color32::WHITE,
        )]
    }

    #[test]
    fn one_outlier_does_not_set_the_whole_scale() {
        // The bug this guards: a single 400 ms reply set the top of the axis
        // and drew a 12 ms baseline as a flat line along the bottom.
        let s = Settings::default();
        let calm = vec![12.0; 200];
        let mut spiked = calm.clone();
        spiked.push(400.0);

        let (quiet_top, _) = scale(&series(&calm), &s);
        let (top, above) = scale(&series(&spiked), &s);

        assert_eq!(top, quiet_top, "one outlier must not move the axis at all");
        assert!(top < 60.0, "a 12 ms link is drawn at 12 ms scale, not at its worst moment: {top}");
        assert_eq!(above, 1, "and the outlier is counted rather than quietly dropped");
    }

    #[test]
    fn a_fast_link_is_not_flattened_to_fit_a_threshold() {
        // Making room for the 120 ms "poor" line would draw a 3 ms link as a
        // line along the bottom of the chart, which is the whole complaint.
        let s = Settings::default();
        let (top, above) = scale(&series(&[3.0; 100]), &s);
        assert!(top < s.ping_ok_ms, "got {top}, which is mostly empty chart");
        assert_eq!(above, 0);
    }

    #[test]
    fn a_link_that_is_genuinely_slow_gets_a_scale_that_shows_it() {
        let s = Settings::default();
        let (top, _) = scale(&series(&[140.0; 100]), &s);
        assert!(top > s.ping_bad_ms, "the thresholds have to be visible once they matter: {top}");
    }

    #[test]
    fn the_top_of_the_scale_is_a_number_a_person_would_pick() {
        let s = Settings::default();
        for sample in [7.0, 63.0, 180.0, 640.0] {
            let (top, _) = scale(&series(&[sample; 100]), &s);
            let step = if top <= 50.0 {
                10.0
            } else if top <= 200.0 {
                25.0
            } else {
                100.0
            };
            assert!((top % step).abs() < 1e-9, "{top} is not a multiple of {step}");
        }
    }

    #[test]
    fn a_window_with_nothing_in_it_still_has_a_scale() {
        let s = Settings::default();
        let (top, above) = scale(&[], &s);
        assert!(top > 0.0, "an empty plot needs an axis, not a division by zero");
        assert_eq!(above, 0);
    }

    #[test]
    fn total_loss_leaves_no_readings_to_scale_from() {
        let empty: Vec<(ChartSeries, egui::Color32)> = vec![(
            ChartSeries {
                key: "t".into(),
                label: "t".into(),
                points: vec![(0.0, None), (1.0, None)],
                losses: Vec::new(),
                outages: Vec::new(),
            },
            egui::Color32::WHITE,
        )];
        let (top, above) = scale(&empty, &Settings::default());
        assert!(top > 0.0);
        assert_eq!(above, 0);
    }
}

#[cfg(test)]
mod reduce_tests {
    use super::*;

    fn ramp(n: usize, value: f64) -> Vec<(f64, Option<f64>)> {
        (0..n).map(|i| (i as f64, Some(value))).collect()
    }

    #[test]
    fn a_short_window_is_drawn_exactly_as_measured() {
        let raw = ramp(120, 12.0);
        let (points, losses, _) = reduce(&raw, 120.0, false);
        assert_eq!(points, raw, "nothing to gain by bucketing what already fits");
        assert!(losses.is_empty());
    }

    #[test]
    fn an_hour_is_cut_down_but_keeps_its_extremes() {
        // A flat 12 ms line with one 300 ms spike buried in the middle: the
        // spike is the only thing on this chart worth seeing, and a naive
        // every-nth-sample reduction throws it away most of the time.
        let mut raw = ramp(3600, 12.0);
        raw[1800] = (1800.0, Some(300.0));

        let (points, _, _) = reduce(&raw, 3600.0, false);
        assert!(points.len() < 2200, "an hour has to cost less than an hour: {}", points.len());

        let peak = points.iter().filter_map(|(_, v)| *v).fold(0.0, f64::max);
        assert_eq!(peak, 300.0, "the spike survived the reduction");
        let floor = points.iter().filter_map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
        assert_eq!(floor, 12.0, "and so did the baseline it stood out from");
    }

    #[test]
    fn the_trend_view_drops_the_spike_on_purpose() {
        let mut raw = ramp(3600, 12.0);
        raw[1800] = (1800.0, Some(300.0));

        let (points, _, _) = reduce(&raw, 3600.0, true);
        let peak = points.iter().filter_map(|(_, v)| *v).fold(0.0, f64::max);
        assert!(peak < 100.0, "a mean is not a maximum: {peak}");
        assert!(peak > 12.0, "but the spike still moved the average it landed in");
    }

    #[test]
    fn a_slice_that_lost_everything_breaks_the_line() {
        let mut raw = ramp(1200, 12.0);
        for r in raw.iter_mut().take(700).skip(600) {
            r.1 = None;
        }
        let (points, losses, outages) = reduce(&raw, 1200.0, false);
        assert!(points.iter().any(|(_, v)| v.is_none()), "a full outage is a gap");
        assert!(losses.is_empty(), "a gap is not also a stray-loss mark");
        assert!(!outages.is_empty(), "and it is the kind of gap worth marking");
    }

    #[test]
    fn one_lost_packet_in_an_hour_is_marked_and_not_a_gap() {
        // The reason this is not simply "any loss breaks the line": at an
        // hour's zoom that shreds an otherwise healthy chart into fragments.
        let mut raw = ramp(3600, 12.0);
        raw[1234].1 = None;

        let (points, losses, _) = reduce(&raw, 3600.0, false);
        assert_eq!(losses.len(), 1, "it is marked");
        assert!(!points.iter().any(|(_, v)| v.is_none()), "and the line stays whole");
    }

    #[test]
    fn time_the_app_was_closed_breaks_the_line_and_is_not_blamed_on_the_link() {
        // The chart reads the database, so it covers stretches when nothing
        // was running. Drawn straight through, an hour nobody measured became
        // a flat line asserting a steady latency -- and marked red, it became
        // an outage the app invented.
        let mut raw: Vec<(f64, Option<f64>)> = (0..60).map(|i| (i as f64, Some(12.0))).collect();
        raw.extend((0..60).map(|i| (600.0 + i as f64, Some(12.0))));

        let marked = mark_recording_gaps(raw);
        let (points, _, outages) = reduce(&marked, 660.0, false);

        assert!(points.iter().any(|(_, v)| v.is_none()), "the line has to break across it");
        assert!(outages.is_empty(), "nothing was lost there; nothing was even asked");
    }

    #[test]
    fn ordinary_jitter_in_the_sweep_is_not_a_recording_gap() {
        let raw: Vec<(f64, Option<f64>)> =
            (0..60).map(|i| (i as f64 * 1.0 + (i % 3) as f64 * 0.2, Some(12.0))).collect();
        let marked = mark_recording_gaps(raw.clone());
        assert_eq!(marked.len(), raw.len(), "a sweep that runs late has not stopped");
    }

    #[test]
    fn a_lost_probe_is_still_reported_as_one() {
        let mut raw: Vec<(f64, Option<f64>)> = (0..60).map(|i| (i as f64, Some(12.0))).collect();
        raw[30].1 = None;
        let marked = mark_recording_gaps(raw);
        let (_, _, outages) = reduce(&marked, 60.0, false);
        assert_eq!(outages.len(), 1, "this one the probes are responsible for");
    }

    #[test]
    fn a_long_window_is_fetched_less_often_than_a_short_one() {
        assert_eq!(refresh_interval(60.0), 1.0);
        assert_eq!(refresh_interval(300.0), 1.0);
        assert!(refresh_interval(3600.0) > refresh_interval(300.0));
        assert!(refresh_interval(3600.0) <= 5.0, "never so rare that the chart looks frozen");
    }
}
