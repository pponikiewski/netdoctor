//! Diagnose tab: the verdict first, the evidence behind it second.
//!
//! The scan used to hand the user a flat list of a dozen findings, most of
//! them green, and leave the reading of it to them. Ordering by severity is
//! not the same as answering the question, so the tab now leads with the one
//! thing the scan concluded — which segment of the chain is at fault, what it
//! costs, what to do — and keeps the findings underneath as the evidence for
//! that conclusion. Everything that came back healthy is collapsed: it is
//! worth being able to confirm, and not worth reading past.

use std::sync::Arc;

use eframe::egui;

use super::{
    button, button_ex, card, status_dot, App, Emphasis, Job, ACCENT, BG2, BG3, FG, FG_DIM, GREEN,
    RED, S_LG, S_MD, S_SM, S_XS, T_BODY, T_HEAD, T_LEAD, T_META, T_TITLE, YELLOW,
};
use crate::diagnose::{self, Ipv6State, Link, LinkState, Segment, Severity, Verdict};
use crate::i18n;
use crate::longrun::{LongRun, Trouble};
use crate::probe::netstate::Medium;

fn severity_colour(s: Severity) -> egui::Color32 {
    match s {
        Severity::Critical => RED,
        Severity::Warn => YELLOW,
        Severity::Info => super::ACCENT,
        Severity::Good => GREEN,
    }
}

fn segment_colour(seg: Segment) -> egui::Color32 {
    match seg {
        Segment::Healthy => GREEN,
        Segment::Config => super::ACCENT,
        Segment::Unmeasured => FG_DIM,
        _ => RED,
    }
}

/// What the last scan found, kept when the next one replaces it, so a scan
/// run after a fix can say whether the fix moved anything.
#[derive(Debug, Clone)]
pub struct PrevScan {
    at: f64,
    segment: Segment,
    router_ms: Option<f64>,
    internet_ms: Option<f64>,
}

impl PrevScan {
    pub fn of(at: f64, verdict: &Verdict, m: &diagnose::Measurements) -> Self {
        let avg = |s: &Option<crate::store::Stats>| s.as_ref().and_then(|s| s.avg);
        PrevScan {
            at,
            segment: verdict.segment,
            router_ms: avg(&m.gateway),
            internet_ms: avg(&m.internet),
        }
    }
}

/// Whether two progress labels are the same step saying how far it got. The
/// long measurement reports "Long measurement: 0:45 of 5:00" every second,
/// and a checklist of those would be three hundred lines long.
pub fn same_step(a: &str, b: &str) -> bool {
    match (a.split_once(':'), b.split_once(':')) {
        (Some((x, _)), Some((y, _))) => x == y,
        _ => false,
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    header(app, ui);
    ui.add_space(S_MD);

    // What was clicked is decided while the page reads `app`, and applied
    // once it is done reading.
    let mut jump_to: Option<String> = None;
    let mut picked: Option<usize> = None;
    let mut fix: Option<String> = None;
    let mut ask_ai = false;

    // One scroll for the whole report. The findings list used to scroll on
    // its own under a fixed verdict, which was fine for a card and a list;
    // with the chain and the numbers above it, a second scroll inside the
    // first is a list nobody finds the bottom of.
    let view: &App = app;
    egui::ScrollArea::vertical().id_salt("diag").auto_shrink([false, false]).show(ui, |ui| {
        if view.scanning {
            running_card(view, ui);
        }
        if view.findings.is_empty() {
            if !view.scanning {
                empty_card(ui);
            }
            return;
        }
        verdict_card(view, ui, &mut jump_to);
        ai_card(view, ui, &mut ask_ai);
        chain_card(view, ui);
        if let Some(run) = &view.measurements.long {
            long_card(run, ui);
        }
        measure_card(view, ui);
        findings(view, ui, &mut picked, &mut fix);
    });

    if ask_ai {
        start_ai(app);
    }
    if let Some(id) = jump_to.or(fix) {
        open_tweak(app, &id);
    } else if let Some(i) = picked {
        app.selected_finding = Some(i);
    }
}

/// The title, the controls, and what running them costs.
fn header(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(i18n::diag_title()).size(T_LEAD).strong().color(FG));
    ui.add_space(S_XS);
    ui.label(egui::RichText::new(i18n::diag_blurb()).size(T_BODY).color(FG_DIM));
    ui.add_space(S_MD);

    ui.horizontal(|ui| {
        // A checkbox and a combo box are shorter than the app's buttons, so
        // in a centred row they sat below them.
        ui.spacing_mut().interact_size.y = super::BTN_H;
        // Not while the speed test runs: a scan beside it measures the test,
        // and its own load step would saturate the line a second time.
        let blocked = app.scanning || app.bloat_running;
        let label =
            if app.findings.is_empty() { i18n::diag_btn_scan() } else { i18n::diag_btn_rescan() };
        let scan = button_ex(ui, label, Emphasis::Primary, !blocked, 0.0);
        let scan =
            if app.bloat_running { scan.on_hover_text(i18n::diag_busy_load()) } else { scan };
        if scan.clicked() {
            start_scan(app);
        }
        if app.scanning
            && app.long_secs > 0
            && button(ui, i18n::diag_btn_stop(), Emphasis::Secondary).clicked()
        {
            app.scan_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        ui.add_space(S_SM);
        ui.add_enabled(!app.scanning, egui::Checkbox::new(&mut app.deep_scan, i18n::diag_deep()))
            .on_hover_text(i18n::diag_deep_hint());

        ui.add_space(S_MD);
        ui.label(egui::RichText::new(i18n::diag_duration()).size(T_BODY).color(FG_DIM))
            .on_hover_text(i18n::diag_duration_hint());
        let name = |secs: usize| {
            if secs == 0 {
                i18n::diag_duration_quick().to_string()
            } else {
                format!("+ {} min", secs / 60)
            }
        };
        ui.add_enabled_ui(!app.scanning, |ui| {
            egui::ComboBox::from_id_salt("long_secs")
                .selected_text(name(app.long_secs))
                .show_ui(ui, |ui| {
                    for secs in LONG_CHOICES {
                        ui.selectable_value(&mut app.long_secs, secs, name(secs));
                    }
                })
                .response
                .on_hover_text(i18n::diag_duration_hint());
        });

        // The report carries the scan on screen, so it is offered once there
        // is one to carry.
        if !app.findings.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if app.report_busy {
                    i18n::hist_report_saving()
                } else {
                    i18n::live_btn_report()
                };
                if button_ex(ui, label, Emphasis::Ghost, !app.report_busy, 0.0).clicked() {
                    let range = app.report_range;
                    super::report::save_in_background(app, range);
                }
            });
        }
    });

    // The same warning the speed test tab carries. The box used to be ticked
    // from the start and the main button then pulled up to a gigabyte with
    // nothing on this screen saying so.
    ui.add_space(S_XS);
    ui.horizontal(|ui| {
        let (dot, text) = if app.deep_scan {
            (YELLOW, i18n::bloat_cost_warning())
        } else {
            (FG_DIM, i18n::diag_quick_note())
        };
        status_dot(ui, dot, 3.5);
        ui.add(egui::Label::new(egui::RichText::new(text).size(T_META).color(FG_DIM)).wrap());
    });
}

/// The running scan: the bar, and every step it has reached, the current one
/// lit. A bar alone says how far, not what it is doing.
fn running_card(app: &App, ui: &mut egui::Ui) {
    card(ui, i18n::diag_running_title(), |ui| {
        ui.add(egui::ProgressBar::new(app.scan_progress).desired_height(6.0));
        ui.add_space(S_SM);
        let n = app.scan_steps.len();
        for (i, step) in app.scan_steps.iter().enumerate() {
            let current = i + 1 == n;
            ui.horizontal(|ui| {
                status_dot(ui, if current { ACCENT } else { GREEN }, 3.5);
                ui.label(egui::RichText::new(step).size(T_BODY).color(if current {
                    FG
                } else {
                    FG_DIM
                }));
            });
        }
    });
}

/// Before the first scan: what it will look at, in the order it looks.
fn empty_card(ui: &mut egui::Ui) {
    card(ui, i18n::diag_empty_title(), |ui| {
        let steps = [
            i18n::diag_empty_1(),
            i18n::diag_empty_2(),
            i18n::diag_empty_3(),
            i18n::diag_empty_4(),
            i18n::diag_empty_5(),
        ];
        for (i, step) in steps.iter().enumerate() {
            ui.horizontal_top(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(18.0, T_BODY + 4.0), egui::Sense::hover());
                ui.painter().text(
                    rect.left_top() + egui::vec2(0.0, 1.0),
                    egui::Align2::LEFT_TOP,
                    format!("{}.", i + 1),
                    egui::FontId::proportional(T_BODY),
                    ACCENT,
                );
                ui.add(egui::Label::new(egui::RichText::new(*step).size(T_BODY).color(FG)).wrap());
            });
            ui.add_space(S_XS);
        }
        ui.add_space(S_SM);
        ui.label(egui::RichText::new(i18n::diag_scan_hint()).size(T_META).color(FG_DIM));
    });
}

/// The optional second opinion. Off, it is one line saying it exists; on, it
/// is a button, a plain statement of what the button sends, and the answer
/// labelled as an interpretation.
fn ai_card(app: &App, ui: &mut egui::Ui, ask: &mut bool) {
    let Some(_) = crate::ai::key(&app.settings) else {
        ui.label(egui::RichText::new(i18n::ai_off_hint()).size(super::T_META).color(FG_DIM));
        ui.add_space(S_MD);
        return;
    };
    let model = crate::ai::model(&app.settings);

    card(ui, i18n::ai_heading(), |ui| {
        match &app.ai_answer {
            Some(Ok(text)) => {
                ui.label(egui::RichText::new(text).size(T_BODY).color(FG));
                ui.add_space(S_SM);
                ui.label(
                    egui::RichText::new(i18n::ai_disclaimer(model))
                        .size(super::T_MICRO)
                        .color(FG_DIM),
                );
            }
            Some(Err(why)) => {
                ui.label(egui::RichText::new(why).size(T_BODY).color(RED));
            }
            None if !app.ai_running => {
                ui.label(
                    egui::RichText::new(i18n::ai_sends(model)).size(super::T_META).color(FG_DIM),
                );
            }
            None => {}
        }

        ui.add_space(S_SM);
        ui.horizontal(|ui| {
            if app.ai_running {
                ui.spinner();
                ui.label(egui::RichText::new(i18n::ai_running()).size(T_BODY).color(FG_DIM));
                return;
            }
            let label =
                if app.ai_answer.is_some() { i18n::ai_btn_again() } else { i18n::ai_btn_ask() };
            if button_ex(ui, label, Emphasis::Secondary, !app.scanning, 0.0).clicked() {
                *ask = true;
            }
        });

        // Being able to read the exact payload is what makes "we strip your
        // network's name" a checkable statement rather than a promise.
        egui::CollapsingHeader::new(
            egui::RichText::new(i18n::ai_preview()).size(super::T_META).color(FG_DIM),
        )
        .id_salt("ai_payload")
        .show(ui, |ui| {
            let payload = crate::ai::report(&scan_of(app), &app.scan_net);
            ui.label(super::figure(payload, super::T_MICRO, FG_DIM));
        });
    });
}

/// The scan on screen, reassembled for the AI request. The clone is
/// deliberate: the request runs on its own thread.
fn scan_of(app: &App) -> diagnose::Scan {
    diagnose::Scan {
        findings: app.findings.clone(),
        verdict: app.verdict.clone(),
        measurements: app.measurements.clone(),
    }
}

fn start_ai(app: &mut App) {
    app.ai_running = true;
    app.ai_answer = None;
    let tx = app.tx.clone();
    let scan = scan_of(app);
    let net = app.scan_net.clone();
    let settings = app.settings.clone();
    let generation = app.scan_gen;
    std::thread::spawn(move || {
        let answer = crate::ai::explain(&scan, &net, &settings);
        let _ = tx.send(Job::AiDone(generation, answer));
    });
}

/// The individual checks: a list beside its detail pane.
fn findings(app: &App, ui: &mut egui::Ui, picked: &mut Option<usize>, fix: &mut Option<String>) {
    let findings = &app.findings;
    let selected_finding = app.selected_finding;

    card(ui, i18n::findings_heading(), |ui| {
        // A list beside its detail pane when there is room, one above the
        // other when there is not. Half of a narrow window is not enough for
        // either: the findings wrap onto three lines each and the
        // explanation beside them becomes a column of single words.
        let narrow = super::is_narrow(ui);
        let mut list = |ui: &mut egui::Ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut row = |ui: &mut egui::Ui, i: usize, f: &crate::diagnose::Finding| {
                    let colour = severity_colour(f.severity);
                    let first = f.detail.lines().next().unwrap_or("");
                    let hit = ui
                        .push_id(&f.key, |ui| {
                            super::list_row(
                                ui,
                                44.0,
                                selected_finding == Some(i),
                                colour,
                                &f.title,
                                false,
                                Some((f.severity.label(), colour)),
                                &[(first, FG_DIM)],
                            )
                        })
                        .inner;
                    if hit.clicked() {
                        *picked = Some(i);
                    }
                };

                // Anything that needs attention stays in the open.
                let mut healthy = Vec::new();
                for (i, f) in findings.iter().enumerate() {
                    if f.severity == Severity::Good {
                        healthy.push(i);
                    } else {
                        row(ui, i, f);
                    }
                }

                // The rest is not a result, it is a reassurance. Worth being
                // able to open, never worth pushing the real findings off the
                // screen.
                if !healthy.is_empty() {
                    ui.add_space(S_SM);
                    egui::CollapsingHeader::new(
                        egui::RichText::new(i18n::diag_checked_ok(healthy.len()))
                            .size(T_BODY)
                            .color(FG_DIM),
                    )
                    .id_salt("healthy")
                    .show(ui, |ui| {
                        for i in healthy {
                            row(ui, i, &findings[i]);
                        }
                    });
                }
            });
        };

        let mut detail = |ui: &mut egui::Ui| {
            egui::Frame::none()
                .fill(BG3)
                .rounding(6.0)
                .inner_margin(egui::Margin::same(S_MD))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    match selected_finding.and_then(|i| findings.get(i)) {
                        None => {
                            ui.label(
                                egui::RichText::new(i18n::diag_select_finding())
                                    .size(T_BODY)
                                    .color(FG_DIM),
                            );
                        }
                        Some(f) => {
                            ui.label(
                                egui::RichText::new(&f.title)
                                    .size(T_TITLE)
                                    .strong()
                                    .color(severity_colour(f.severity)),
                            );
                            ui.add_space(S_SM);
                            if !f.detail.is_empty() {
                                ui.label(egui::RichText::new(&f.detail).size(T_BODY).color(FG));
                                ui.add_space(S_SM);
                            }
                            if !f.advice.is_empty() {
                                ui.label(egui::RichText::new(&f.advice).size(T_BODY).color(FG_DIM));
                            }
                            if let Some(id) = &f.tweak_id {
                                ui.add_space(S_MD);
                                if button(ui, i18n::diag_btn_fix(), Emphasis::Secondary).clicked() {
                                    *fix = Some(id.clone());
                                }
                            }
                        }
                    }
                });
        };

        if narrow {
            list(ui);
            ui.add_space(S_MD);
            detail(ui);
        } else {
            ui.columns(2, |cols| {
                list(&mut cols[0]);
                detail(&mut cols[1]);
            });
        }
    });
}

/// The colour a link is drawn in: the fault's colour on the link the verdict
/// names, otherwise what its own reading says.
fn link_colour(link: &Link, at_fault: bool, fault_colour: egui::Color32) -> egui::Color32 {
    if at_fault {
        return fault_colour;
    }
    match &link.state {
        LinkState::Measured(s) if s.loss_pct > 0.0 => YELLOW,
        LinkState::Measured(_) => GREEN,
        LinkState::Silent => RED,
        LinkState::Filtered | LinkState::Unknown | LinkState::NotMeasured => FG_DIM,
    }
}

/// The two lines under a link: what it measured, and who answered.
fn link_text(link: &Link) -> (String, String) {
    let first = match &link.state {
        LinkState::Measured(s) => i18n::link_caption(link.added_ms, s.loss_pct),
        LinkState::Silent => i18n::link_silent().into(),
        LinkState::Filtered => i18n::link_filtered().into(),
        LinkState::Unknown => i18n::link_unknown().into(),
        LinkState::NotMeasured => i18n::link_not_measured().into(),
    };
    let second = if link.local {
        i18n::link_local().into()
    } else {
        link.addr.map(|a| a.to_string()).unwrap_or_default()
    };
    (first, second)
}

/// This PC, the router, the provider and the internet, joined by the three
/// links the scan measured. The picture is the verdict's evidence at a
/// glance: which link carries the milliseconds, which one lost packets.
fn chain_card(app: &App, ui: &mut egui::Ui) {
    let links = diagnose::chain(&app.findings, &app.measurements);
    let fault = app.verdict.segment.link_index();
    let fault_colour = segment_colour(app.verdict.segment);

    card(ui, i18n::chain_heading(), |ui| {
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 84.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        // Room at each end for the outermost node's name, which is centred
        // on its dot.
        let pad = 46.0;
        let x = |i: usize| rect.left() + pad + (w - 2.0 * pad) * i as f32 / 3.0;
        let name_y = rect.top();
        let dot_y = rect.top() + 30.0;
        let text_y = dot_y + 14.0;

        for (i, link) in links.iter().enumerate() {
            let at_fault = fault == Some(i);
            let colour = link_colour(link, at_fault, fault_colour);
            let width = if at_fault { 4.0 } else { 2.0 };
            let (a, b) = (egui::pos2(x(i) + 9.0, dot_y), egui::pos2(x(i + 1) - 9.0, dot_y));
            // A link the scan knows nothing about is drawn broken, so an
            // unknown never looks like a healthy line in a dim colour.
            if matches!(link.state, LinkState::Measured(_)) {
                painter.line_segment([a, b], egui::Stroke::new(width, colour));
            } else {
                painter.extend(egui::Shape::dashed_line(
                    &[a, b],
                    egui::Stroke::new(width, colour),
                    5.0,
                    4.0,
                ));
            }
            let mid = (x(i) + x(i + 1)) / 2.0;
            let (first, second) = link_text(link);
            // The caption describes this reading, not the verdict. The verdict
            // can rest on the outage history while the link measured clean
            // just now, and numbers drawn in red read as bad numbers.
            let caption = match link_colour(link, false, fault_colour) {
                GREEN => FG,
                other => other,
            };
            painter.text(
                egui::pos2(mid, text_y),
                egui::Align2::CENTER_TOP,
                first,
                egui::FontId::proportional(super::T_META),
                caption,
            );
            painter.text(
                egui::pos2(mid, text_y + 16.0),
                egui::Align2::CENTER_TOP,
                second,
                egui::FontId::monospace(super::T_MICRO),
                FG_DIM,
            );
        }

        let names = [i18n::node_pc(), i18n::node_router(), i18n::node_isp(), i18n::node_internet()];
        for (i, name) in names.iter().enumerate() {
            painter.circle(
                egui::pos2(x(i), dot_y),
                6.0,
                super::BG3,
                egui::Stroke::new(2.0, FG_DIM),
            );
            painter.text(
                egui::pos2(x(i), name_y),
                egui::Align2::CENTER_TOP,
                *name,
                egui::FontId::proportional(super::T_META),
                FG,
            );
        }

        ui.add_space(S_SM);
        ui.label(egui::RichText::new(i18n::chain_note()).size(super::T_MICRO).color(FG_DIM));
    });
}

/// Every number the scan took, including the ones it could not take. A
/// verdict that cannot be checked is a verdict taken on trust, and this app
/// is used to argue with a provider.
fn measure_card(app: &App, ui: &mut egui::Ui) {
    let m = &app.measurements;
    let links = diagnose::chain(&app.findings, m);
    let has = |key: &str| app.findings.iter().any(|f| f.key == key);

    card(ui, i18n::measure_heading(), |ui| {
        if let Some(why) = &m.blind {
            ui.label(egui::RichText::new(i18n::measure_blind()).size(T_BODY).color(YELLOW));
            ui.label(egui::RichText::new(why).size(T_BODY).color(FG));
            ui.add_space(S_SM);
        }

        let legs = [
            (i18n::measure_leg_router(), &links[0]),
            (i18n::measure_leg_isp(), &links[1]),
            (i18n::measure_leg_internet(), &links[2]),
        ];
        let anchors = format!("{}, {}", diagnose::ANCHOR, diagnose::ANCHOR_ALT);
        egui::ScrollArea::horizontal().id_salt("measure_grid").show(ui, |ui| {
            egui::Grid::new("measure").striped(true).spacing([S_LG, S_SM]).show(ui, |ui| {
                for head in [
                    i18n::measure_col_leg(),
                    i18n::measure_col_addr(),
                    i18n::measure_col_sent(),
                    i18n::measure_col_loss(),
                    i18n::measure_col_min(),
                    i18n::measure_col_avg(),
                    i18n::measure_col_max(),
                    i18n::measure_col_jitter(),
                ] {
                    ui.label(egui::RichText::new(head).size(super::T_META).color(FG_DIM));
                }
                ui.end_row();

                for (i, (name, link)) in legs.iter().enumerate() {
                    ui.label(egui::RichText::new(*name).size(T_BODY).color(FG));
                    let addr = match (i, link.addr) {
                        (2, _) => anchors.clone(),
                        (_, Some(a)) if link.local => format!("{a} ({})", i18n::link_local()),
                        (_, Some(a)) => a.to_string(),
                        (_, None) => String::new(),
                    };
                    ui.label(super::figure(addr, super::T_META, FG_DIM));
                    match &link.state {
                        LinkState::Measured(s) => {
                            let ms = |v: Option<f64>| {
                                v.map(|v| format!("{v:.1} ms")).unwrap_or_else(|| "-".into())
                            };
                            let loss_colour = if s.loss_pct > 0.0 { YELLOW } else { FG };
                            ui.label(super::figure(s.count.to_string(), T_BODY, FG));
                            ui.label(super::figure(
                                format!("{:.0}%", s.loss_pct),
                                T_BODY,
                                loss_colour,
                            ));
                            for v in [s.min, s.avg, s.max, s.jitter] {
                                ui.label(super::figure(ms(v), T_BODY, FG));
                            }
                        }
                        other => {
                            let (text, colour) = match other {
                                LinkState::Silent => (i18n::link_silent(), RED),
                                LinkState::Filtered => (i18n::link_filtered(), FG_DIM),
                                LinkState::Unknown => (i18n::link_unknown(), FG_DIM),
                                _ => (i18n::link_not_measured(), FG_DIM),
                            };
                            ui.label(egui::RichText::new(text).size(T_BODY).color(colour));
                        }
                    }
                    ui.end_row();
                }
            });
        });
        ui.add_space(S_XS);
        ui.label(
            egui::RichText::new(i18n::measure_cumulative()).size(super::T_MICRO).color(FG_DIM),
        );
        ui.add_space(S_MD);

        let ms = |v: f64| format!("{v:.0} ms");
        let medium = match (&m.medium, m.signal_pct) {
            (Medium::Wifi, Some(pct)) => i18n::measure_signal(pct),
            (medium, _) => medium.label().to_string(),
        };
        let dns = match m.dns_ms {
            Some(v) => ms(v),
            None if has("dns_resolve") => i18n::link_silent().into(),
            None => i18n::measure_none().into(),
        };
        let tcp = match m.tcp_ms {
            Some(v) => ms(v),
            None if m.tcp_blocked => i18n::measure_tcp_blocked().into(),
            None => i18n::measure_none().into(),
        };
        let baseline = match m.baseline {
            Some((v, n)) => i18n::measure_baseline_value(v, n),
            // The history is only looked up against a reading to compare it
            // with; without one, "not enough history" would be a guess.
            None if m.internet.is_none() => i18n::measure_none().into(),
            None => i18n::measure_baseline_none().into(),
        };
        let load = match &m.load {
            Some(l) if l.grade.is_some() => i18n::f_load_detail(
                l.idle_avg.unwrap_or(0.0),
                l.loaded_avg.unwrap_or(0.0),
                l.mbps.unwrap_or(0.0),
                l.grade_or_unknown().letter(),
            ),
            Some(l) => l.error.clone(),
            None => i18n::measure_load_skipped().into(),
        };

        let ipv6 = match &m.ipv6 {
            Some(Ipv6State::Works(v)) => ms(*v),
            Some(Ipv6State::Absent) => i18n::ipv6_absent().into(),
            Some(Ipv6State::Broken) => i18n::ipv6_broken().into(),
            Some(Ipv6State::Failed(why)) => why.clone(),
            None => i18n::measure_none().into(),
        };
        let dns_public = match (m.dns_own_ms, m.dns_public_ms) {
            (Some(own), Some(public)) => format!("{} / {}", ms(own), ms(public)),
            (Some(own), None) => format!("{} / {}", ms(own), i18n::link_silent()),
            _ => i18n::measure_none().into(),
        };
        let route = match &m.vpn {
            Some(name) => i18n::measure_route_vpn(name),
            None => i18n::measure_route_direct().to_string(),
        };
        let proxy = m.proxy.clone().unwrap_or_else(|| i18n::word_none().to_string());
        let syslog = if m.syslog.is_empty() {
            i18n::measure_syslog_none().to_string()
        } else {
            m.syslog
                .iter()
                .take(4)
                .map(|e| format!("{} {}", diagnose::format_clock_s(e.ts), i18n::log_kind(e.kind)))
                .collect::<Vec<_>>()
                .join(", ")
        };

        egui::Grid::new("measure_other").spacing([S_LG, S_SM]).show(ui, |ui| {
            for (k, v) in [
                (i18n::measure_medium(), medium),
                (i18n::measure_route(), route),
                (i18n::measure_proxy(), proxy),
                (i18n::measure_dns(), dns),
                (i18n::measure_dns_public(), dns_public),
                (i18n::measure_tcp(), tcp),
                (i18n::measure_ipv6(), ipv6),
                (i18n::measure_baseline(), baseline),
                (i18n::measure_load(), load),
                (i18n::measure_syslog(), syslog),
            ] {
                ui.label(egui::RichText::new(k).size(T_BODY).color(FG_DIM));
                ui.label(egui::RichText::new(v).size(T_BODY).color(FG));
                ui.end_row();
            }
        });
    });
}

/// The lengths offered for the long measurement, in seconds. 0 is none.
const LONG_CHOICES: [usize; 3] = [0, 120, 300];

/// Where each second's trouble started, over the whole run, and the list of
/// drops with the times a provider will ask for.
fn long_card(run: &LongRun, ui: &mut egui::Ui) {
    card(ui, i18n::long_heading(), |ui| {
        // The strip: one row per link, one column per second.
        let rows = [
            (i18n::measure_leg_router(), Segment::Lan),
            (i18n::measure_leg_isp(), Segment::Isp),
            (i18n::measure_leg_internet(), Segment::Internet),
        ];
        let row_of = |seg: Segment| match seg {
            Segment::Lan => 0,
            Segment::Isp | Segment::Uplink => 1,
            _ => 2,
        };
        let label_w = 120.0;
        let row_h = 16.0;
        let w = ui.available_width();
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(w, row_h * 3.0 + 22.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let strip_w = (w - label_w).max(1.0);
        let n = run.ticks.len().max(1) as f32;
        let x_of = |s: usize| rect.left() + label_w + strip_w * s as f32 / n;

        for (i, (name, _)) in rows.iter().enumerate() {
            let top = rect.top() + i as f32 * row_h;
            painter.text(
                egui::pos2(rect.left(), top + row_h / 2.0),
                egui::Align2::LEFT_CENTER,
                *name,
                egui::FontId::proportional(super::T_META),
                FG_DIM,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + label_w, top + 2.0),
                    egui::pos2(rect.right(), top + row_h - 2.0),
                ),
                2.0,
                super::BG3,
            );
        }
        for e in &run.episodes {
            let top = rect.top() + row_of(e.segment) as f32 * row_h;
            let colour = if e.kind == Trouble::Loss { RED } else { YELLOW };
            // At five minutes a second is a pixel or two; a drop has to stay
            // visible however short it was.
            let (x0, x1) = (x_of(e.start_s), x_of(e.start_s + e.len_s).max(x_of(e.start_s) + 3.0));
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x0, top + 2.0),
                    egui::pos2(x1, top + row_h - 2.0),
                ),
                1.0,
                colour,
            );
        }
        let axis_y = rect.top() + row_h * 3.0 + 4.0;
        for (s, align) in [(0, egui::Align2::LEFT_TOP), (run.ticks.len(), egui::Align2::RIGHT_TOP)]
        {
            painter.text(
                egui::pos2(x_of(s), axis_y),
                align,
                diagnose::format_clock_s(run.started_at + s as f64),
                egui::FontId::monospace(super::T_MICRO),
                FG_DIM,
            );
        }
        ui.label(egui::RichText::new(i18n::long_rows_note()).size(super::T_MICRO).color(FG_DIM));
        ui.add_space(S_MD);

        if run.episodes.is_empty() {
            ui.label(egui::RichText::new(i18n::long_no_episodes()).size(T_BODY).color(FG));
            return;
        }
        // Worth listing in full up to a point; past it the strip above is
        // the better reading and the list only pushes the rest of the page.
        const SHOWN: usize = 15;
        egui::Grid::new("long_episodes").striped(true).spacing([S_LG, S_XS]).show(ui, |ui| {
            for head in [
                i18n::long_col_time(),
                i18n::long_col_what(),
                i18n::long_col_where(),
                i18n::long_col_len(),
            ] {
                ui.label(egui::RichText::new(head).size(super::T_META).color(FG_DIM));
            }
            ui.end_row();
            for e in run.episodes.iter().take(SHOWN) {
                let loss = e.kind == Trouble::Loss;
                let colour = match (loss, e.len_s) {
                    // A lone lost second is recorded, not accused.
                    (true, 1) => FG_DIM,
                    (true, _) => RED,
                    (false, _) => YELLOW,
                };
                ui.label(super::figure(
                    diagnose::format_clock_s(run.started_at + e.start_s as f64),
                    T_BODY,
                    FG,
                ));
                ui.label(
                    egui::RichText::new(i18n::long_episode_kind(loss)).size(T_BODY).color(colour),
                );
                ui.label(egui::RichText::new(e.segment.label()).size(T_BODY).color(FG));
                ui.label(super::figure(i18n::long_episode_len(e.len_s, e.worst_ms), T_BODY, FG));
                ui.end_row();
            }
        });
        if run.episodes.len() > SHOWN {
            ui.label(
                egui::RichText::new(format!("+{}", run.episodes.len() - SHOWN))
                    .size(super::T_META)
                    .color(FG_DIM),
            );
        }
    });
}

/// The verdict: one segment, how sure the scan is, what it costs, and the
/// short ordered plan. This is the part most users will read and nothing else.
fn verdict_card(app: &App, ui: &mut egui::Ui, jump_to: &mut Option<String>) {
    let v: &Verdict = &app.verdict;
    let colour = segment_colour(v.segment);

    let panel = egui::Frame::none()
        .fill(BG2)
        .rounding(6.0)
        .inner_margin(egui::Margin { left: S_LG + 4.0, right: S_LG, top: S_LG, bottom: S_LG })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let when = app.scan_at.map(diagnose::format_clock).unwrap_or_default();
            ui.label(
                egui::RichText::new(i18n::verdict_meta(&when, &scan_where(&app.scan_net)))
                    .size(T_META)
                    .color(FG_DIM),
            );
            ui.add_space(S_XS);
            ui.label(
                egui::RichText::new(i18n::verdict_confident(
                    v.segment.label(),
                    v.confidence.label(),
                ))
                .size(T_TITLE)
                .strong()
                .color(colour),
            );
            if let Some(split) = &v.split {
                ui.add_space(S_XS);
                ui.label(egui::RichText::new(split).size(T_BODY).color(FG_DIM));
            }
            ui.add_space(S_MD);
            ui.label(egui::RichText::new(&v.cost).size(T_HEAD).color(FG));

            if let Some(prev) = &app.prev_scan {
                ui.add_space(S_SM);
                ui.label(egui::RichText::new(since(prev, app)).size(T_META).color(FG_DIM));
            }

            if v.actions.is_empty() {
                return;
            }
            ui.add_space(S_MD);
            ui.label(
                egui::RichText::new(i18n::verdict_actions_heading())
                    .size(T_BODY)
                    .strong()
                    .color(FG_DIM),
            );
            for (n, action) in v.actions.iter().enumerate() {
                ui.add_space(S_SM);
                ui.horizontal_top(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{}.", n + 1))
                            .size(T_BODY)
                            .strong()
                            .color(colour),
                    );
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&action.text).size(T_BODY).color(FG));
                        if let Some(id) = &action.tweak_id {
                            ui.add_space(S_XS);
                            if button(ui, i18n::diag_btn_fix(), Emphasis::Secondary).clicked() {
                                *jump_to = Some(id.clone());
                            }
                        }
                    });
                });
            }
        });
    // Painted once the panel's size is known, like the other verdicts.
    let rect = panel.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(4.0, rect.height())),
        egui::Rounding { nw: 6.0, sw: 6.0, ne: 0.0, se: 0.0 },
        colour,
    );
    ui.add_space(S_MD);
}

/// The connection the scan ran on, as a person names it.
fn scan_where(net: &crate::probe::netstate::NetState) -> String {
    match net.medium {
        Medium::Wifi if !net.ssid.is_empty() => format!("Wi-Fi {}", net.ssid),
        _ => net.medium.label().to_string(),
    }
}

/// What the scan before this one found, and how the two pings moved since.
fn since(prev: &PrevScan, app: &App) -> String {
    let avg = |s: &Option<crate::store::Stats>| s.as_ref().and_then(|s| s.avg);
    let router = prev.router_ms.zip(avg(&app.measurements.gateway));
    let internet = prev.internet_ms.zip(avg(&app.measurements.internet));
    i18n::verdict_since(&diagnose::format_clock(prev.at), prev.segment.label(), router, internet)
}

fn open_tweak(app: &mut App, id: &str) {
    app.tab = super::Tab::Optimise;
    app.refresh_tweaks();
    app.selected_tweak = app.tweak_states.iter().position(|(tid, _, _)| tid == id);
}

fn start_scan(app: &mut App) {
    app.scanning = true;
    app.scan_steps.clear();
    app.scan_progress = 0.0;
    app.scan_label = i18n::diag_scan_starting().into();
    // The load test saturates the line. Anything the monitor sampled during it
    // would be measuring the test rather than the connection, and would then
    // pollute the very baseline the next scan compares against.
    if app.deep_scan {
        app.monitor.hold();
        app.scan_held = true;
    }

    let tx = app.tx.clone();
    let store = Arc::clone(&app.store);
    let settings = app.settings.clone();
    let net = app.net.clone();
    app.scan_net = net.clone();
    let deep = app.deep_scan;
    app.scan_cancel = Arc::default();
    let long = (app.long_secs > 0)
        .then(|| diagnose::LongOpts { secs: app.long_secs, cancel: Arc::clone(&app.scan_cancel) });

    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress: diagnose::Progress = Arc::new(move |label: &str, frac: f32| {
            let _ = progress_tx.send(Job::ScanProgress(label.to_string(), frac));
        });
        let scan = diagnose::scan(&net, &store, &settings, deep, long, Some(progress));
        let _ = tx.send(Job::ScanDone(Box::new(scan)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_reporting_its_progress_is_one_step_and_two_steps_are_two() {
        assert!(same_step("Long measurement: 0:05 of 5:00", "Long measurement: 0:06 of 5:00"));
        assert!(!same_step("Checking IPv6", "Checking MTU"));
        assert!(!same_step("Long measurement: 0:05 of 5:00", "Checking MTU"));
    }
}
