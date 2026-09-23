//! The top of the Live tab, for someone who does not know what a gateway is:
//! what is happening, which link it is on, and what to do about it.
//!
//! Everything below it on the tab is the evidence. This is the reading of it,
//! in the words a person would use on the phone to their provider, and it
//! claims no more than the verdict it is read from: a link nobody measured is
//! drawn as unknown, not as fine.

use eframe::egui;

use super::{
    button, status_colour, App, Emphasis, Tab, BG2, BG3, FG, FG_DIM, GREEN, RED, S_MD, S_SM, S_XS,
    T_BODY, T_META, T_TITLE, YELLOW,
};
use crate::i18n;
use crate::monitor::{Seen, Status};
use crate::store::Stats;

/// One link in the chain the app watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Good,
    Slow,
    Broken,
    /// Not measured, or not measurable while something nearer is broken.
    Unknown,
}

impl Link {
    fn colour(self) -> egui::Color32 {
        match self {
            Link::Good => GREEN,
            Link::Slow => YELLOW,
            Link::Broken => RED,
            Link::Unknown => BG3,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Link::Good => i18n::sum_link_good(),
            Link::Slow => i18n::sum_link_slow(),
            Link::Broken => i18n::sum_link_broken(),
            Link::Unknown => i18n::sum_link_unknown(),
        }
    }
}

/// What the one button under the advice does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Resume,
    Diagnose,
    Report,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// This computer to the router: the Wi-Fi or the cable.
    pub local: Link,
    /// The router to the internet: the provider.
    pub provider: Link,
    /// The one sentence that answers "is my internet working".
    pub say: &'static str,
    /// What to do about it, in steps a person can take without a manual.
    pub todo: &'static str,
    pub action: Option<Action>,
    pub colour: egui::Color32,
}

/// Fewest router pings in the window before its latency counts.
const ROUTER_MIN_SAMPLES: usize = 10;
/// Wi-Fi signal below this is weak enough to name, the same line the header
/// turns the signal red at.
const WEAK_SIGNAL_PCT: u32 = 34;

/// Reads the verdict into the summary. `router` is the last minute of pings
/// to the router, `router_slow_ms` the latency past which it counts as slow.
pub fn read(
    seen: Seen<'_>,
    wifi: bool,
    signal_pct: Option<u32>,
    router: &Stats,
    router_slow_ms: f64,
) -> Summary {
    let unknown = |say, todo, action| Summary {
        local: Link::Unknown,
        provider: Link::Unknown,
        say,
        todo,
        action,
        colour: FG_DIM,
    };
    let status = match seen {
        Seen::Paused => {
            return unknown(i18n::sum_paused(), i18n::sum_paused_todo(), Some(Action::Resume))
        }
        Seen::Waiting => return unknown(i18n::sum_waiting(), i18n::sum_waiting_todo(), None),
        Seen::Blind => return unknown(i18n::sum_blind(), i18n::sum_blind_todo(), None),
        Seen::Stale(_) => return unknown(i18n::sum_stale(), i18n::sum_stale_todo(), None),
        Seen::Verdict(status, _) => status,
    };

    // The local link is only called slow on evidence that lives on it: the
    // router itself answering slowly, or a weak signal. Loss on the router's
    // own replies is left out, because routers throttle those on purpose.
    let router_slow =
        router.count >= ROUTER_MIN_SAMPLES && router.avg.is_some_and(|avg| avg > router_slow_ms);
    let weak = wifi && signal_pct.is_some_and(|p| p < WEAK_SIGNAL_PCT);
    let local_quality = if router_slow || weak { Link::Slow } else { Link::Good };

    let colour = status_colour(status);
    let (local, provider, say, todo, action) = match status {
        Status::Ok => (Link::Good, Link::Good, i18n::sum_ok(), i18n::sum_ok_todo(), None),
        Status::Degraded => {
            // A local link that is slow explains it. One that is not means the
            // trouble is past the router: the router answers quickly and
            // cleanly while the internet does not.
            let provider = if local_quality == Link::Slow { Link::Good } else { Link::Slow };
            let todo = if weak {
                i18n::sum_slow_weak_todo()
            } else if local_quality == Link::Slow {
                i18n::sum_slow_local_todo()
            } else {
                i18n::sum_slow_todo()
            };
            (local_quality, provider, i18n::sum_slow(), todo, Some(Action::Diagnose))
        }
        Status::DnsFail => {
            (Link::Good, Link::Good, i18n::sum_dns(), i18n::sum_dns_todo(), Some(Action::Diagnose))
        }
        Status::IspDown => {
            (Link::Good, Link::Broken, i18n::sum_isp(), i18n::sum_isp_todo(), Some(Action::Report))
        }
        Status::LanDown => (
            Link::Broken,
            Link::Unknown,
            i18n::sum_lan(),
            if wifi { i18n::sum_lan_wifi_todo() } else { i18n::sum_lan_cable_todo() },
            None,
        ),
        Status::AdapterDown => (
            Link::Broken,
            Link::Unknown,
            i18n::sum_adapter(),
            if wifi { i18n::sum_adapter_wifi_todo() } else { i18n::sum_adapter_cable_todo() },
            None,
        ),
    };
    Summary { local, provider, say, todo, action, colour }
}

/// Draws the summary panel. `note` is the monitor's own explanation of the
/// reading, when it has one.
pub fn show(app: &mut App, ui: &mut egui::Ui, summary: &Summary, note: &str) {
    let wifi = app.net.medium == crate::probe::netstate::Medium::Wifi;
    egui::Frame::none().fill(BG2).rounding(8.0).inner_margin(egui::Margin::same(S_MD + S_XS)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                super::status_dot(ui, summary.colour, 6.0);
                ui.add_space(S_XS);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(summary.say).size(T_TITLE).strong().color(FG),
                    )
                    .wrap(),
                );
            });
            ui.add_space(S_MD);
            chain(ui, wifi, summary);
            ui.add_space(S_MD);
            ui.add(
                egui::Label::new(egui::RichText::new(summary.todo).size(T_BODY).color(FG)).wrap(),
            );
            if !note.is_empty() {
                ui.add_space(S_XS);
                ui.add(
                    egui::Label::new(egui::RichText::new(note).size(T_META).color(FG_DIM)).wrap(),
                );
            }
            if let Some(action) = summary.action {
                ui.add_space(S_SM);
                act(app, ui, action);
            }
        },
    );
}

fn act(app: &mut App, ui: &mut egui::Ui, action: Action) {
    // A pause held by a running load test or scan is not the user's to lift,
    // and a Resume that does nothing is worse than none.
    if action == Action::Resume && !app.monitor.is_paused() {
        return;
    }
    let label = match action {
        Action::Resume => i18n::live_btn_resume(),
        Action::Diagnose => i18n::sum_btn_diagnose(),
        Action::Report if app.report_busy => i18n::hist_report_saving(),
        Action::Report => i18n::sum_btn_report(),
    };
    if button(ui, label, Emphasis::Primary).clicked() {
        match action {
            Action::Resume => app.monitor.set_paused(false),
            Action::Diagnose => app.tab = Tab::Diagnose,
            Action::Report => super::report::save_in_background(app, super::report::Range::Day),
        }
    }
}

/// This computer, the router and the internet, joined by the two links the
/// verdict is about, each coloured and named by its state.
fn chain(ui: &mut egui::Ui, wifi: bool, s: &Summary) {
    let height = 58.0;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let p = ui.painter_at(rect);

    let label_font = egui::FontId::proportional(T_BODY);
    let small = egui::FontId::proportional(T_META);
    let y = rect.top() + 12.0;
    let xs = [rect.left(), rect.center().x, rect.right()];
    let nodes = [i18n::sum_node_pc(), i18n::sum_node_router(), i18n::sum_node_internet()];
    let aligns =
        [egui::Align2::LEFT_CENTER, egui::Align2::CENTER_CENTER, egui::Align2::RIGHT_CENTER];

    // Node labels first, so each bar can start where its label ends.
    let mut spans = [(0.0f32, 0.0f32); 3];
    for i in 0..3 {
        let r = p.text(egui::pos2(xs[i], y), aligns[i], nodes[i], label_font.clone(), FG);
        spans[i] = (r.left(), r.right());
    }

    let local_name = if wifi { i18n::sum_link_wifi() } else { i18n::sum_link_cable() };
    let links = [(s.local, local_name), (s.provider, i18n::sum_link_provider())];
    for (i, (link, name)) in links.iter().enumerate() {
        let from = spans[i].1 + S_SM;
        let to = spans[i + 1].0 - S_SM;
        if to <= from {
            continue;
        }
        let bar = egui::Rect::from_min_max(egui::pos2(from, y - 2.0), egui::pos2(to, y + 2.0));
        p.rect_filled(bar, 2.0, link.colour());
        if *link == Link::Broken {
            // A gap in the middle of the bar: broken reads as broken even to
            // someone who cannot tell red from green.
            let mid = bar.center().x;
            p.rect_filled(
                egui::Rect::from_center_size(egui::pos2(mid, y), egui::vec2(10.0, 12.0)),
                0.0,
                BG2,
            );
        }
        let caption = format!("{name}: {}", link.word());
        let colour = if *link == Link::Unknown { FG_DIM } else { link.colour() };
        p.text(
            egui::pos2((from + to) / 2.0, y + 22.0),
            egui::Align2::CENTER_CENTER,
            caption,
            small.clone(),
            colour,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router(avg: f64) -> Stats {
        Stats { count: 60, avg: Some(avg), ..Default::default() }
    }

    fn verdict(status: Status) -> Summary {
        read(Seen::Verdict(status, ""), true, Some(80), &router(3.0), 20.0)
    }

    #[test]
    fn each_outage_breaks_the_link_it_is_about_and_nothing_further() {
        let isp = verdict(Status::IspDown);
        assert_eq!((isp.local, isp.provider), (Link::Good, Link::Broken));
        assert_eq!(isp.action, Some(Action::Report));

        // Past a broken local link nothing is known about the provider.
        for s in [Status::LanDown, Status::AdapterDown] {
            let v = verdict(s);
            assert_eq!((v.local, v.provider), (Link::Broken, Link::Unknown), "{s:?}");
        }
        let ok = verdict(Status::Ok);
        assert_eq!((ok.local, ok.provider, ok.action), (Link::Good, Link::Good, None));
    }

    #[test]
    fn slowness_is_placed_only_where_the_evidence_is() {
        // Router quick, signal strong: the trouble is past the router.
        let past = verdict(Status::Degraded);
        assert_eq!((past.local, past.provider), (Link::Good, Link::Slow));
        // The router itself slow: the local link, and the provider is not accused.
        let local = read(Seen::Verdict(Status::Degraded, ""), true, Some(80), &router(45.0), 20.0);
        assert_eq!((local.local, local.provider), (Link::Slow, Link::Good));
        // A weak signal is local too.
        let weak = read(Seen::Verdict(Status::Degraded, ""), true, Some(20), &router(3.0), 20.0);
        assert_eq!(weak.local, Link::Slow);
        // Too few router pings to judge: not called slow on one reading.
        let few = Stats { count: 2, avg: Some(90.0), ..Default::default() };
        let v = read(Seen::Verdict(Status::Degraded, ""), false, None, &few, 20.0);
        assert_eq!(v.local, Link::Good);
    }

    #[test]
    fn nothing_measured_draws_nothing_as_fine() {
        for seen in [Seen::Paused, Seen::Waiting, Seen::Blind, Seen::Stale(60.0)] {
            let v = read(seen, true, Some(80), &router(3.0), 20.0);
            assert_eq!((v.local, v.provider), (Link::Unknown, Link::Unknown), "{seen:?}");
        }
        assert_eq!(read(Seen::Paused, true, None, &router(3.0), 20.0).action, Some(Action::Resume));
    }
}
