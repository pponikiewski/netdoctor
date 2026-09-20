//! Tweaks that reach into the radio itself.
//!
//! Everything here is one of the entries on the Advanced tab of the adapter's
//! properties in Device Manager. Those live in the driver's class key as plain
//! strings, and the options each one accepts are listed underneath in
//! `Ndi\Params\<name>\Enum`, where the value name is what gets written and the
//! data is the wording Windows shows.
//!
//! Two things follow from that, and they shape this whole module. The name of
//! a property is the vendor's choice — Intel, Realtek, Qualcomm and Broadcom
//! all spell "roaming aggressiveness" differently — so each tweak carries a
//! list of names and takes the first the driver actually has. And the numbers
//! behind the options are the vendor's choice too, so nothing here hardcodes
//! one: the wanted option is found by matching the driver's own wording, which
//! means "highest transmit power" stays "highest" on a card that numbers it
//! 1..5 and on one that numbers it 0..100.
//!
//! None of this takes effect until the adapter is restarted, exactly as in
//! Device Manager, so every tweak here reports that it needs a reboot.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::{adapter_class_key, Category, Risk, State, Tweak};
use crate::probe::netstate::{Medium, NetState};
use crate::winreg::{self, Root};

/// Which option of an advanced property we are after.
pub enum Target {
    /// Match the driver's own wording for the option, in order of preference.
    /// Case-insensitive substring match against the text Device Manager
    /// shows, so a card that calls the top setting "Highest" and one that
    /// calls it "5. Highest" both resolve.
    Named(&'static [&'static str]),
    /// Write a literal value. Used for the properties that are a plain
    /// number or a bare 0/1 with no option list.
    Exact(&'static str),
}

pub struct AdvTweak {
    pub id: &'static str,
    pub title: fn() -> &'static str,
    pub what: fn() -> &'static str,
    pub why: fn() -> &'static str,
    pub risk: Risk,
    pub category: Category,
    /// Names this property goes by across vendors; the first one present on
    /// this driver wins.
    pub params: &'static [&'static str],
    pub target: Target,
    /// The connection this property is about. A Wi-Fi tweak on a desktop with
    /// a cable reports "not applicable" rather than silently doing nothing.
    pub medium: Medium,
}

/// A property as this particular driver exposes it.
struct Param {
    /// The class key of the adapter, e.g. `...\Class\{4d36e972-…}\0001`.
    class: String,
    /// The registry name the driver uses.
    name: String,
}

impl Param {
    fn params_key(&self) -> String {
        format!("{}\\Ndi\\Params\\{}", self.class, self.name)
    }

    /// The option list, as (value written, wording shown). Empty when the
    /// property is a free number rather than a dropdown.
    fn options(&self) -> Vec<(String, String)> {
        let enum_key = format!("{}\\Enum", self.params_key());
        winreg::value_names(Root::LocalMachine, &enum_key)
            .into_iter()
            .map(|v| {
                let label = winreg::read_string(Root::LocalMachine, &enum_key, &v)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                (v, label)
            })
            .collect()
    }

    fn current(&self) -> Option<String> {
        winreg::read_string(Root::LocalMachine, &self.class, &self.name).ok().flatten().or_else(
            || {
                // Not overridden yet, so the driver is using its own default.
                winreg::read_string(Root::LocalMachine, &self.params_key(), "default")
                    .ok()
                    .flatten()
            },
        )
    }

    /// The wording for a value, falling back to the raw value when the
    /// property has no option list.
    fn label_of(&self, value: &str) -> String {
        self.options()
            .into_iter()
            .find(|(v, _)| v == value)
            .map(|(v, l)| if l.is_empty() { v } else { l })
            .unwrap_or_else(|| value.to_string())
    }
}

/// Finds the first of `names` that this adapter's driver actually declares.
fn find_param(net: &NetState, names: &[&str]) -> Option<Param> {
    let class = adapter_class_key(net)?;
    let declared = winreg::subkeys(Root::LocalMachine, &format!("{class}\\Ndi\\Params"));
    for wanted in names {
        if let Some(found) = declared.iter().find(|d| d.eq_ignore_ascii_case(wanted)) {
            return Some(Param { class, name: found.clone() });
        }
    }
    None
}

impl AdvTweak {
    /// The value this tweak wants written, and how it will read on screen.
    fn wanted(&self, param: &Param) -> Option<(String, String)> {
        match self.target {
            Target::Exact(v) => Some((v.to_string(), param.label_of(v))),
            Target::Named(keywords) => {
                let options = param.options();
                for want in keywords {
                    let want = want.to_lowercase();
                    if let Some((v, l)) =
                        options.iter().find(|(_, label)| label.to_lowercase().contains(&want))
                    {
                        return Some((v.clone(), l.clone()));
                    }
                }
                None
            }
        }
    }

    fn applicable(&self, net: &NetState) -> bool {
        net.medium == self.medium
    }
}

impl Tweak for AdvTweak {
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
        true
    }

    fn read(&self, net: &NetState) -> State {
        if !self.applicable(net) {
            return State::new(
                crate::i18n::tw_adv_wrong_medium(self.medium.label()),
                None,
                Value::Null,
            );
        }
        let Some(param) = find_param(net, self.params) else {
            return State::new(crate::i18n::tw_adv_unsupported(), None, Value::Null);
        };
        let current = param.current();
        let Some((wanted, wanted_label)) = self.wanted(&param) else {
            return State::new(crate::i18n::tw_adv_no_option(), None, Value::Null);
        };
        let snapshot = json!({
            "key": param.class,
            "name": param.name,
            "value": current,
        });
        match current {
            Some(v) if v == wanted => State::new(param.label_of(&v), Some(true), snapshot),
            Some(v) => State::new(
                crate::i18n::tw_adv_now_vs_wanted(&param.label_of(&v), &wanted_label),
                Some(false),
                snapshot,
            ),
            None => State::new(crate::i18n::tw_adv_unset(&wanted_label), Some(false), snapshot),
        }
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        if !self.applicable(net) {
            return Err(anyhow!(crate::i18n::tw_adv_wrong_medium(self.medium.label())));
        }
        let param = find_param(net, self.params)
            .ok_or_else(|| anyhow!(crate::i18n::tw_adv_unsupported()))?;
        let (value, label) =
            self.wanted(&param).ok_or_else(|| anyhow!(crate::i18n::tw_adv_no_option()))?;
        winreg::write_string(Root::LocalMachine, &param.class, &param.name, &value)?;
        Ok(crate::i18n::tw_adv_applied(&param.name, &label))
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let key =
            snapshot["key"].as_str().ok_or_else(|| anyhow!(crate::i18n::tw_snapshot_no_key()))?;
        let name =
            snapshot["name"].as_str().ok_or_else(|| anyhow!(crate::i18n::tw_snapshot_no_key()))?;
        match snapshot["value"].as_str() {
            Some(v) => winreg::write_string(Root::LocalMachine, key, name, v)?,
            // It was never overridden, so removing ours hands it back to the
            // driver's default rather than pinning that default in place.
            None => winreg::delete_value(Root::LocalMachine, key, name)?,
        }
        Ok(crate::i18n::tw_adv_reverted(name))
    }
}

pub fn all() -> Vec<Box<dyn Tweak>> {
    vec![
        Box::new(AdvTweak {
            id: "radio_tx_power",
            title: crate::i18n::tw_tx_title,
            what: crate::i18n::tw_tx_what,
            why: crate::i18n::tw_tx_why,
            risk: Risk::Medium,
            category: Category::Reach,
            params: &[
                "TxPower",
                "TransmitPower",
                "TxPowerLevel",
                "PowerOutput",
                "TransmitPowerLevel",
                "ulTxPower",
            ],
            target: Target::Named(&["highest", "100", "maximum", "najwy"]),
            medium: Medium::Wifi,
        }),
        Box::new(AdvTweak {
            id: "radio_roaming",
            title: crate::i18n::tw_roam_title,
            what: crate::i18n::tw_roam_what,
            why: crate::i18n::tw_roam_why,
            risk: Risk::Medium,
            category: Category::Reach,
            params: &[
                "RoamAggressiveness",
                "RoamingAggressiveness",
                "RoamingTendency",
                "RoamTendency",
                "RoamingPolicy",
                "ScanWhenAssociated",
            ],
            target: Target::Named(&["highest", "aggressive", "5.", "najwy"]),
            medium: Medium::Wifi,
        }),
        Box::new(AdvTweak {
            id: "radio_power_save",
            title: crate::i18n::tw_psm_title,
            what: crate::i18n::tw_psm_what,
            why: crate::i18n::tw_psm_why,
            risk: Risk::Low,
            category: Category::Power,
            params: &[
                "PowerSaveMode",
                "PowerSavingMode",
                "WirelessMode_PowerSave",
                "uAPSDSupport",
                "ulPowerSaveMode",
            ],
            target: Target::Named(&["no power sav", "maximum perf", "disabled", "off", "cam"]),
            medium: Medium::Wifi,
        }),
        Box::new(AdvTweak {
            id: "radio_mimo_ps",
            title: crate::i18n::tw_mimo_title,
            what: crate::i18n::tw_mimo_what,
            why: crate::i18n::tw_mimo_why,
            risk: Risk::Low,
            category: Category::Power,
            params: &["MIMOPowerSaveMode", "MimoPowerSaveMode", "SMPS", "HT_SMPS"],
            target: Target::Named(&["no smps", "disabled", "off", "none"]),
            medium: Medium::Wifi,
        }),
        Box::new(AdvTweak {
            id: "radio_width_24",
            title: crate::i18n::tw_w24_title,
            what: crate::i18n::tw_w24_what,
            why: crate::i18n::tw_w24_why,
            risk: Risk::Medium,
            category: Category::Reach,
            params: &[
                "ChannelWidth24GHz",
                "ChannelWidth_24GHz",
                "Bandwidth24G",
                "BandwidthCapability24GHz",
                "HT_BW_24G",
            ],
            target: Target::Named(&["20 mhz", "20mhz", "1 (20", "20"]),
            medium: Medium::Wifi,
        }),
        Box::new(AdvTweak {
            id: "radio_band_pref",
            title: crate::i18n::tw_band_title,
            what: crate::i18n::tw_band_what,
            why: crate::i18n::tw_band_why,
            risk: Risk::Medium,
            category: Category::Reach,
            params: &[
                "RoamingPreferredBandType",
                "PreferredBandType",
                "PreferredBand",
                "BandPreference",
                "VHTBandPreference",
            ],
            target: Target::Named(&["prefer 5", "5.2", "5 ghz", "5ghz"]),
            medium: Medium::Wifi,
        }),
        Box::new(AdvTweak {
            id: "nic_interrupt_moderation",
            title: crate::i18n::tw_intmod_title,
            what: crate::i18n::tw_intmod_what,
            why: crate::i18n::tw_intmod_why,
            risk: Risk::Medium,
            category: Category::Throughput,
            params: &["*InterruptModeration", "InterruptModeration", "ITR"],
            target: Target::Exact("0"),
            medium: Medium::Ethernet,
        }),
        Box::new(AdvTweak {
            id: "nic_green_ethernet",
            title: crate::i18n::tw_green_title,
            what: crate::i18n::tw_green_what,
            why: crate::i18n::tw_green_why,
            risk: Risk::Low,
            category: Category::Power,
            params: &[
                "EnableGreenEthernet",
                "*EEE",
                "AdvancedEEE",
                "EnableSavePowerNow",
                "PowerSavingMode",
            ],
            target: Target::Exact("0"),
            medium: Medium::Ethernet,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_radio_tweak_is_described_and_uniquely_named() {
        let mut ids = std::collections::HashSet::new();
        for t in all() {
            assert!(ids.insert(t.id().to_string()), "duplicate id {}", t.id());
            assert!(!t.title().is_empty());
            assert!(!t.what().is_empty());
            assert!(!t.why().is_empty());
        }
    }

    #[test]
    fn option_matching_prefers_the_earlier_keyword() {
        // Reproduces what a driver's Enum key looks like, without touching
        // the registry: the wording decides, not the number behind it.
        let options = [
            ("0".to_string(), "Lowest".to_string()),
            ("3".to_string(), "Medium".to_string()),
            ("5".to_string(), "5. Highest".to_string()),
        ];
        let pick = |keywords: &[&str]| -> Option<String> {
            for want in keywords {
                let want = want.to_lowercase();
                if let Some((v, _)) = options.iter().find(|(_, l)| l.to_lowercase().contains(&want))
                {
                    return Some(v.clone());
                }
            }
            None
        };
        assert_eq!(pick(&["highest", "medium"]).as_deref(), Some("5"));
        assert_eq!(pick(&["nothing here", "medium"]).as_deref(), Some("3"));
        assert_eq!(pick(&["nothing at all"]), None);
    }
}
