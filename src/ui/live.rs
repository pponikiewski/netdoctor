//! Live tab: the latency plot and the headline numbers.

use eframe::egui;
use egui_plot::{HLine, Line, Plot, PlotBounds, PlotPoint, PlotPoints, Text, VLine};

use super::{
    btn_width, button, button_ex, figure, is_narrow, latency_colour, App, Emphasis, Job, ACCENT,
    BG2, BG3, FG, FG_DIM, GREEN, LINE, RED, SERIES_COLOURS, S_MD, S_SM, S_XS, T_BODY, T_HEAD,
    T_META, T_MICRO, YELLOW,
};
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

    if !app.last.note.is_empty() || app.last.roamed || app.last.blind.is_some() {
        ui.add_space(S_SM);
        let mut note = app.last.blind.clone().unwrap_or_else(|| app.last.note.clone());
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

/// One legend entry, drawn as a chip you can tell is a control.
///
/// It was a swatch and a word, which is what a legend looks like when it does
/// nothing. This one is clickable, and everything about it now says so: a
/// surface with an edge, a hover state, the pointer changing, and the target's
/// current reading carried inside it so the chip is worth looking at even when
/// nobody intends to click. Hidden series keep their place and lose their
/// colour, because a legend that removes its own entries cannot be used to put
/// them back.
fn legend_chip(
    ui: &mut egui::Ui,
    colour: egui::Color32,
    label: &str,
    value: Option<(String, egui::Color32)>,
    on: bool,
) -> egui::Response {
    // Reserved before the content so the background lands underneath it: the
    // fill depends on the hover state, which is not known until the content
    // has been laid out and the rect exists.
    let bg = ui.painter().add(egui::Shape::Noop);

    let inner = egui::Frame::none().inner_margin(egui::Margin::symmetric(S_SM, S_XS + 1.0)).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = S_XS + 2.0;
            ui.horizontal(|ui| {
                swatch(ui, if on { colour } else { FG_DIM.linear_multiply(0.35) });
                let text = egui::RichText::new(label).size(T_BODY).color(if on {
                    FG
                } else {
                    FG_DIM.linear_multiply(0.55)
                });
                ui.label(if on { text } else { text.strikethrough() });
                if let Some((v, c)) = value {
                    let c = if on { c } else { FG_DIM.linear_multiply(0.45) };
                    ui.label(figure(v, T_META, c));
                }
            });
        },
    );

    let resp = inner.response.interact(egui::Sense::click());
    let (fill, stroke) = if resp.hovered() {
        (BG3, egui::Stroke::new(1.0_f32, colour.linear_multiply(0.75)))
    } else if on {
        (BG2, egui::Stroke::new(1.0_f32, LINE))
    } else {
        (egui::Color32::TRANSPARENT, egui::Stroke::new(1.0_f32, LINE))
    };
    ui.painter().set(bg, egui::epaint::RectShape::new(resp.rect, 7.0, fill, stroke));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    /// Which stretch of the path this target measures.
    ///
    /// Carried on the series rather than looked up when the legend is drawn.
    /// `Settings::targets()` resolves every hostname the user added, which is
    /// a blocking DNS lookup, and the legend is drawn on every frame: at the
    /// one moment the tab repaints continuously, when the pointer is over the
    /// chart, that was a resolver query per frame from the render thread. The
    /// cache is rebuilt every second or five, which is where that belongs.
    pub scope: crate::settings::Scope,
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

    // Resolved once per rebuild. Each call re-resolves every hostname the
    // user added, and this function used to ask for the list twice.
    let targets = app.settings.targets();

    let mut series = Vec::new();
    for t in &targets {
        let raw: Vec<(f64, Option<f64>)> = rows
            .iter()
            .filter(|(_, target, _, _)| *target == t.key)
            .map(|(ts, _, rtt, ok)| (*ts, if *ok { *rtt } else { None }))
            .collect();
        if raw.is_empty() {
            continue;
        }
        let (raw, breaks) = mark_recording_gaps(raw);
        let (points, losses, outages) = reduce(&raw, &breaks, range, app.chart_smooth);
        series.push(ChartSeries {
            key: t.key.clone(),
            label: t.label.clone(),
            points,
            losses,
            outages,
            scope: t.scope,
        });
    }

    // Spikes are counted on the raw readings, not on the reduced ones: a
    // bucket's maximum is a spike by construction, so counting after
    // reduction would report one for every bucket that contains any jitter.
    let raw_series: Vec<crate::monitor::Series> = targets
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

/// The sweep interval these samples were actually recorded at.
///
/// The median rather than the configured `probe_interval_ms`: the window can
/// hold samples from an older run at a different cadence, and the setting says
/// nothing about what is already in the database.
fn cadence(points: &[(f64, Option<f64>)]) -> Option<f64> {
    let mut deltas: Vec<f64> =
        points.windows(2).map(|w| w[1].0 - w[0].0).filter(|d| *d > 0.0).collect();
    if deltas.is_empty() {
        return None;
    }
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(deltas[deltas.len() / 2])
}

/// How far apart two samples have to be before the space between them means
/// "nobody was watching" rather than "the probe failed".
///
/// Three missed sweeps, and never less than a couple of seconds: ordinary
/// scheduling jitter must not be reported as the recorder stopping. Both the
/// code that inserts the breaks and the code that reads them back use this,
/// because they are two halves of one rule — and when only one of them knew
/// the cadence, the other one hardcoded four seconds and lost every outage
/// marker at any interval of two seconds or more.
fn gap_threshold(step: f64) -> f64 {
    (step * 3.0).max(step + 2.0)
}

/// Breaks the series wherever nothing was recorded for a while.
///
/// A lost packet leaves a row saying so. Time the app spent closed leaves no
/// row at all, and a line drawn straight from the last sample before the gap
/// to the first one after it asserts a steady latency across hours nobody
/// measured. The step is taken from the data rather than from the configured
/// interval, so it stays right when the interval is changed or the samples
/// come from an older run at a different cadence.
/// Returns the series with the breaks inserted, and the timestamps of the
/// breaks themselves.
///
/// The caller needs that second list because both a break and a lost packet
/// are a `None` in the series, and they mean opposite things: "nobody was
/// watching" against "we watched and nothing came back". Saying which is
/// which is this function's business — it is the one that put them there —
/// and the alternative, working it back out from how far apart the
/// neighbours are, is what broke at any sweep interval of two seconds or more.
fn mark_recording_gaps(points: Vec<(f64, Option<f64>)>) -> (Vec<(f64, Option<f64>)>, Vec<f64>) {
    if points.len() < 3 {
        return (points, Vec::new());
    }
    let Some(step) = cadence(&points) else { return (points, Vec::new()) };
    let threshold = gap_threshold(step);

    let mut out = Vec::with_capacity(points.len() + 8);
    let mut breaks = Vec::new();
    for (i, point) in points.iter().enumerate() {
        if i > 0 {
            let previous = points[i - 1].0;
            if point.0 - previous > threshold {
                let at = (previous + point.0) * 0.5;
                breaks.push(at);
                out.push((at, None));
            }
        }
        out.push(*point);
    }
    (out, breaks)
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

fn reduce(raw: &[(f64, Option<f64>)], breaks: &[f64], range: f64, smooth: bool) -> Reduced {
    let mut losses = Vec::new();
    let mut outages = Vec::new();
    if raw.len() <= TARGET_POINTS && !smooth {
        // Unreduced, a `None` is either a lost probe or a break
        // `mark_recording_gaps` inserted. Only the first has a row of its own
        // behind it, and the second is named in `breaks`, so no arithmetic is
        // needed to tell them apart.
        for (ts, v) in raw.iter() {
            if v.is_none() && !breaks.contains(ts) {
                outages.push(*ts);
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
    let all: Vec<(String, String, egui::Color32, crate::settings::Scope)> = cache
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (s.key.clone(), s.label.clone(), SERIES_COLOURS[i % SERIES_COLOURS.len()], s.scope)
        })
        .collect();

    let (y_top, above) = scale(&visible, &app.settings);
    let x_min = cache.oldest - newest;
    let x_min = if x_min < -1.0 { x_min } else { -app.chart_range_s };

    // The plot is the tab's centrepiece, so it takes a share of whatever
    // height there is rather than a fixed 280 px that is most of a laptop
    // screen and a fifth of a desktop one.
    let plot_height = (ui.ctx().screen_rect().height() * 0.32).clamp(170.0, 420.0);

    // The axis numbers were set in the body size, a step and a half above
    // every other caption on the tab, which made the scale shout over the
    // thing it was scaling. egui_plot resolves its tick labels against the
    // style of the `Ui` the plot is added to, so a scope is the only place
    // this can be said.
    let plotted = ui
        .scope(|ui| {
            ui.style_mut().text_styles.insert(
                egui::TextStyle::Body,
                egui::FontId::new(T_MICRO, egui::FontFamily::Proportional),
            );
            Plot::new("latency")
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
                    // The leading newline is the gap. egui_plot draws the x
                    // labels at the very top of the axis strip, which is flush
                    // with the bottom of the plot, and it has no padding to
                    // offer: an empty first line is the only way to get the
                    // numbers off the frame without drawing the axis by hand,
                    // and drawing it by hand would mean picking tick positions
                    // that the plot's own grid lines would then disagree with.
                    let back = -mark.value;
                    if back < 1.0 {
                        format!("\n{}", i18n::live_x_now())
                    } else if back < 60.0 {
                        format!("\n-{back:.0}s")
                    } else {
                        let mins = (back / 60.0).floor();
                        let secs = (back - mins * 60.0).round();
                        format!("\n-{mins:.0}:{secs:02.0}")
                    }
                })
                .y_axis_formatter(|mark, _| {
                    // egui_plot right-aligns the y labels hard against the plot's
                    // left edge and has no padding of its own, so "20" ended up
                    // touching whichever line happened to pass near it and read as
                    // part of the chart rather than as its scale. The gap has to be
                    // part of the text; the spaces are non-breaking because a plain
                    // trailing space is not guaranteed to keep its width through
                    // layout.
                    let v = mark.value;
                    let text = if (v - v.round()).abs() < 0.05 {
                        format!("{v:.0}")
                    } else {
                        format!("{v:.1}")
                    };
                    format!("{text}\u{a0}\u{a0}\u{a0}")
                })
                .label_formatter(|_, _| String::new())
                .show(ui, |plot_ui| {
                    plot_ui.set_plot_bounds(PlotBounds::from_min_max([x_min, 0.0], [0.0, y_top]));

                    // The unit, once, in the corner of the plot it belongs
                    // to. It used to be a word in the caption row, a long way
                    // from the numbers it was the unit for; two letters over
                    // the top of the scale is the whole of what that word had
                    // to say. "ms" is the same in both languages.
                    plot_ui.text(
                        Text::new(
                            PlotPoint::new(x_min, y_top),
                            egui::RichText::new("ms").size(T_MICRO).color(FG_DIM),
                        )
                        .anchor(egui::Align2::LEFT_TOP),
                    );

                    // The two thresholds, each with its value written on it.
                    // They were a faint green line and a faint red one with
                    // nothing to say what height they marked: the reader had
                    // to find the same number in the settings to learn what
                    // the chart was drawing. The label sits at the oldest
                    // edge, where the data is thinnest, and just above the
                    // line it belongs to.
                    for (level, colour) in
                        [(app.settings.ping_ok_ms, GREEN), (app.settings.ping_bad_ms, RED)]
                    {
                        if level > 0.0 && level < y_top {
                            plot_ui.hline(HLine::new(level).color(colour.linear_multiply(0.25)));
                            plot_ui.text(
                                Text::new(
                                    PlotPoint::new(x_min, level),
                                    egui::RichText::new(i18n::live_threshold_mark(level))
                                        .size(T_MICRO)
                                        .color(colour.linear_multiply(0.7)),
                                )
                                .anchor(egui::Align2::LEFT_BOTTOM),
                            );
                        }
                    }

                    for (ts, n) in &spikes.correlated {
                        let strength = if *n >= visible.len().max(2) { 0.42 } else { 0.22 };
                        plot_ui
                            .vline(VLine::new(ts - newest).color(YELLOW.linear_multiply(strength)));
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
                                                .width(1.6_f32)
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
                                Line::new(PlotPoints::from(run))
                                    .color(*colour)
                                    .width(1.6_f32)
                                    .name(&s.label),
                            );
                        }
                    }

                    hovered
                })
        })
        .inner;

    if let Some(at) = plotted.inner {
        // Repaint while the pointer is over the chart. Without it the readout
        // only moves when the next sweep lands, which at a second a sweep
        // looks like the crosshair sticking to the last place it was.
        ui.ctx().request_repaint();
        let x = at.x;
        // Anchored to the pointer, not to the plot. The readout says what
        // every series was at the moment under the crosshair, and the
        // crosshair is wherever the pointer is: shown below the plot instead,
        // the numbers sat a long way from the place on the chart they
        // described, and on a tall plot that was most of the window away.
        plotted.response.on_hover_ui_at_pointer(|ui| {
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
        ui.spacing_mut().item_spacing = egui::vec2(S_XS + 2.0, S_XS);
        for (key, label, colour, scope) in &all {
            let on = !app.hidden_series.contains(key);

            // The last reading, shown on the chip. This is the number the
            // user came to the legend for; making them hover for it was the
            // legend keeping its own contents secret.
            let sample = app.last.results.get(key);
            let value = match sample {
                Some(s) => match (&s.rtt_ms, &s.error) {
                    (Some(rtt), _) => {
                        Some((format!("{rtt:.0} ms"), latency_colour(*rtt, &app.settings)))
                    }
                    (None, Some(_)) => Some((i18n::live_hover_lost().to_string(), RED)),
                    (None, None) => None,
                },
                None => None,
            };

            let resp = legend_chip(ui, *colour, label, value, on);

            // Four lines crossing each other is the state this chart is in
            // most of the time, and the question is usually about one of
            // them. Clicking its name takes the rest away.
            if resp.clicked() {
                toggled = Some(key.clone());
            }

            // The reading, then what this target is and what a high figure on
            // it means. The number alone was the part nobody could use: four
            // lines of milliseconds say nothing until you know which stretch
            // of the path each one measures.
            let reading = match sample {
                Some(s) => match (&s.rtt_ms, &s.error) {
                    (Some(rtt), _) => (
                        format!(
                            "{rtt:.2} ms \u{2014} {}",
                            super::latency_verdict(*rtt, &app.settings)
                        ),
                        latency_colour(*rtt, &app.settings),
                    ),
                    (None, Some(err)) => (err.clone(), RED),
                    (None, None) => (i18n::live_no_data().to_string(), FG_DIM),
                },
                None => (i18n::live_no_data().to_string(), FG_DIM),
            };
            let meaning = i18n::live_target_meaning(key, *scope);
            let hint = if on { i18n::live_series_toggle() } else { i18n::live_series_show() };
            let label = label.clone();
            resp.on_hover_ui(move |ui| {
                ui.set_max_width(super::TIP_WIDTH);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&label).size(T_HEAD).color(FG).strong());
                    ui.label(figure(&reading.0, T_BODY, reading.1));
                });
                ui.add_space(S_XS);
                ui.separator();
                ui.add_space(S_SM);
                if !meaning.is_empty() {
                    super::tip_prose(ui, meaning);
                    ui.add_space(S_SM);
                }
                ui.label(
                    egui::RichText::new(hint)
                        .size(T_MICRO)
                        .italics()
                        .color(ACCENT.linear_multiply(0.85)),
                );
            });
        }
    });

    ui.add_space(S_SM);
    key_row(ui);
    facts_row(ui, &spikes, above, y_top);

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
            if ui
                .selectable_label(on, egui::RichText::new(i18n::range_name(secs)).size(T_META))
                .clicked()
            {
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
fn scale(series: &[(ChartSeries, egui::Color32)], s: &crate::settings::Settings) -> (f64, usize) {
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

/// The headline cards, and when they were last read out of the store.
pub struct CardCache {
    built_at: f64,
    stats: Vec<Stat>,
}

/// How long a set of cards is allowed to stand before it is read again.
///
/// One sweep. The figures behind them are averages over five minutes and a
/// count of a day's outages, so a second is already finer than the data can
/// move, and it is the cadence the rest of the tab runs at.
const CARD_MAX_AGE_S: f64 = 1.0;

/// One card's worth of content, decided before anything is laid out.
pub struct Stat {
    label: &'static str,
    value: String,
    sub: String,
    colour: egui::Color32,
    tip: &'static str,
}

/// The first hop is on a different scale from the rest of the path.
///
/// The settings thresholds describe a trip across the internet, where 30 ms is
/// good. Applied to the router they would call anything short of a disaster
/// healthy: a wired first hop is a fraction of a millisecond, and a Wi-Fi one
/// that has reached 20 ms is already the thing ruining every other figure on
/// the page. These are the numbers that scale belongs on.
const ROUTER_GOOD_MS: f64 = 5.0;
const ROUTER_OK_MS: f64 = 20.0;

/// A name lookup is paid once per site rather than per packet, so it is
/// tolerable at a latency that would be unusable for traffic. Hence its own
/// pair rather than the latency thresholds.
const DNS_GOOD_MS: f64 = 60.0;
const DNS_OK_MS: f64 = 150.0;

/// A latency figure at the precision the size of the number deserves.
///
/// The cards printed whole milliseconds for the internet and one decimal for
/// the router, which is two formats for the same unit sitting side by side.
/// Below ten the decimal is the only thing distinguishing 1.2 ms from 1.9 ms;
/// above it, it is noise on a value that moves by whole milliseconds anyway.
fn ms_text(ms: f64) -> String {
    // The bound is where the decimal would round away rather than at ten
    // exactly: 9.95 printed as "10.0 ms" claims a precision the rounding just
    // threw away, and sits next to "10 ms" from the branch below it.
    if ms < 9.95 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.0} ms")
    }
}

/// Green, amber or red by two thresholds, in the order they are crossed.
fn band(v: f64, good: f64, ok: f64) -> egui::Color32 {
    if v < good {
        GREEN
    } else if v < ok {
        YELLOW
    } else {
        RED
    }
}

fn cards(app: &mut App, ui: &mut egui::Ui) {
    let now = crate::store::now();
    let stale = match &app.card_cache {
        Some(c) => now - c.built_at >= CARD_MAX_AGE_S,
        None => true,
    };
    if stale {
        app.card_cache = Some(CardCache { built_at: now, stats: collect_stats(app) });
    }
    let Some(cache) = app.card_cache.as_ref() else {
        return;
    };
    let stats = &cache.stats;

    // Cards of equal width in aligned columns, rather than a wrapped row.
    //
    // Wrapped, each card was as wide as its own contents, so the row's
    // columns did not line up and the break landed wherever it happened to
    // fall -- most often leaving two cards alone on a second row while the
    // first had four. The count per row is chosen so that every row is full:
    // six cards go six, three or two across and never five.
    let per_row = if ui.available_width() >= 6.0 * CARD_MIN {
        6
    } else if ui.available_width() >= 3.0 * CARD_MIN {
        3
    } else {
        2
    };

    for (row, chunk) in stats.chunks(per_row).enumerate() {
        if row > 0 {
            ui.add_space(S_SM);
        }
        ui.columns(per_row, |cols| {
            for (i, stat) in chunk.iter().enumerate() {
                // Never negative: the frame's own margins are wider than
                // the column at the point a window is squeezed to nothing,
                // and a negative width reaches egui's layout sanity check.
                let width = (cols[i].available_width() - 2.0 * S_MD).max(0.0);
                super::stat_card_ex(
                    &mut cols[i],
                    stat.label,
                    &stat.value,
                    &stat.sub,
                    stat.colour,
                    Some(width),
                    stat.tip,
                );
            }
        });
    }
}

/// How narrow a card is allowed to get before the row drops to fewer of them.
///
/// A card holds a label, a figure at the metric size and a line of context;
/// below about this width the context line is the one that gives, and it is
/// the line doing the explaining.
const CARD_MIN: f32 = 172.0;

fn collect_stats(app: &App) -> Vec<Stat> {
    let s = &app.settings;
    let cf = app.store.stats("cloudflare", 300.0);
    let gw = app.store.stats("gateway", 300.0);
    let mut out = Vec::with_capacity(6);

    out.push(match cf.avg {
        Some(avg) => Stat {
            label: i18n::live_card_latency(),
            value: ms_text(avg),
            sub: i18n::live_minmax(cf.min.unwrap_or(0.0), cf.max.unwrap_or(0.0)),
            colour: latency_colour(avg, s),
            tip: i18n::live_tip_latency(),
        },
        None => Stat {
            label: i18n::live_card_latency(),
            value: "—".into(),
            sub: i18n::live_card_latency_sub_none().into(),
            colour: FG_DIM,
            tip: i18n::live_tip_latency(),
        },
    });

    out.push(match cf.jitter {
        Some(j) => Stat {
            label: i18n::live_card_jitter(),
            value: ms_text(j),
            sub: i18n::live_card_jitter_sub().into(),
            colour: band(j, s.jitter_good_ms, s.jitter_ok_ms),
            tip: i18n::live_tip_jitter(),
        },
        None => Stat {
            label: i18n::live_card_jitter(),
            value: "—".into(),
            sub: i18n::live_card_jitter_sub().into(),
            colour: FG_DIM,
            tip: i18n::live_tip_jitter(),
        },
    });

    out.push(Stat {
        label: i18n::live_card_loss(),
        value: format!("{:.1}%", cf.loss_pct),
        sub: i18n::live_card_loss_sub().into(),
        colour: band(cf.loss_pct, s.loss_good_pct, s.loss_ok_pct),
        tip: i18n::live_tip_loss(),
    });

    out.push(match gw.avg {
        Some(avg) => Stat {
            label: i18n::live_card_router(),
            value: ms_text(avg),
            sub: i18n::live_router_loss(gw.loss_pct),
            colour: band(avg, ROUTER_GOOD_MS, ROUTER_OK_MS),
            tip: i18n::live_tip_router(),
        },
        None => Stat {
            label: i18n::live_card_router(),
            value: "—".into(),
            sub: i18n::live_card_router_none().into(),
            colour: RED,
            tip: i18n::live_tip_router(),
        },
    });

    out.push(if !app.last.dns_error.is_empty() {
        Stat {
            label: i18n::live_card_dns(),
            value: i18n::live_card_dns_err().into(),
            sub: app.last.dns_error.clone(),
            colour: RED,
            tip: i18n::live_tip_dns(),
        }
    } else {
        match app.last.dns_ms {
            Some(ms) => Stat {
                label: i18n::live_card_dns(),
                value: ms_text(ms),
                sub: i18n::live_card_dns_sub().into(),
                colour: band(ms, DNS_GOOD_MS, DNS_OK_MS),
                tip: i18n::live_tip_dns(),
            },
            None => Stat {
                label: i18n::live_card_dns(),
                value: "—".into(),
                sub: i18n::live_card_dns_sub().into(),
                colour: FG_DIM,
                tip: i18n::live_tip_dns(),
            },
        }
    });

    let events = app.store.events_since(24.0 * 3600.0);
    out.push(if events.is_empty() {
        // No outage logged is only "24 h+" if we were actually watching for
        // 24 h without a break. On a fresh install the history is minutes
        // old, and on a machine that slept for a week the week is not
        // evidence of anything -- claiming either is a lie the user can catch
        // immediately. The monitor keeps the start of the current unbroken
        // stretch; see [`crate::store::observing_since`].
        let observed =
            app.last.observed_from.map_or(0.0, |from| (crate::store::now() - from).max(0.0));
        let full_day = observed >= 24.0 * 3600.0;
        Stat {
            label: i18n::live_card_uninterrupted(),
            value: if full_day {
                i18n::live_card_uninterrupted_val().into()
            } else {
                i18n::live_uninterrupted_for(observed)
            },
            sub: if full_day {
                i18n::live_card_uninterrupted_sub().into()
            } else {
                i18n::live_uninterrupted_short().into()
            },
            colour: GREEN,
            tip: i18n::live_tip_uptime(),
        }
    } else {
        let last = events.iter().map(|e| e.ts_start).fold(f64::NEG_INFINITY, f64::max);
        let mins = (crate::store::now() - last) / 60.0;
        Stat {
            label: i18n::live_card_since_outage(),
            value: format!("{mins:.0} min"),
            sub: i18n::live_outages_24h(events.len()),
            colour: if events.len() < 3 { YELLOW } else { RED },
            tip: i18n::live_tip_uptime(),
        }
    });

    out
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
                Some(added) => i18n::path_blame_delay(b.ttl, &b.addr.to_string(), added, owner),
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
    egui::Grid::new("path_hops").num_columns(5).striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
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
            ui.label(egui::RichText::new(i18n::path_owner(hop.owner)).size(T_META).color(FG_DIM));
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
    });

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
            // The last day, as it always was; the History tab picks a range.
            let label =
                if app.report_busy { i18n::hist_report_saving() } else { i18n::live_btn_report() };
            if button(ui, label, Emphasis::Ghost).clicked() {
                super::report::save_in_background(app, super::report::Range::Day);
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

    egui::Frame::none().fill(super::BG2).rounding(6.0).inner_margin(egui::Margin::same(S_MD)).show(
        ui,
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("trace_output")
                .max_height(260.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &app.trace {
                        ui.label(egui::RichText::new(line).monospace().size(T_META).color(ACCENT));
                    }
                });
        },
    );
}

/// The chart's key: what the markings on it mean, and nothing else.
///
/// Each entry is a swatch in the colour it is about, followed by what that
/// colour marks. The words that named the colours are gone — the swatch is
/// the name — and so is everything that was not a marking: the unit, the
/// threshold words and the instruction now live behind the badge at the end,
/// which is where a thing you read once belongs.
fn key_row(ui: &mut egui::Ui) {
    // The heading gets a line of its own rather than a place at the head of
    // the row. Inside the row it wrapped along with the entries, and a title
    // that can end up alone at the end of a line is not a title.
    ui.label(egui::RichText::new(i18n::live_key_heading()).size(T_MICRO).color(FG_DIM).strong());
    ui.add_space(S_XS);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = S_XS + 2.0;

        swatch(ui, RED);
        micro(ui, i18n::live_key_lost());
        ui.add_space(S_SM);

        swatch(ui, YELLOW);
        micro(ui, i18n::live_key_spike());
        ui.add_space(S_SM);

        micro(ui, i18n::live_key_bands());
        for (i, colour) in [GREEN, YELLOW, RED].into_iter().enumerate() {
            if i > 0 {
                ui.add_space(S_XS * 0.5);
            }
            swatch(ui, colour);
        }

        ui.add_space(S_SM);
        help_badge(ui);
    });
}

/// What the data in view happens to be doing, kept off the key row.
///
/// These two are counts, not a legend: they change with the window and with
/// the link, and mixed into the fixed key they were two more items in a row
/// of seven that nobody could tell apart. On a quiet chart the row is not
/// drawn at all.
fn facts_row(ui: &mut egui::Ui, spikes: &crate::monitor::Spikes, above: usize, y_top: f64) {
    let total = spikes.single + spikes.correlated.len();
    if above == 0 && total == 0 {
        return;
    }

    ui.add_space(S_XS);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = S_SM;
        if above > 0 {
            ui.label(
                egui::RichText::new(i18n::live_above_scale(above, y_top))
                    .size(T_MICRO)
                    .color(YELLOW),
            );
        }
        if above > 0 && total > 0 {
            dot(ui);
        }
        if total > 0 {
            // The tally is the line that reframes the chart: it says how much
            // of what looks like instability was never about the connection.
            // Short here, with the reasoning behind it on hover, because the
            // full sentence was three lines of prose under a chart.
            ui.label(
                egui::RichText::new(i18n::live_spike_counts(
                    spikes.correlated.len(),
                    spikes.single,
                ))
                .size(T_MICRO)
                .color(FG_DIM),
            )
            .on_hover_cursor(egui::CursorIcon::Help)
            .on_hover_ui(|ui| {
                super::tip_prose(
                    ui,
                    &format!(
                        "{}

{}",
                        i18n::live_spike_tally(spikes.correlated.len(), spikes.single),
                        i18n::live_spike_explainer()
                    ),
                );
            });
        }
    });
}

/// One caption, at the size the captions are.
fn micro(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(T_MICRO).color(FG_DIM));
}

/// The badge that holds everything about the chart worth explaining once.
///
/// A question mark and nothing else. It was a question mark followed by "what
/// am I looking at?", which is three more words on a row whose whole problem
/// was that it had too many: the mark is the oldest symbol there is for help
/// behind it, and the row is a key, not a sentence. Someone who already knows
/// what a millisecond is skips it with their eyes; someone who does not has
/// one obvious place to ask.
fn help_badge(ui: &mut egui::Ui) {
    let size = T_MICRO + 2.0 * S_XS;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let hovered = response.hovered();
    ui.painter().circle_filled(
        rect.center(),
        size * 0.5,
        if hovered { BG3.linear_multiply(1.6) } else { BG3 },
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "?",
        egui::FontId::proportional(T_MICRO),
        if hovered { FG } else { ACCENT },
    );
    response.on_hover_cursor(egui::CursorIcon::Help).on_hover_ui(|ui| {
        super::tip_heading(ui, i18n::live_key_help());
        super::tip_prose(ui, &i18n::live_chart_help());
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
                scope: crate::settings::Scope::Internet,
            },
            egui::Color32::WHITE,
        )]
    }

    /// Twelve sweeps at a given cadence, with three in the middle lost.
    fn sweeps(interval_s: f64) -> Vec<(f64, Option<f64>)> {
        (0..12)
            .map(|i| {
                let ts = 1_000.0 + i as f64 * interval_s;
                (ts, if (5..8).contains(&i) { None } else { Some(14.0) })
            })
            .collect()
    }

    #[test]
    fn lost_probes_are_marked_at_every_configured_interval() {
        // The bug this guards: the unreduced path tested the gap between a
        // lost probe's neighbours against a hardcoded 4.0 seconds, which
        // assumes a sweep every second. Settings allow anything from 300 ms
        // up, and at two seconds every outage marker silently disappeared.
        for interval in [0.3, 1.0, 2.0, 3.0, 5.0] {
            let (raw, breaks) = mark_recording_gaps(sweeps(interval));
            let (_, _, outages) = reduce(&raw, &breaks, 3_600.0, false);
            assert_eq!(
                outages.len(),
                3,
                "probe_interval = {interval} s produced {} marker(s) for 3 lost probes",
                outages.len()
            );
        }
    }

    #[test]
    fn a_hole_in_the_recording_is_not_an_outage() {
        // The other half of the same rule: a break this app inserted because
        // nothing was recorded must not be counted as a lost packet.
        let mut points: Vec<(f64, Option<f64>)> =
            (0..8).map(|i| (1_000.0 + i as f64, Some(14.0))).collect();
        points.extend((0..8).map(|i| (1_400.0 + i as f64, Some(14.0))));

        let (raw, breaks) = mark_recording_gaps(points);
        let (_, _, outages) = reduce(&raw, &breaks, 3_600.0, false);
        assert!(outages.is_empty(), "a recording gap is not a lost probe: {outages:?}");
    }

    #[test]
    fn the_first_probe_lost_after_a_break_is_still_marked() {
        // The case that decided how this is told apart. Inferring it from how
        // far the neighbours sit apart gets this one wrong: the probe's left
        // neighbour *is* the inserted break, so the span across it is wide,
        // and the loss that started the moment recording resumed would be
        // read as more of the silence. Asking the function that inserted the
        // break has no such edge.
        let mut points: Vec<(f64, Option<f64>)> =
            (0..8).map(|i| (1_000.0 + i as f64, Some(14.0))).collect();
        points.extend((0..8).map(|i| (1_400.0 + i as f64, Some(14.0))));
        points[8].1 = None;

        let (raw, breaks) = mark_recording_gaps(points);
        let (_, _, outages) = reduce(&raw, &breaks, 3_600.0, false);
        assert_eq!(outages, vec![1_400.0], "the probe after the break failed, and says so");
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
                scope: crate::settings::Scope::Internet,
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
        let (points, losses, _) = reduce(&raw, &[], 120.0, false);
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

        let (points, _, _) = reduce(&raw, &[], 3600.0, false);
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

        let (points, _, _) = reduce(&raw, &[], 3600.0, true);
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
        let (points, losses, outages) = reduce(&raw, &[], 1200.0, false);
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

        let (points, losses, _) = reduce(&raw, &[], 3600.0, false);
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

        let (marked, breaks) = mark_recording_gaps(raw);
        let (points, _, outages) = reduce(&marked, &breaks, 660.0, false);

        assert!(points.iter().any(|(_, v)| v.is_none()), "the line has to break across it");
        assert!(outages.is_empty(), "nothing was lost there; nothing was even asked");
    }

    #[test]
    fn ordinary_jitter_in_the_sweep_is_not_a_recording_gap() {
        let raw: Vec<(f64, Option<f64>)> =
            (0..60).map(|i| (i as f64 * 1.0 + (i % 3) as f64 * 0.2, Some(12.0))).collect();
        let (marked, _) = mark_recording_gaps(raw.clone());
        assert_eq!(marked.len(), raw.len(), "a sweep that runs late has not stopped");
    }

    #[test]
    fn a_lost_probe_is_still_reported_as_one() {
        let mut raw: Vec<(f64, Option<f64>)> = (0..60).map(|i| (i as f64, Some(12.0))).collect();
        raw[30].1 = None;
        let (marked, breaks) = mark_recording_gaps(raw);
        let (_, _, outages) = reduce(&marked, &breaks, 60.0, false);
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
