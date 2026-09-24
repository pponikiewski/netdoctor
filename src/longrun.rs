//! The long measurement: minutes of pings instead of seconds.
//!
//! The ordinary scan is a thirty-second window, and most "the internet keeps
//! dropping" happens a few times an hour. A clean scan of a line like that is
//! true and useless. This watches the same three links for minutes, one probe
//! per link per second, all on one clock, so that when something goes wrong
//! the readings from the same second can say where: a second in which the
//! router stopped answering is not the provider's fault, whatever the far end
//! did in it.
//!
//! Recording and reading are kept apart. [`record`] only collects; [`analyse`]
//! is pure and is where every rule lives, so the rules are tested on
//! constructed seconds rather than on whatever the network did that day.

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::diagnose::Segment;
use crate::probe::icmp::Pinger;
use crate::store::{self, Stats};

/// One second of the run: the round trip on each link, `None` when lost.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tick {
    pub gw: Option<f64>,
    /// `None` also when there was no edge hop to probe; see [`LongRun::edge`].
    pub edge: Option<f64>,
    /// The faster of the two anchors: the internet answered if either did.
    pub net: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    /// Packets went missing.
    Loss,
    /// Everything answered, but late enough to be felt.
    Spike,
}

/// A run of bad seconds, charged to the link where the trouble started.
#[derive(Debug, Clone, PartialEq)]
pub struct Episode {
    /// Seconds since the run started.
    pub start_s: usize,
    pub len_s: usize,
    pub kind: Trouble,
    pub segment: Segment,
    /// For a spike, how far above its usual the far end went.
    pub worst_ms: Option<f64>,
}

/// What the run found.
#[derive(Debug, Clone, Default)]
pub struct LongRun {
    /// How long it was meant to run, and how long it did.
    pub planned_s: usize,
    /// Wall-clock seconds at the first tick, so an episode can be given the
    /// time a provider's support line will ask for.
    pub started_at: f64,
    pub ticks: Vec<Tick>,
    pub gw: Option<Stats>,
    pub edge: Option<Stats>,
    pub net: Option<Stats>,
    pub episodes: Vec<Episode>,
    /// Stopped by the user before `planned_s`.
    pub cancelled: bool,
}

impl LongRun {
    /// The episodes that are worth a verdict. One lost second in five minutes
    /// is what every link does and names nothing.
    pub fn significant(&self) -> impl Iterator<Item = &Episode> {
        self.episodes.iter().filter(|e| match e.kind {
            Trouble::Loss => e.len_s >= MIN_LOSS_S,
            Trouble::Spike => true,
        })
    }

    /// The segment most of the significant trouble belongs to, the share of
    /// the bad seconds it accounts for, and how many episodes there were.
    ///
    /// Spikes alone need to recur: one late burst in five minutes is a
    /// download starting somewhere in the house.
    pub fn culprit(&self) -> Option<(Segment, f64, usize)> {
        let found: Vec<&Episode> = self.significant().collect();
        let losses = found.iter().filter(|e| e.kind == Trouble::Loss).count();
        if losses == 0 && found.len() < MIN_SPIKES {
            return None;
        }
        let total: usize = found.iter().map(|e| e.len_s).sum();
        let mut by_seg: Vec<(Segment, usize)> = Vec::new();
        for e in &found {
            match by_seg.iter_mut().find(|(s, _)| *s == e.segment) {
                Some((_, n)) => *n += e.len_s,
                None => by_seg.push((e.segment, e.len_s)),
            }
        }
        // Ties go to the link nearest the user, as everywhere else: trouble
        // that is present at the router did not come from further out.
        let (seg, secs) = by_seg.into_iter().max_by_key(|(s, n)| (*n, nearness(*s)))?;
        Some((seg, secs as f64 / total.max(1) as f64, found.len()))
    }
}

/// Higher is nearer the user.
fn nearness(s: Segment) -> u8 {
    match s {
        Segment::Lan => 3,
        Segment::Isp => 2,
        _ => 1,
    }
}

/// Consecutive lost seconds before a loss is a drop rather than noise.
const MIN_LOSS_S: usize = 2;
/// Spike episodes needed, with no loss at all, before they name a link.
const MIN_SPIKES: usize = 3;
/// How far above the link's own median a round trip has to go to count as a
/// spike. Both conditions: 50 ms is felt in a call, and doubling a 5 ms line
/// to 10 is not.
const SPIKE_ABOVE_MS: f64 = 50.0;
const SPIKE_RATIO: f64 = 2.0;

/// Read the run. `edge_local` is the scan's finding that the hop past the
/// router is still the user's own equipment, whose trouble is the LAN's.
pub fn analyse(ticks: Vec<Tick>, planned_s: usize, edge_local: bool, cancelled: bool) -> LongRun {
    let stats = |pick: fn(&Tick) -> Option<f64>| {
        let rtts: Vec<f64> = ticks.iter().filter_map(pick).collect();
        // A link that never answered once was not probed, or refuses ICMP;
        // either way it has no statistics to show, only silence.
        (!rtts.is_empty()).then(|| store::summarise(ticks.len(), &rtts))
    };
    let gw = stats(|t| t.gw);
    let edge = stats(|t| t.edge);
    let net = stats(|t| t.net);
    let median = |s: &Option<Stats>| s.as_ref().and_then(|s| s.median);
    let (gw_med, edge_med, net_med) = (median(&gw), median(&edge), median(&net));
    let edge_seg = if edge_local { Segment::Lan } else { Segment::Isp };

    let classify = |t: &Tick| -> Option<(Trouble, Segment, Option<f64>)> {
        // Loss is charged to the first link that shows it. The edge is only
        // believed when the far end lost the same second: routers drop pings
        // addressed to themselves long before they drop traffic.
        if gw_med.is_some() && t.gw.is_none() {
            return Some((Trouble::Loss, Segment::Lan, None));
        }
        if net_med.is_some() && t.net.is_none() {
            let seg = match (edge_med, t.edge) {
                // The provider's edge answered while both anchors did not:
                // the break is past it.
                (Some(_), Some(_)) => Segment::Internet,
                (Some(_), None) => edge_seg,
                // Without an edge to split on, two independent operators
                // going silent together is the provider's line.
                (None, _) => Segment::Isp,
            };
            return Some((Trouble::Loss, seg, None));
        }

        let (net_rtt, net_med) = (t.net?, net_med?);
        let rise = net_rtt - net_med;
        if rise < SPIKE_ABOVE_MS || net_rtt < net_med * SPIKE_RATIO {
            return None;
        }
        // Where the extra milliseconds first appear. Half the rise already
        // present at a link means that link carries it.
        let rise_at = |rtt: Option<f64>, med: Option<f64>| match (rtt, med) {
            (Some(r), Some(m)) => r - m,
            _ => 0.0,
        };
        let seg = if rise_at(t.gw, gw_med) >= rise / 2.0 {
            Segment::Lan
        } else if rise_at(t.edge, edge_med) >= rise / 2.0 {
            edge_seg
        } else {
            Segment::Internet
        };
        Some((Trouble::Spike, seg, Some(rise)))
    };

    let mut episodes: Vec<Episode> = Vec::new();
    for (i, t) in ticks.iter().enumerate() {
        let Some((kind, segment, rise)) = classify(t) else { continue };
        match episodes.last_mut() {
            // One good second inside a run of bad ones does not end it: a
            // flapping link is one problem, not five.
            Some(e) if e.kind == kind && e.segment == segment && i <= e.start_s + e.len_s + 1 => {
                e.len_s = i + 1 - e.start_s;
                e.worst_ms = match (e.worst_ms, rise) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
            }
            _ => episodes.push(Episode { start_s: i, len_s: 1, kind, segment, worst_ms: rise }),
        }
    }

    LongRun { planned_s, started_at: 0.0, ticks, gw, edge, net, episodes, cancelled }
}

pub type Progress = Arc<dyn Fn(usize, usize) + Send + Sync>;

/// Probe every link once a second for `secs` seconds, or until `cancel`.
///
/// Each link has its own thread and its own ICMP handle, and every thread
/// sends on the same schedule (start + n seconds), so tick `n` of each link
/// is the same second. A thread that falls behind a timeout does not drift:
/// it skips to the next slot, and the slot it missed is a lost reply.
///
/// `Err` when no ICMP handle could be opened; the run then has nothing to say.
pub fn record(
    gw: Option<Ipv4Addr>,
    edge: Option<Ipv4Addr>,
    anchors: [Ipv4Addr; 2],
    secs: usize,
    timeout_ms: u32,
    cancel: Arc<AtomicBool>,
    progress: Option<Progress>,
) -> Result<Vec<Tick>, String> {
    let start = Instant::now();
    // A probe has to come back before the next one is due.
    let timeout_ms = timeout_ms.min(900);
    let series = |addr: Option<Ipv4Addr>| -> Result<Vec<Option<f64>>, String> {
        let Some(addr) = addr else { return Ok(Vec::new()) };
        let pinger = Pinger::new().map_err(|e| e.describe())?;
        let mut out = Vec::with_capacity(secs);
        for n in 0..secs {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let due = start + Duration::from_secs(n as u64);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            out.push(pinger.ping(addr, timeout_ms).rtt_ms);
        }
        Ok(out)
    };

    let (gw_s, edge_s, a_s, b_s) = std::thread::scope(|s| {
        let gw_h = s.spawn(|| series(gw));
        let edge_h = s.spawn(|| series(edge));
        let a_h = s.spawn(|| series(Some(anchors[0])));
        let b_h = s.spawn(|| series(Some(anchors[1])));
        // The progress reports come from here, not from the probing threads,
        // so a slow link cannot hold the bar back.
        if let Some(p) = &progress {
            while !a_h.is_finished() {
                p(start.elapsed().as_secs() as usize, secs);
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        let join = |h: std::thread::ScopedJoinHandle<'_, Result<Vec<Option<f64>>, String>>| {
            h.join().unwrap_or_else(|_| Err("probe thread died".into()))
        };
        (join(gw_h), join(edge_h), join(a_h), join(b_h))
    });
    let (gw_s, edge_s, a_s, b_s) = (gw_s?, edge_s?, a_s?, b_s?);

    // A cancelled run stops every series at about the same second; the
    // shortest decides how many whole seconds there are.
    let len = [&a_s, &b_s]
        .iter()
        .map(|v| v.len())
        .chain([&gw_s, &edge_s].iter().filter(|v| !v.is_empty()).map(|v| v.len()))
        .min()
        .unwrap_or(0);
    let at = |v: &Vec<Option<f64>>, i: usize| v.get(i).copied().flatten();
    Ok((0..len)
        .map(|i| Tick {
            gw: at(&gw_s, i),
            edge: at(&edge_s, i),
            net: match (at(&a_s, i), at(&b_s, i)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Tick {
        Tick { gw: Some(3.0), edge: Some(8.0), net: Some(15.0) }
    }

    fn run(ticks: Vec<Tick>) -> LongRun {
        let n = ticks.len();
        analyse(ticks, n, false, false)
    }

    fn with(n: usize, at: &[(usize, Tick)]) -> Vec<Tick> {
        let mut v = vec![ok(); n];
        for (i, t) in at {
            v[*i] = *t;
        }
        v
    }

    #[test]
    fn a_quiet_line_has_no_episodes_and_no_culprit() {
        let r = run(vec![ok(); 120]);
        assert!(r.episodes.is_empty());
        assert_eq!(r.culprit(), None);
        assert_eq!(r.net.as_ref().map(|s| s.count), Some(120));
    }

    #[test]
    fn loss_at_the_router_is_the_lans_even_when_everything_past_it_is_lost_too() {
        let dead = Tick { gw: None, edge: None, net: None };
        let r = run(with(60, &[(10, dead), (11, dead), (12, dead)]));
        assert_eq!(r.episodes.len(), 1);
        assert_eq!(r.episodes[0].segment, Segment::Lan);
        assert_eq!((r.episodes[0].start_s, r.episodes[0].len_s), (10, 3));
        assert_eq!(r.culprit(), Some((Segment::Lan, 1.0, 1)));
    }

    #[test]
    fn loss_past_the_router_is_split_on_the_providers_edge() {
        let isp = Tick { gw: Some(3.0), edge: None, net: None };
        let far = Tick { gw: Some(3.0), edge: Some(8.0), net: None };
        let r = run(with(60, &[(5, isp), (6, isp), (30, far), (31, far)]));
        let segs: Vec<Segment> = r.episodes.iter().map(|e| e.segment).collect();
        assert_eq!(segs, vec![Segment::Isp, Segment::Internet]);
    }

    #[test]
    fn a_second_router_of_the_users_own_keeps_its_loss_in_the_lan() {
        let t = Tick { gw: Some(3.0), edge: None, net: None };
        let r = analyse(with(60, &[(5, t), (6, t)]), 60, true, false);
        assert_eq!(r.episodes[0].segment, Segment::Lan);
    }

    #[test]
    fn an_edge_that_ignores_its_own_pings_is_not_loss() {
        // The far end answered every second; the edge's silence is a filter.
        let t = Tick { gw: Some(3.0), edge: None, net: Some(15.0) };
        let r = run(with(60, &[(5, t), (6, t), (7, t)]));
        assert!(r.episodes.is_empty());
    }

    #[test]
    fn one_lost_second_is_recorded_but_names_nothing() {
        let t = Tick { gw: Some(3.0), edge: Some(8.0), net: None };
        let r = run(with(300, &[(100, t)]));
        assert_eq!(r.episodes.len(), 1);
        assert_eq!(r.significant().count(), 0);
        assert_eq!(r.culprit(), None);
    }

    #[test]
    fn one_good_second_inside_a_drop_does_not_split_it() {
        let t = Tick { gw: Some(3.0), edge: Some(8.0), net: None };
        let r = run(with(60, &[(10, t), (11, t), (13, t), (14, t)]));
        assert_eq!(r.episodes.len(), 1);
        assert_eq!(r.episodes[0].len_s, 5);
    }

    #[test]
    fn a_spike_is_charged_to_the_link_where_the_extra_time_appears() {
        // +100 ms at the far end, already +90 at the router: the air, not
        // the provider.
        let air = Tick { gw: Some(93.0), edge: Some(98.0), net: Some(115.0) };
        // +100 ms that only appears past the provider's edge.
        let far = Tick { gw: Some(3.0), edge: Some(9.0), net: Some(115.0) };
        let r = run(with(120, &[(10, air), (60, far)]));
        let got: Vec<(Trouble, Segment)> = r.episodes.iter().map(|e| (e.kind, e.segment)).collect();
        assert_eq!(got, vec![(Trouble::Spike, Segment::Lan), (Trouble::Spike, Segment::Internet)]);
        assert_eq!(r.episodes[0].worst_ms, Some(100.0));
    }

    #[test]
    fn a_small_rise_on_a_fast_line_is_not_a_spike() {
        // 15 -> 45 ms: triple, but under the 50 ms anyone would feel.
        let t = Tick { gw: Some(3.0), edge: Some(8.0), net: Some(45.0) };
        assert!(run(with(60, &[(10, t)])).episodes.is_empty());
    }

    #[test]
    fn spikes_alone_name_a_link_only_when_they_recur() {
        let air = Tick { gw: Some(93.0), edge: Some(98.0), net: Some(115.0) };
        assert_eq!(run(with(120, &[(10, air), (60, air)])).culprit(), None);
        let r = run(with(120, &[(10, air), (40, air), (90, air)]));
        assert_eq!(r.culprit(), Some((Segment::Lan, 1.0, 3)));
    }

    #[test]
    fn the_culprit_is_where_most_of_the_bad_seconds_were() {
        let lan = Tick { gw: None, edge: None, net: None };
        let isp = Tick { gw: Some(3.0), edge: None, net: None };
        let r = run(with(
            120,
            &[(5, lan), (6, lan), (40, isp), (41, isp), (42, isp), (43, isp), (80, isp), (81, isp)],
        ));
        let (seg, share, n) = r.culprit().unwrap();
        assert_eq!((seg, n), (Segment::Isp, 3));
        assert!((share - 0.75).abs() < 1e-9, "{share}");
    }

    #[test]
    fn a_link_that_never_answered_has_no_statistics_and_no_loss() {
        // No gateway pings at all (a router that drops them): its silence is
        // not charged as loss every second.
        let t = Tick { gw: None, edge: None, net: Some(15.0) };
        let r = run(vec![t; 60]);
        assert!(r.gw.is_none());
        assert!(r.episodes.is_empty());
    }

    /// Goes to the network for ten seconds: `cargo test -- --ignored ten_seconds`.
    #[test]
    #[ignore]
    fn ten_seconds_of_the_live_line() {
        let net = crate::probe::netstate::read();
        let ticks = record(
            net.gateway,
            None,
            [crate::diagnose::ANCHOR, crate::diagnose::ANCHOR_ALT],
            10,
            1000,
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .unwrap();
        println!("{ticks:?}");
        assert_eq!(ticks.len(), 10);
        assert!(ticks.iter().any(|t| t.net.is_some()));
    }
}
