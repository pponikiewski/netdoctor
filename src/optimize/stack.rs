//! Tweaks below the adapter: the TCP stack, the resolver, and the two
//! Windows features that quietly use the link on their own.
//!
//! These change how the machine behaves on the network rather than how the
//! radio behaves, so none of them depends on the card, and none of them is
//! part of "apply everything safe" unless it is genuinely hard to regret.

use anyhow::Result;
use serde_json::{json, Value};

use super::{run, Category, Risk, State, Tweak};
use crate::probe::netstate::NetState;
use crate::winreg::{self, Root};

// ---------------------------------------------------------------------------
// a DWORD in the registry, which is most of this file
// ---------------------------------------------------------------------------

pub struct DwordTweak {
    pub id: &'static str,
    pub title: fn() -> &'static str,
    pub what: fn() -> &'static str,
    pub why: fn() -> &'static str,
    pub risk: Risk,
    pub category: Category,
    pub path: &'static str,
    pub name: &'static str,
    pub wanted: u32,
    /// Created when absent. Policy keys do not exist until a policy is set.
    pub create: bool,
    pub reboot: bool,
}

impl Tweak for DwordTweak {
    fn id(&self) -> &'static str {
        self.id
    }
    fn title(&self) -> &'static str {
        (self.title)()
    }
    fn what(&self) -> &'static str {
        (self.what)()
    }
    fn why(&self) -> &'static str {
        (self.why)()
    }
    fn risk(&self) -> Risk {
        self.risk
    }
    fn category(&self) -> Category {
        self.category
    }
    fn needs_reboot(&self) -> bool {
        self.reboot
    }

    fn read(&self, _net: &NetState) -> State {
        match winreg::read_dword(Root::LocalMachine, self.path, self.name) {
            Ok(Some(v)) => State::new(
                crate::i18n::tw_dw_state(self.name, v),
                Some(v == self.wanted),
                json!({ "value": v }),
            ),
            // The key or the value is missing, which both mean the same
            // thing here: Windows is running on its own default.
            Ok(None) | Err(_) => State::new(
                crate::i18n::tw_dw_unset(self.name),
                Some(false),
                json!({ "value": Value::Null }),
            ),
        }
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        if self.create {
            winreg::create_key(Root::LocalMachine, self.path)?;
        }
        winreg::write_dword(Root::LocalMachine, self.path, self.name, self.wanted)?;
        Ok(crate::i18n::tw_dw_applied(self.name, self.wanted))
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        match snapshot["value"].as_u64() {
            Some(v) => {
                winreg::write_dword(Root::LocalMachine, self.path, self.name, v as u32)?;
                Ok(crate::i18n::tw_dw_reverted(self.name, v as u32))
            }
            None => {
                winreg::delete_value(Root::LocalMachine, self.path, self.name)?;
                Ok(crate::i18n::tw_dw_removed(self.name))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TCP congestion control
// ---------------------------------------------------------------------------

/// Swaps the congestion control algorithm used for internet connections.
///
/// The default, CUBIC, reads every lost packet as congestion and backs off.
/// On a cable that is right. On Wi-Fi at the edge of range, where packets are
/// lost to interference rather than to a full queue, it means the throughput
/// collapses for a reason that was never there. BBR2 paces against measured
/// bandwidth and round-trip time instead, which is why it is worth trying on
/// exactly the link this app spends its time diagnosing.
///
/// It is not free. BBR2 wants a long flow to measure, and it does not read a
/// lost packet as a signal at all, so it waits out a retransmission timeout
/// where CUBIC would recover immediately. Anything built out of hundreds of
/// short TLS connections behind a short timeout — game launchers above all —
/// can stop loading entirely. Hence `Risk::High`.
pub struct CongestionControl;

/// The algorithm names netsh accepts and prints. They are identifiers, not
/// prose, so they survive a localised Windows — which is the only reason
/// reading this back out of netsh output is safe.
const PROVIDERS: [&str; 5] = ["bbr2", "cubic", "ctcp", "dctcp", "newreno"];

impl CongestionControl {
    fn show() -> Option<String> {
        run("netsh", &["int", "tcp", "show", "supplemental"]).ok()
    }

    /// Reads the provider for the `internet` template out of netsh's output.
    ///
    /// The layout and the labels differ between Windows builds and languages;
    /// the template names and the algorithm names do not. So: find where the
    /// internet template is described, and take the first algorithm named
    /// after it.
    fn parse(out: &str) -> Option<String> {
        let lower = out.to_lowercase();
        let start = lower.find("internet").unwrap_or(0);
        lower[start..]
            .split(|c: char| !c.is_ascii_alphanumeric())
            .find(|w| PROVIDERS.contains(w))
            .map(|w| w.to_string())
    }

    fn set(provider: &str) -> Result<()> {
        run(
            "netsh",
            &[
                "int",
                "tcp",
                "set",
                "supplemental",
                "template=internet",
                &format!("congestionprovider={provider}"),
            ],
        )
        .map(|_| ())
    }
}

impl Tweak for CongestionControl {
    fn id(&self) -> &'static str {
        "tcp_congestion"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_cong_title()
    }
    fn what(&self) -> &'static str {
        crate::i18n::tw_cong_what()
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_cong_why()
    }
    fn risk(&self) -> Risk {
        Risk::High
    }
    fn category(&self) -> Category {
        Category::Throughput
    }

    fn read(&self, _net: &NetState) -> State {
        let Some(out) = Self::show() else {
            return State::new(crate::i18n::tw_cong_unreadable(), None, Value::Null);
        };
        match Self::parse(&out) {
            Some(p) => State::new(p.to_uppercase(), Some(p == "bbr2"), json!({ "provider": p })),
            None => State::new(crate::i18n::tw_cong_unreadable(), None, Value::Null),
        }
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        // BBR2 is only present on Windows 11 and is rejected outright
        // elsewhere, so a refusal is a version answer, not a failure.
        match Self::set("bbr2") {
            Ok(()) => Ok(crate::i18n::tw_cong_applied("BBR2")),
            Err(_) => {
                Self::set("ctcp")?;
                Ok(crate::i18n::tw_cong_applied_fallback("CTCP"))
            }
        }
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let provider = snapshot["provider"].as_str().unwrap_or("cubic");
        Self::set(provider)?;
        Ok(crate::i18n::tw_cong_reverted(&provider.to_uppercase()))
    }
}

// ---------------------------------------------------------------------------

const DNSCACHE: &str = r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters";
const TCPIP6: &str = r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters";
const DELIVERY_OPT: &str = r"SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization";
const WCM_CONFIG: &str = r"SOFTWARE\Microsoft\WcmSvc\wifinetworkmanager\config";

/// Prefer IPv4 when both are on offer. Bit 5 of `DisabledComponents` reorders
/// the prefix policy table; it leaves IPv6 itself working.
const IPV6_PREFER_IPV4: u32 = 0x20;

pub fn all() -> Vec<Box<dyn Tweak>> {
    vec![
        Box::new(CongestionControl),
        Box::new(DwordTweak {
            id: "dns_negative_cache",
            title: crate::i18n::tw_negdns_title,
            what: crate::i18n::tw_negdns_what,
            why: crate::i18n::tw_negdns_why,
            risk: Risk::Low,
            category: Category::Naming,
            path: DNSCACHE,
            name: "MaxNegativeCacheTtl",
            wanted: 0,
            create: false,
            reboot: false,
        }),
        Box::new(DwordTweak {
            id: "delivery_optimisation",
            title: crate::i18n::tw_do_title,
            what: crate::i18n::tw_do_what,
            why: crate::i18n::tw_do_why,
            risk: Risk::Low,
            category: Category::Neighbours,
            path: DELIVERY_OPT,
            name: "DODownloadMode",
            wanted: 0,
            create: true,
            reboot: false,
        }),
        Box::new(DwordTweak {
            id: "hotspot_autoconnect",
            title: crate::i18n::tw_hotspot_title,
            what: crate::i18n::tw_hotspot_what,
            why: crate::i18n::tw_hotspot_why,
            risk: Risk::Low,
            category: Category::Neighbours,
            path: WCM_CONFIG,
            name: "AutoConnectAllowedOEM",
            wanted: 0,
            create: true,
            reboot: false,
        }),
        Box::new(DwordTweak {
            id: "prefer_ipv4",
            title: crate::i18n::tw_ipv4_title,
            what: crate::i18n::tw_ipv4_what,
            why: crate::i18n::tw_ipv4_why,
            risk: Risk::High,
            category: Category::Naming,
            path: TCPIP6,
            name: "DisabledComponents",
            wanted: IPV6_PREFER_IPV4,
            create: false,
            reboot: true,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_congestion_provider_is_read_out_of_the_internet_template() {
        let out = "\
Template                            : automatic
CongestionProvider                  : cubic

Template                            : internet
CongestionProvider                  : bbr2
";
        assert_eq!(CongestionControl::parse(out).as_deref(), Some("bbr2"));
    }

    #[test]
    fn a_localised_netsh_still_parses() {
        // Only the labels are translated; the identifiers are not.
        let out = "Szablon: internet\nDostawca kontroli przeciążenia: ctcp\n";
        assert_eq!(CongestionControl::parse(out).as_deref(), Some("ctcp"));
    }

    #[test]
    fn output_with_no_algorithm_in_it_reads_as_unknown() {
        assert_eq!(CongestionControl::parse("nothing useful here"), None);
    }

    #[test]
    fn every_stack_tweak_is_described_and_uniquely_named() {
        let mut ids = std::collections::HashSet::new();
        for t in all() {
            assert!(ids.insert(t.id().to_string()), "duplicate id {}", t.id());
            assert!(!t.title().is_empty());
            assert!(!t.what().is_empty());
            assert!(!t.why().is_empty());
        }
    }
}
