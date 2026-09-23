//! Which hop the loss starts at.
//!
//! The monitor can already say the connection broke past the router, and for
//! a lot of people that is where the investigation stops: "it is the ISP" is
//! a claim, not evidence, and the provider's first move is to blame the
//! Wi-Fi. What settles it is a per-hop measurement — loss that is absent at
//! the router, absent at the edge, and present from the third hop onwards
//! names the box that is dropping packets, and that box has an owner.
//!
//! So the path to a fixed anchor is walked once every few minutes and then
//! each hop on it is pinged directly on a slow cadence, alongside the ordinary
//! sweep. What comes out is the same picture `mtr` draws, kept continuously
//! rather than run once after the fact — which matters, because the outage
//! worth measuring is never happening while you are typing the command.
//!
//! # The one reading everybody gets wrong
//!
//! A router that shows 60% loss while every hop behind it shows none is not
//! broken and is not dropping your traffic. It is rate-limiting the ICMP
//! replies it generates itself, which is a configuration choice, not a fault,
//! and forwarded traffic never touches that path. Loss only means something
//! when it persists through every hop after it — see [`blame`], which refuses
//! to name a hop on any weaker evidence.

use std::collections::VecDeque;
use std::net::Ipv4Addr;

use crate::probe::icmp::{self, Pinger};

/// Who a hop most likely belongs to. The distinction is the difference
/// between a complaint worth making and one that will be dismissed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Owner {
    /// The router this machine is configured to use.
    Gateway,
    /// A private address past the gateway. It can be the household's own
    /// second router, mesh node or ISP box in router mode, or the provider's
    /// access network, which is often numbered privately; the address alone
    /// does not say which. (Named `Local` because it is stored in outage
    /// contexts under that name.)
    Local,
    /// Carrier-grade NAT, or the first public address. The provider's access
    /// network.
    Edge,
    /// Public addresses beyond the edge: the provider's core, its peers, the
    /// wider internet.
    Internet,
}

impl Owner {
    /// Whether a fault here is the user's own to fix, or `None` when the
    /// address cannot tell: see [`Owner::Local`].
    pub fn is_mine(&self) -> Option<bool> {
        match self {
            Owner::Gateway => Some(true),
            Owner::Local => None,
            Owner::Edge | Owner::Internet => Some(false),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hop {
    /// Distance from this machine, which is also the TTL that revealed it.
    pub ttl: u32,
    pub addr: Ipv4Addr,
    pub owner: Owner,
}

/// The measured path, as last walked.
#[derive(Debug, Clone, Default)]
pub struct Path {
    pub hops: Vec<Hop>,
    /// When the walk was taken, so a stale path can be noticed.
    pub discovered: f64,
    /// Where the walk was headed. Only a hop at this address has nothing
    /// behind it by nature; any other last hop is where the walk ran out.
    pub dest: Option<Ipv4Addr>,
}

/// How far to walk. Past a dozen hops the addresses belong to networks nobody
/// reading this can influence, and every extra hop is another second spent
/// waiting on a router that will not answer.
pub const MAX_HOPS: u32 = 12;

/// Walks the path to `dest` and labels each hop with who owns it.
///
/// Hops that never answer are dropped rather than kept as gaps: a router that
/// does not reply to a TTL expiry cannot be measured, and carrying it as a row
/// of dashes would imply it was tested and found wanting.
pub fn discover(dest: Ipv4Addr, gateway: Option<Ipv4Addr>, timeout_ms: u32) -> Path {
    let walked = icmp::traceroute(dest, MAX_HOPS, timeout_ms);
    let mut hops: Vec<Hop> = Vec::new();
    let mut past_private = false;

    for h in walked {
        let Some(addr) = h.addr else { continue };
        // A path that loops back through an address already seen is a sign of
        // the walk going wrong, not of a real topology worth measuring.
        if hops.iter().any(|existing| existing.addr == addr) {
            continue;
        }

        let owner = if Some(addr) == gateway {
            Owner::Gateway
        } else if addr.is_private() || crate::diagnose::is_apipa(addr) {
            Owner::Local
        } else if is_cgnat(addr) || !past_private {
            // The first public address after the private stretch is the
            // provider's edge; everything past it is somebody else's network.
            past_private = true;
            Owner::Edge
        } else {
            Owner::Internet
        };
        if matches!(owner, Owner::Edge) {
            past_private = true;
        }

        hops.push(Hop { ttl: h.hop, addr, owner });
    }

    Path { hops, discovered: crate::store::now(), dest: Some(dest) }
}

/// Addresses in 100.64.0.0/10: the provider's own NAT, not the user's.
fn is_cgnat(a: Ipv4Addr) -> bool {
    let o = a.octets();
    o[0] == 100 && (64..128).contains(&o[1])
}

// ---------------------------------------------------------------------------
// measuring the path that was found
// ---------------------------------------------------------------------------

/// How many probes per hop are kept. At one probe every few sweeps this is a
/// couple of minutes of history, which is long enough for a loss figure to
/// mean something and short enough to follow a path that changes.
pub const WINDOW: usize = 40;

/// A hop's rolling history, and the summary read off it.
#[derive(Debug, Clone, Default)]
pub struct HopStats {
    samples: VecDeque<Option<f64>>,
    /// Whether a direct probe to this hop has *ever* been answered.
    ///
    /// Sticky, and deliberately not part of the rolling window. Plenty of
    /// routers answer a TTL expiry — which is how the walk found them — while
    /// dropping every echo addressed to themselves. Measured naively that
    /// reads as a hop losing 100% of its packets, which is alarming, wrong,
    /// and the loudest thing in the table. A hop that has never once replied
    /// is not losing anything: it is declining to be measured, and the two
    /// have to be said differently. A hop that used to reply and has stopped
    /// keeps this flag and is reported as the failure it is.
    ever_replied: bool,
}

impl HopStats {
    fn push(&mut self, rtt: Option<f64>) {
        if rtt.is_some() {
            self.ever_replied = true;
        }
        if self.samples.len() == WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(rtt);
    }

    /// Whether this hop has answered a direct probe at any point.
    pub fn answers(&self) -> bool {
        self.ever_replied
    }

    pub fn count(&self) -> usize {
        self.samples.len()
    }

    pub fn loss_pct(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let lost = self.samples.iter().filter(|s| s.is_none()).count();
        lost as f64 / self.samples.len() as f64 * 100.0
    }

    pub fn avg_ms(&self) -> Option<f64> {
        let v: Vec<f64> = self.samples.iter().flatten().copied().collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    }
}

/// One row of the path table: a hop and how it is currently behaving.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HopReading {
    pub ttl: u32,
    pub addr: Ipv4Addr,
    pub owner: Owner,
    pub loss_pct: f64,
    pub avg_ms: Option<f64>,
    pub samples: usize,
    /// The hop has never answered a probe addressed to it, so its loss figure
    /// measures its ICMP policy rather than the connection. Such a hop is
    /// shown as unmeasurable and is kept out of every verdict.
    pub silent: bool,
}

/// The whole path, probed and summarised.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PathReading {
    pub hops: Vec<HopReading>,
    pub blame: Option<Blame>,
}

/// The hop the trouble starts at.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Blame {
    pub ttl: u32,
    pub addr: Ipv4Addr,
    pub owner: Owner,
    pub loss_pct: f64,
    /// Set when the hop is not losing packets but is where the delay appears.
    pub added_ms: Option<f64>,
}

/// Loss below this is ordinary internet weather, not a fault.
const LOSS_FLOOR: f64 = 8.0;

/// How much of the upstream loss has to survive to the end of the path before
/// a hop is blamed for it. Not all of it: the hops behind a genuinely lossy
/// one measure a little differently, and demanding an exact match would let
/// every real fault escape on a rounding difference.
const LOSS_CARRY: f64 = 0.6;

/// Extra milliseconds at one hop, sustained to the end, before the delay is
/// pinned there rather than treated as that router answering slowly.
const DELAY_FLOOR_MS: f64 = 40.0;

/// Fewest probes a hop needs before it is allowed to accuse anybody.
const MIN_SAMPLES: usize = 8;

/// Names the first hop where the trouble begins and stays.
///
/// The test that matters is not "which hop shows loss" — that hop is usually
/// a router rate-limiting its own ICMP replies, and blaming it is the classic
/// misreading of a traceroute. The test is whether the loss *carries*: a hop
/// is only responsible if the hops behind it are losing too. A spike that
/// clears at the very next hop measured nothing but that one router's
/// willingness to answer.
///
/// `dest` is where the path was walked to. A hop with nothing measurable
/// behind it is only taken at its word when it is that destination: a walk
/// that ran out at `MAX_HOPS`, or ended at a silent target, leaves a router
/// rate-limiting its own replies as the last one visible, and that is not
/// where the loss starts.
pub fn blame(hops: &[HopReading], dest: Option<Ipv4Addr>) -> Option<Blame> {
    let usable: Vec<&HopReading> =
        hops.iter().filter(|h| h.samples >= MIN_SAMPLES && !h.silent).collect();
    if usable.len() < 2 {
        return None;
    }

    for (i, h) in usable.iter().enumerate() {
        if h.loss_pct < LOSS_FLOOR {
            continue;
        }
        let downstream = &usable[i + 1..];
        // The destination has nothing behind it to corroborate with, so it
        // is taken at face value: loss at the far end is loss at the far end.
        let carries = if downstream.is_empty() {
            Some(h.addr) == dest
        } else {
            downstream.iter().all(|d| d.loss_pct >= h.loss_pct * LOSS_CARRY)
        };
        if carries {
            return Some(Blame {
                ttl: h.ttl,
                addr: h.addr,
                owner: h.owner,
                loss_pct: h.loss_pct,
                added_ms: None,
            });
        }
    }

    // No hop is losing packets, so look for where the time goes instead. The
    // same corroboration rule applies: a router that takes 80 ms to answer
    // while everything behind it answers in 20 is busy, not slow to forward.
    let mut previous = 0.0;
    for (i, h) in usable.iter().enumerate() {
        let Some(avg) = h.avg_ms else { continue };
        let added = avg - previous;
        previous = avg;
        if added < DELAY_FLOOR_MS {
            continue;
        }
        let downstream = &usable[i + 1..];
        let carries = if downstream.is_empty() {
            Some(h.addr) == dest
        } else {
            downstream.iter().filter_map(|d| d.avg_ms).all(|d| d >= avg * 0.8)
        };
        if carries {
            return Some(Blame {
                ttl: h.ttl,
                addr: h.addr,
                owner: h.owner,
                loss_pct: h.loss_pct,
                added_ms: Some(added),
            });
        }
    }

    None
}

/// The live state: the path, and a rolling history per hop address.
///
/// History is keyed by address rather than by position, so a path that
/// re-routes keeps whatever it already knew about the hops that stayed and
/// starts fresh only on the ones that are genuinely new.
#[derive(Default)]
pub struct Tracker {
    path: Path,
    stats: Vec<(Ipv4Addr, HopStats)>,
}

impl Tracker {
    pub fn path_age(&self, now: f64) -> f64 {
        if self.path.hops.is_empty() {
            return f64::INFINITY;
        }
        now - self.path.discovered
    }

    pub fn set_path(&mut self, path: Path) {
        self.stats.retain(|(addr, _)| path.hops.iter().any(|h| h.addr == *addr));
        self.path = path;
    }

    pub fn is_empty(&self) -> bool {
        self.path.hops.is_empty()
    }

    /// Pings every hop once and folds the result into the rolling history.
    pub fn probe(&mut self, pinger: &Pinger, timeout_ms: u32) {
        for hop in &self.path.hops {
            let r = pinger.ping(hop.addr, timeout_ms);
            let rtt = r.ok().then_some(r.rtt_ms).flatten();
            match self.stats.iter_mut().find(|(a, _)| *a == hop.addr) {
                Some((_, s)) => s.push(rtt),
                None => {
                    let mut s = HopStats::default();
                    s.push(rtt);
                    self.stats.push((hop.addr, s));
                }
            }
        }
    }

    /// The table and the verdict, as they stand.
    pub fn reading(&self) -> PathReading {
        let hops: Vec<HopReading> = self
            .path
            .hops
            .iter()
            .map(|h| {
                let s = self.stats.iter().find(|(a, _)| *a == h.addr).map(|(_, s)| s);
                let samples = s.map(|s| s.count()).unwrap_or(0);
                HopReading {
                    ttl: h.ttl,
                    addr: h.addr,
                    owner: h.owner,
                    loss_pct: s.map(|s| s.loss_pct()).unwrap_or(0.0),
                    avg_ms: s.and_then(|s| s.avg_ms()),
                    samples,
                    silent: samples >= MIN_SAMPLES && !s.map(|s| s.answers()).unwrap_or(false),
                }
            })
            .collect();
        let blame = blame(&hops, self.path.dest);
        PathReading { hops, blame }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(ttl: u32, last_octet: u8, loss: f64, avg: f64, owner: Owner) -> HopReading {
        HopReading {
            ttl,
            addr: Ipv4Addr::new(10, 0, 0, last_octet),
            owner,
            loss_pct: loss,
            avg_ms: Some(avg),
            samples: 30,
            silent: false,
        }
    }

    #[test]
    fn loss_that_clears_at_the_next_hop_accuses_nobody() {
        // The textbook false positive: one router rate-limits the replies it
        // generates itself, and every packet it forwards is fine.
        let path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 60.0, 9.0, Owner::Edge),
            hop(3, 3, 0.0, 14.0, Owner::Internet),
            hop(4, 4, 0.0, 16.0, Owner::Internet),
        ];
        assert_eq!(blame(&path, None), None, "a rate-limited router is not a fault");
    }

    #[test]
    fn loss_that_carries_to_the_end_names_the_hop_it_started_at() {
        let path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 0.0, 9.0, Owner::Edge),
            hop(3, 3, 40.0, 14.0, Owner::Internet),
            hop(4, 4, 38.0, 16.0, Owner::Internet),
            hop(5, 5, 45.0, 18.0, Owner::Internet),
        ];
        let b = blame(&path, None).expect("persistent loss has a source");
        assert_eq!(b.ttl, 3);
        assert_eq!(b.owner, Owner::Internet);
        assert!(b.added_ms.is_none(), "this is a loss verdict, not a latency one");
    }

    #[test]
    fn the_first_lossy_hop_is_blamed_and_not_the_worst_one() {
        let path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 30.0, 9.0, Owner::Edge),
            hop(3, 3, 80.0, 14.0, Owner::Internet),
        ];
        assert_eq!(blame(&path, None).unwrap().ttl, 2, "loss enters the path at hop 2");
    }

    #[test]
    fn the_last_visible_hop_is_blamed_only_when_it_is_the_destination() {
        // The walk ran out (twelve hops, or a target that never answers) at a
        // router that rate-limits its own replies. Nothing behind it can
        // confirm the loss, so nothing is accused.
        let path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 0.0, 9.0, Owner::Edge),
            hop(3, 3, 30.0, 14.0, Owner::Internet),
        ];
        assert_eq!(blame(&path, None), None);
        assert_eq!(blame(&path, Some(Ipv4Addr::new(1, 1, 1, 1))), None);
        // The same loss at the destination itself is loss at the far end.
        let b = blame(&path, Some(Ipv4Addr::new(10, 0, 0, 3))).expect("the target is losing");
        assert_eq!(b.ttl, 3);
    }

    #[test]
    fn a_clean_path_is_left_alone() {
        let path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 2.0, 9.0, Owner::Edge),
            hop(3, 3, 0.0, 14.0, Owner::Internet),
        ];
        assert_eq!(blame(&path, None), None);
    }

    #[test]
    fn a_hop_that_adds_the_delay_is_found_when_nothing_is_losing() {
        let path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 0.0, 8.0, Owner::Edge),
            hop(3, 3, 0.0, 95.0, Owner::Internet),
            hop(4, 4, 0.0, 99.0, Owner::Internet),
        ];
        let b = blame(&path, None).expect("the delay has a source too");
        assert_eq!(b.ttl, 3);
        assert!(b.added_ms.unwrap() > 80.0);
    }

    #[test]
    fn a_slow_answer_that_does_not_carry_is_the_router_being_busy() {
        let path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 0.0, 90.0, Owner::Edge),
            hop(3, 3, 0.0, 14.0, Owner::Internet),
        ];
        assert_eq!(blame(&path, None), None, "the hops behind it are fast, so nothing is slow");
    }

    #[test]
    fn a_path_too_fresh_to_have_been_measured_says_nothing() {
        let mut path = [hop(1, 1, 100.0, 5.0, Owner::Gateway), hop(2, 2, 100.0, 9.0, Owner::Edge)];
        for h in &mut path {
            h.samples = 3;
        }
        assert_eq!(blame(&path, None), None, "three probes is not a loss measurement");
    }

    #[test]
    fn a_hop_that_never_answers_is_unmeasurable_rather_than_lossy() {
        // Observed on a real line: two carrier routers answer the TTL expiry
        // that reveals them and drop every echo addressed to themselves,
        // while the destination behind them replies perfectly.
        let mut path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 100.0, 0.0, Owner::Internet),
            hop(3, 3, 0.0, 18.0, Owner::Internet),
        ];
        path[1].silent = true;
        path[1].avg_ms = None;
        assert_eq!(
            blame(&path, None),
            None,
            "a router that declines to answer is not dropping traffic"
        );
    }

    #[test]
    fn a_hop_that_stops_answering_is_still_a_failure() {
        // Same 100% figure, but this one used to reply, so the silence is the
        // fault rather than the policy.
        let path = [
            hop(1, 1, 0.0, 2.0, Owner::Gateway),
            hop(2, 2, 100.0, 0.0, Owner::Edge),
            hop(3, 3, 100.0, 0.0, Owner::Internet),
        ];
        assert_eq!(blame(&path, None).unwrap().ttl, 2);
    }

    #[test]
    fn silence_is_sticky_and_a_single_reply_clears_it_for_good() {
        let mut s = HopStats::default();
        for _ in 0..20 {
            s.push(None);
        }
        assert!(!s.answers());
        s.push(Some(12.0));
        for _ in 0..WINDOW {
            s.push(None);
        }
        assert!(
            s.answers(),
            "a hop that replied once answers echoes, so its later loss is a real measurement"
        );
    }

    #[test]
    fn stats_treat_a_missing_reply_as_loss_and_not_as_zero() {
        let mut s = HopStats::default();
        s.push(Some(10.0));
        s.push(None);
        s.push(Some(20.0));
        s.push(Some(30.0));
        assert_eq!(s.loss_pct(), 25.0);
        assert_eq!(s.avg_ms(), Some(20.0), "the lost probe must not drag the average to zero");
    }

    #[test]
    fn the_window_forgets_the_oldest_probe() {
        let mut s = HopStats::default();
        for _ in 0..WINDOW {
            s.push(None);
        }
        assert_eq!(s.loss_pct(), 100.0);
        for _ in 0..WINDOW {
            s.push(Some(5.0));
        }
        assert_eq!(s.count(), WINDOW);
        assert_eq!(s.loss_pct(), 0.0, "a recovered path must stop reporting the old loss");
    }

    #[test]
    fn re_routing_keeps_what_was_known_about_the_hops_that_stayed() {
        let mut t = Tracker::default();
        let a = Ipv4Addr::new(10, 0, 0, 1);
        let b = Ipv4Addr::new(10, 0, 0, 2);
        t.set_path(Path {
            hops: vec![
                Hop { ttl: 1, addr: a, owner: Owner::Gateway },
                Hop { ttl: 2, addr: b, owner: Owner::Edge },
            ],
            discovered: 0.0,
            dest: None,
        });
        t.stats.push((a, HopStats::default()));
        t.stats.push((b, HopStats::default()));

        // The path re-routes and hop 2 is replaced.
        let c = Ipv4Addr::new(10, 0, 0, 9);
        t.set_path(Path {
            hops: vec![
                Hop { ttl: 1, addr: a, owner: Owner::Gateway },
                Hop { ttl: 2, addr: c, owner: Owner::Edge },
            ],
            discovered: 10.0,
            dest: None,
        });
        assert!(
            t.stats.iter().any(|(addr, _)| *addr == a),
            "the hop that stayed keeps its history"
        );
        assert!(
            !t.stats.iter().any(|(addr, _)| *addr == b),
            "the hop that went away must not go on being reported"
        );
    }

    #[test]
    fn a_private_hop_past_the_router_is_not_claimed_for_the_user() {
        // A real path from this machine: the router, then 192.168.222.1,
        // 172.20.2.1, 10.30.64.1 and 10.8.105.1 at 4 to 9 ms, then the
        // provider's first public address. Four more routers in one house is
        // unlikely; a provider numbering its own access network privately is
        // common. The report called all four "your network", which in a
        // complaint to that provider argues its side.
        let label = crate::i18n::path_owner(Owner::Local);
        assert!(label.contains("provider") || label.contains("dostawcy"), "names both: {label}");
        assert_ne!(label, "your network");
        assert_ne!(label, "twoja sieć");
    }

    #[test]
    fn ownership_splits_a_double_nat_from_the_provider() {
        assert_eq!(Owner::Gateway.is_mine(), Some(true));
        assert_eq!(Owner::Local.is_mine(), None, "a second router, or the provider's network");
        assert_eq!(Owner::Edge.is_mine(), Some(false));
        assert!(is_cgnat(Ipv4Addr::new(100, 70, 1, 1)));
        assert!(!is_cgnat(Ipv4Addr::new(100, 200, 1, 1)), "100.200/8 is ordinary public space");
    }
}
