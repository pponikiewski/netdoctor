//! Tweaks: each one can be inspected, applied and reverted.
//!
//! Nothing here runs by itself. Every tweak records the previous value to
//! `tweak_snapshots.json` before touching anything, so "Revert" restores the
//! exact state the machine was in — including after a reboot, which is when it
//! matters most, because several of these only take effect after one.

pub mod radio;
pub mod stack;

use std::collections::HashMap;
use std::process::Command;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::probe::netstate::NetState;
use crate::settings;
use crate::winreg::{self, Root};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What a tweak is *about*, so the list can be read a section at a time
/// instead of as twenty-one unrelated switches. The order here is the order
/// they appear in, and it runs from the causes of outright dropouts down to
/// the ones that only shave milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// Windows or the driver switching hardware off underneath you.
    Power,
    /// How far the radio reaches and which access point it holds on to.
    Reach,
    /// Turning names into addresses, and which address family wins.
    Naming,
    /// How fast bytes move once the link is up.
    Throughput,
    /// Other things on this machine helping themselves to the uplink.
    Neighbours,
    /// Blunt instruments for when the link is already broken.
    LastResort,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Power,
        Category::Reach,
        Category::Naming,
        Category::Throughput,
        Category::Neighbours,
        Category::LastResort,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Category::Power => crate::i18n::cat_power(),
            Category::Reach => crate::i18n::cat_reach(),
            Category::Naming => crate::i18n::cat_naming(),
            Category::Throughput => crate::i18n::cat_throughput(),
            Category::Neighbours => crate::i18n::cat_neighbours(),
            Category::LastResort => crate::i18n::cat_last_resort(),
        }
    }

    /// One line saying what the section is for, shown under its heading.
    pub fn blurb(&self) -> &'static str {
        match self {
            Category::Power => crate::i18n::cat_power_blurb(),
            Category::Reach => crate::i18n::cat_reach_blurb(),
            Category::Naming => crate::i18n::cat_naming_blurb(),
            Category::Throughput => crate::i18n::cat_throughput_blurb(),
            Category::Neighbours => crate::i18n::cat_neighbours_blurb(),
            Category::LastResort => crate::i18n::cat_last_resort_blurb(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Low,
    Medium,
    /// Changes how the machine reaches the network rather than how fast it
    /// does so. Reversible, but wrong for some setups, so it is never part of
    /// "apply everything safe".
    High,
}

impl Risk {
    pub fn label(&self) -> &'static str {
        match self {
            Risk::Low => crate::i18n::risk_low(),
            Risk::Medium => crate::i18n::risk_medium(),
            Risk::High => crate::i18n::risk_high(),
        }
    }
}

/// What a tweak currently looks like on this machine.
pub struct State {
    /// Human-readable current value.
    pub text: String,
    /// `Some(true)` already optimal, `Some(false)` worth changing,
    /// `None` not applicable or unreadable.
    pub optimal: Option<bool>,
    /// Opaque value stored so revert can restore it.
    pub snapshot: Value,
}

impl State {
    fn new(text: impl Into<String>, optimal: Option<bool>, snapshot: Value) -> Self {
        State { text: text.into(), optimal, snapshot }
    }
}

pub trait Tweak: Send + Sync {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn what(&self) -> &'static str;
    fn why(&self) -> &'static str;
    fn risk(&self) -> Risk;
    /// Which section of the list this belongs under. Deliberately has no
    /// default: a new tweak has to say where it goes, or it does not compile.
    fn category(&self) -> Category;

    fn reversible(&self) -> bool {
        true
    }
    fn needs_admin(&self) -> bool {
        true
    }
    /// True when the change only takes effect after a restart.
    fn needs_reboot(&self) -> bool {
        false
    }

    fn read(&self, net: &NetState) -> State;
    fn apply(&self, net: &NetState) -> Result<String>;
    fn revert(&self, net: &NetState, snapshot: &Value) -> Result<String>;
}

// ---------------------------------------------------------------------------
// snapshot persistence
// ---------------------------------------------------------------------------

pub fn load_snapshots() -> HashMap<String, Value> {
    std::fs::read_to_string(settings::snapshot_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_snapshots(map: &HashMap<String, Value>) -> Result<()> {
    std::fs::create_dir_all(settings::data_dir())?;
    std::fs::write(settings::snapshot_path(), serde_json::to_string_pretty(map)?)?;
    Ok(())
}

pub fn has_snapshot(id: &str) -> bool {
    load_snapshots().contains_key(id)
}

/// Snapshot, then apply. Refuses without elevation so a half-applied change
/// cannot happen.
pub fn apply(tweak: &dyn Tweak, net: &NetState) -> Result<String> {
    if tweak.needs_admin() && !is_elevated() {
        return Err(anyhow!(crate::i18n::tw_needs_admin()));
    }
    let state = tweak.read(net);
    let mut snaps = load_snapshots();
    snaps.insert(tweak.id().to_string(), state.snapshot.clone());
    save_snapshots(&snaps)?;

    tweak.apply(net)
}

pub fn revert(tweak: &dyn Tweak, net: &NetState) -> Result<String> {
    if tweak.needs_admin() && !is_elevated() {
        return Err(anyhow!(crate::i18n::tw_revert_needs_admin()));
    }
    let snaps = load_snapshots();
    let Some(snapshot) = snaps.get(tweak.id()) else {
        return Err(anyhow!(crate::i18n::tw_no_snapshot()));
    };
    let msg = tweak.revert(net, snapshot)?;
    let mut snaps = load_snapshots();
    snaps.remove(tweak.id());
    save_snapshots(&snaps)?;
    Ok(msg)
}

pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size,
            &mut size,
        )
        .is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Run a console tool without flashing a window.
pub(crate) fn run(program: &str, args: &[&str]) -> Result<String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd.output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        Err(anyhow!(text.trim().to_string()))
    }
}

// ---------------------------------------------------------------------------
// adapter power management
// ---------------------------------------------------------------------------

const NET_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}";

/// Finds the driver subkey belonging to an adapter GUID.
pub(crate) fn adapter_class_key(net: &NetState) -> Option<String> {
    if net.adapter_guid.is_empty() {
        return None;
    }
    for sub in winreg::subkeys(Root::LocalMachine, NET_CLASS) {
        if !sub.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let path = format!("{NET_CLASS}\\{sub}");
        if let Ok(Some(id)) = winreg::read_string(Root::LocalMachine, &path, "NetCfgInstanceId") {
            if id.eq_ignore_ascii_case(&net.adapter_guid) {
                return Some(path);
            }
        }
    }
    None
}

pub struct AdapterPowerSaving;

// Bit 3 (value 24 = 0x18) is what the Device Manager checkbox writes to stop
// Windows powering the adapter down.
const PNP_DISABLE_POWER_DOWN: u32 = 24;

impl Tweak for AdapterPowerSaving {
    fn id(&self) -> &'static str {
        "adapter_power"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_power_title()
    }
    fn what(&self) -> &'static str {
        crate::i18n::tw_power_what()
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_power_why()
    }
    fn risk(&self) -> Risk {
        Risk::Low
    }
    fn category(&self) -> Category {
        Category::Power
    }
    fn needs_reboot(&self) -> bool {
        true
    }

    fn read(&self, net: &NetState) -> State {
        let Some(key) = adapter_class_key(net) else {
            return State::new(crate::i18n::tw_power_no_adapter(), None, Value::Null);
        };
        match winreg::read_dword(Root::LocalMachine, &key, "PnPCapabilities") {
            Ok(Some(v)) if v & PNP_DISABLE_POWER_DOWN == PNP_DISABLE_POWER_DOWN => State::new(
                crate::i18n::tw_power_off_detail(v),
                Some(true),
                json!({ "key": key, "value": v }),
            ),
            Ok(current) => State::new(
                match current {
                    Some(v) => crate::i18n::tw_power_on_detail(v),
                    None => crate::i18n::tw_power_on_unset().to_string(),
                },
                Some(false),
                json!({ "key": key, "value": current }),
            ),
            Err(e) => State::new(crate::i18n::tw_cannot_read(&e.to_string()), None, Value::Null),
        }
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let key = adapter_class_key(net).ok_or_else(|| anyhow!(crate::i18n::tw_no_adapter_key()))?;
        winreg::write_dword(Root::LocalMachine, &key, "PnPCapabilities", PNP_DISABLE_POWER_DOWN)?;
        Ok(crate::i18n::tw_power_applied().into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let key = snapshot["key"].as_str().ok_or_else(|| anyhow!(crate::i18n::tw_snapshot_no_key()))?;
        match snapshot["value"].as_u64() {
            Some(v) => {
                winreg::write_dword(Root::LocalMachine, key, "PnPCapabilities", v as u32)?
            }
            None => winreg::delete_value(Root::LocalMachine, key, "PnPCapabilities")?,
        }
        Ok(crate::i18n::tw_power_reverted().into())
    }
}

// ---------------------------------------------------------------------------
// power plan: Wi-Fi radio
// ---------------------------------------------------------------------------

pub struct WlanPowerPlan;

const SUB_WIRELESS: &str = "19cbb8fa-5279-450e-9fac-8a3d5fedd0c1";
const SETTING_POWER_SAVING: &str = "12bbebe6-58d6-4636-95bb-3217ef867c1a";

impl Tweak for WlanPowerPlan {
    fn id(&self) -> &'static str {
        "wlan_power_plan"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_wlan_title()
    }
    fn what(&self) -> &'static str {
        crate::i18n::tw_wlan_what()
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_wlan_why()
    }
    fn risk(&self) -> Risk {
        Risk::Low
    }
    fn category(&self) -> Category {
        Category::Power
    }

    fn read(&self, _net: &NetState) -> State {
        let out = match run("powercfg", &["/query", "SCHEME_CURRENT", SUB_WIRELESS, SETTING_POWER_SAVING])
        {
            Ok(t) => t,
            Err(e) => {
                return State::new(crate::i18n::tw_cannot_read(&e.to_string()), None, Value::Null)
            }
        };
        let ac = extract_index(&out, AC_INDEX_LABELS, 0);
        let dc = extract_index(&out, DC_INDEX_LABELS, 1);
        let (Some(ac), Some(dc)) = (ac, dc) else {
            return State::new(crate::i18n::tw_wlan_absent(), None, Value::Null);
        };
        let name = |v: u32| match v {
            0 => crate::i18n::tw_wlan_max_perf(),
            1 => crate::i18n::tw_wlan_low_save(),
            2 => crate::i18n::tw_wlan_med_save(),
            _ => crate::i18n::tw_wlan_max_save(),
        };
        State::new(
            crate::i18n::tw_wlan_state(name(ac), name(dc)),
            Some(ac == 0 && dc == 0),
            json!({ "ac": ac, "dc": dc }),
        )
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        set_power_indices(0, 0)?;
        Ok(crate::i18n::tw_wlan_applied().into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let ac = snapshot["ac"].as_u64().unwrap_or(0) as u32;
        let dc = snapshot["dc"].as_u64().unwrap_or(3) as u32;
        set_power_indices(ac, dc)?;
        Ok(crate::i18n::tw_wlan_reverted().into())
    }
}

fn set_power_indices(ac: u32, dc: u32) -> Result<()> {
    run("powercfg", &["/setacvalueindex", "SCHEME_CURRENT", SUB_WIRELESS, SETTING_POWER_SAVING, &ac.to_string()])?;
    run("powercfg", &["/setdcvalueindex", "SCHEME_CURRENT", SUB_WIRELESS, SETTING_POWER_SAVING, &dc.to_string()])?;
    run("powercfg", &["/setactive", "SCHEME_CURRENT"])?;
    Ok(())
}

// powercfg and netsh localise their output, so matching a single English
// label silently returns None on a Polish Windows and the tweak reports
// "cannot read" forever. Each helper below knows the labels for the languages
// we ship and falls back to a structural rule that holds whatever the locale.

const AC_INDEX_LABELS: &[&str] = &["Current AC Power Setting Index", "prądem zmiennym"];
const DC_INDEX_LABELS: &[&str] = &["Current DC Power Setting Index", "prądem stałym"];
const AUTOTUNE_LABELS: &[&str] = &["Auto-Tuning Level", "automatycznego dostrajania"];

fn find_labelled_line<'a>(text: &'a str, labels: &[&str]) -> Option<&'a str> {
    text.lines().find(|l| labels.iter().any(|label| l.contains(label)))
}

fn parse_hex(line: &str) -> Option<u32> {
    let hex = line.split("0x").nth(1)?.trim();
    u32::from_str_radix(hex, 16).ok()
}

/// Map a possibly translated auto-tuning level onto the keyword `netsh set`
/// accepts. Unrecognised input falls back to `normal`, which is the value
/// Windows itself defaults to.
fn canonical_autotune_level(value: &str) -> &'static str {
    let v = value.to_lowercase();
    if v.starts_with("normal") {
        "normal"
    } else if v.starts_with("disabled") || v.starts_with("wyłącz") {
        "disabled"
    } else if v.starts_with("highly") || v.starts_with("wysoce") {
        "highlyrestricted"
    } else if v.starts_with("restricted") || v.starts_with("ogranicz") {
        "restricted"
    } else if v.starts_with("experimental") || v.starts_with("eksperyment") {
        "experimental"
    } else {
        "normal"
    }
}

/// A powercfg setting index. `ordinal` is the fallback: powercfg prints the AC
/// index before the DC one in every language, so position identifies them even
/// when the label does not match.
fn extract_index(text: &str, labels: &[&str], ordinal: usize) -> Option<u32> {
    if let Some(v) = find_labelled_line(text, labels).and_then(parse_hex) {
        return Some(v);
    }
    text.lines().filter_map(parse_hex).nth(ordinal)
}

// ---------------------------------------------------------------------------
// DNS
// ---------------------------------------------------------------------------

pub struct FastDns;

impl Tweak for FastDns {
    fn id(&self) -> &'static str {
        "fast_dns"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_dns_title()
    }
    fn what(&self) -> &'static str {
        crate::i18n::tw_dns_what()
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_dns_why()
    }
    fn risk(&self) -> Risk {
        Risk::Low
    }
    fn category(&self) -> Category {
        Category::Naming
    }

    fn read(&self, net: &NetState) -> State {
        let current: Vec<String> = net.dns_servers.iter().map(|d| d.to_string()).collect();
        let good = ["1.1.1.1", "1.0.0.1", "8.8.8.8", "8.8.4.4", "9.9.9.9"];
        let mut text = if current.is_empty() {
            crate::i18n::tw_dns_none().to_string()
        } else {
            current.join(", ")
        };
        if net.dns_is_router_only() {
            text.push_str(crate::i18n::tw_dns_router_only_note());
        }
        let optimal = current.iter().any(|d| good.contains(&d.as_str()));
        State::new(text, Some(optimal), json!({ "servers": current }))
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let name = &net.adapter_name;
        run("netsh", &["interface", "ipv4", "set", "dnsservers", &format!("name={name}"), "source=static", "address=1.1.1.1", "register=primary", "validate=no"])?;
        run("netsh", &["interface", "ipv4", "add", "dnsservers", &format!("name={name}"), "address=8.8.8.8", "index=2", "validate=no"])?;
        let _ = run("ipconfig", &["/flushdns"]);
        Ok(crate::i18n::tw_dns_applied().into())
    }

    fn revert(&self, net: &NetState, snapshot: &Value) -> Result<String> {
        let name = &net.adapter_name;
        let servers: Vec<String> = snapshot["servers"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        if servers.is_empty() {
            run("netsh", &["interface", "ipv4", "set", "dnsservers", &format!("name={name}"), "source=dhcp"])?;
            let _ = run("ipconfig", &["/flushdns"]);
            return Ok(crate::i18n::tw_dns_reverted_dhcp().into());
        }
        run("netsh", &["interface", "ipv4", "set", "dnsservers", &format!("name={name}"), "source=static", &format!("address={}", servers[0]), "register=primary", "validate=no"])?;
        for (i, s) in servers.iter().skip(1).enumerate() {
            let _ = run("netsh", &["interface", "ipv4", "add", "dnsservers", &format!("name={name}"), &format!("address={s}"), &format!("index={}", i + 2), "validate=no"]);
        }
        let _ = run("ipconfig", &["/flushdns"]);
        Ok(crate::i18n::tw_dns_reverted().into())
    }
}

// ---------------------------------------------------------------------------
// TCP autotuning
// ---------------------------------------------------------------------------

pub struct TcpAutotuning;

impl Tweak for TcpAutotuning {
    fn id(&self) -> &'static str {
        "tcp_autotuning"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_autotune_title()
    }
    fn what(&self) -> &'static str {
        "netsh int tcp set global autotuninglevel=normal"
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_autotune_why()
    }
    fn risk(&self) -> Risk {
        Risk::Low
    }
    fn category(&self) -> Category {
        Category::Throughput
    }

    fn read(&self, _net: &NetState) -> State {
        let out = match run("netsh", &["int", "tcp", "show", "global"]) {
            Ok(t) => t,
            Err(e) => {
                return State::new(crate::i18n::tw_cannot_read(&e.to_string()), None, Value::Null)
            }
        };
        let Some(line) = find_labelled_line(&out, AUTOTUNE_LABELS) else {
            return State::new(crate::i18n::tw_state_unreadable(), None, Value::Null);
        };
        let value = line.split(':').nth(1).unwrap_or("").trim().to_lowercase();
        // netsh translates the value as well as the label, so the snapshot
        // stores the canonical English keyword: that is the only spelling
        // `netsh set` accepts when reverting.
        let level = canonical_autotune_level(&value);
        State::new(value, Some(level == "normal"), json!({ "level": level }))
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        run("netsh", &["int", "tcp", "set", "global", "autotuninglevel=normal"])?;
        Ok(crate::i18n::tw_autotune_applied().into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let level = snapshot["level"].as_str().unwrap_or("normal");
        run("netsh", &["int", "tcp", "set", "global", &format!("autotuninglevel={level}")])?;
        Ok(crate::i18n::tw_autotune_reverted(level))
    }
}

// ---------------------------------------------------------------------------
// Nagle
// ---------------------------------------------------------------------------

pub struct NagleOff;

const TCPIP_INTERFACES: &str = r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces";

impl NagleOff {
    fn key(net: &NetState) -> Option<String> {
        if net.adapter_guid.is_empty() {
            return None;
        }
        Some(format!("{TCPIP_INTERFACES}\\{}", net.adapter_guid))
    }
}

impl Tweak for NagleOff {
    fn id(&self) -> &'static str {
        "nagle_off"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_nagle_title()
    }
    fn what(&self) -> &'static str {
        crate::i18n::tw_nagle_what()
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_nagle_why()
    }
    fn risk(&self) -> Risk {
        Risk::Medium
    }
    fn category(&self) -> Category {
        Category::Throughput
    }
    fn needs_reboot(&self) -> bool {
        true
    }

    fn read(&self, net: &NetState) -> State {
        let Some(key) = Self::key(net) else {
            return State::new(crate::i18n::tw_nagle_no_guid(), None, Value::Null);
        };
        let ack = winreg::read_dword(Root::LocalMachine, &key, "TcpAckFrequency").ok().flatten();
        let nodelay = winreg::read_dword(Root::LocalMachine, &key, "TCPNoDelay").ok().flatten();
        let text = format!(
            "TcpAckFrequency={}, TCPNoDelay={}",
            ack.map(|v| v.to_string()).unwrap_or_else(|| crate::i18n::tw_not_set().into()),
            nodelay.map(|v| v.to_string()).unwrap_or_else(|| crate::i18n::tw_not_set().into())
        );
        State::new(
            text,
            Some(ack == Some(1) && nodelay == Some(1)),
            json!({ "key": key, "ack": ack, "nodelay": nodelay }),
        )
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let key = Self::key(net).ok_or_else(|| anyhow!(crate::i18n::tw_nagle_no_guid()))?;
        winreg::write_dword(Root::LocalMachine, &key, "TcpAckFrequency", 1)?;
        winreg::write_dword(Root::LocalMachine, &key, "TCPNoDelay", 1)?;
        Ok(crate::i18n::tw_nagle_applied().into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let key = snapshot["key"].as_str().ok_or_else(|| anyhow!(crate::i18n::tw_snapshot_no_key()))?;
        for (name, field) in [("TcpAckFrequency", "ack"), ("TCPNoDelay", "nodelay")] {
            match snapshot[field].as_u64() {
                Some(v) => winreg::write_dword(Root::LocalMachine, key, name, v as u32)?,
                None => winreg::delete_value(Root::LocalMachine, key, name)?,
            }
        }
        Ok(crate::i18n::tw_nagle_reverted().into())
    }
}

// ---------------------------------------------------------------------------
// multimedia network throttling
// ---------------------------------------------------------------------------

pub struct NetworkThrottling;

const MM_PROFILE: &str =
    r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile";

impl Tweak for NetworkThrottling {
    fn id(&self) -> &'static str {
        "net_throttling"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_throttle_title()
    }
    fn what(&self) -> &'static str {
        "NetworkThrottlingIndex = 0xffffffff, SystemResponsiveness = 10."
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_throttle_why()
    }
    fn risk(&self) -> Risk {
        Risk::Medium
    }
    fn category(&self) -> Category {
        Category::Throughput
    }
    fn needs_reboot(&self) -> bool {
        true
    }

    fn read(&self, _net: &NetState) -> State {
        let nti = winreg::read_dword(Root::LocalMachine, MM_PROFILE, "NetworkThrottlingIndex")
            .ok()
            .flatten();
        let sr = winreg::read_dword(Root::LocalMachine, MM_PROFILE, "SystemResponsiveness")
            .ok()
            .flatten();
        let text = format!(
            "NetworkThrottlingIndex={}, SystemResponsiveness={}",
            nti.map(|v| format!("0x{v:x}"))
                .unwrap_or_else(|| crate::i18n::tw_throttle_default_10().into()),
            sr.map(|v| v.to_string())
                .unwrap_or_else(|| crate::i18n::tw_throttle_default_20().into())
        );
        State::new(text, Some(nti == Some(0xFFFF_FFFF)), json!({ "nti": nti, "sr": sr }))
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        winreg::write_dword(Root::LocalMachine, MM_PROFILE, "NetworkThrottlingIndex", 0xFFFF_FFFF)?;
        winreg::write_dword(Root::LocalMachine, MM_PROFILE, "SystemResponsiveness", 10)?;
        Ok(crate::i18n::tw_throttle_applied().into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        for (name, field) in
            [("NetworkThrottlingIndex", "nti"), ("SystemResponsiveness", "sr")]
        {
            match snapshot[field].as_u64() {
                Some(v) => winreg::write_dword(Root::LocalMachine, MM_PROFILE, name, v as u32)?,
                None => winreg::delete_value(Root::LocalMachine, MM_PROFILE, name)?,
            }
        }
        Ok(crate::i18n::tw_throttle_reverted().into())
    }
}

// ---------------------------------------------------------------------------
// MTU
// ---------------------------------------------------------------------------

pub struct MtuFix;

impl MtuFix {
    /// Binary search for the largest payload that survives without
    /// fragmentation, converted to an MTU by adding the 28-byte IP+ICMP header.
    pub fn probe_best_mtu(host: std::net::Ipv4Addr) -> Option<u32> {
        use crate::probe::icmp::probe_df;
        let (mut lo, mut hi, mut best) = (1200u32, 1472u32, None);
        while lo <= hi {
            let mid = (lo + hi) / 2;
            if probe_df(host, mid as u16, 1500) {
                best = Some(mid);
                lo = mid + 1;
            } else {
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            }
        }
        best.map(|b| b + 28)
    }

    fn current(net: &NetState) -> Option<u32> {
        let out = run("netsh", &["interface", "ipv4", "show", "subinterfaces"]).ok()?;
        for line in out.lines() {
            if line.to_lowercase().contains(&net.adapter_name.to_lowercase())
                && !net.adapter_name.is_empty()
            {
                return line.split_whitespace().next()?.parse().ok();
            }
        }
        None
    }
}

impl Tweak for MtuFix {
    fn id(&self) -> &'static str {
        "mtu"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_mtu_title()
    }
    fn what(&self) -> &'static str {
        crate::i18n::tw_mtu_what()
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_mtu_why()
    }
    fn risk(&self) -> Risk {
        Risk::Medium
    }
    fn category(&self) -> Category {
        Category::Throughput
    }

    fn read(&self, net: &NetState) -> State {
        match Self::current(net) {
            Some(mtu) => State::new(crate::i18n::tw_mtu_state(mtu), None, json!({ "mtu": mtu })),
            None => State::new(crate::i18n::tw_mtu_unknown(), None, Value::Null),
        }
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let best = Self::probe_best_mtu(std::net::Ipv4Addr::new(1, 1, 1, 1))
            .ok_or_else(|| anyhow!(crate::i18n::tw_mtu_probe_failed()))?;
        run("netsh", &["interface", "ipv4", "set", "subinterface", &net.adapter_name, &format!("mtu={best}"), "store=persistent"])?;
        Ok(crate::i18n::tw_mtu_applied(best))
    }

    fn revert(&self, net: &NetState, snapshot: &Value) -> Result<String> {
        let mtu = snapshot["mtu"].as_u64().unwrap_or(1500);
        run("netsh", &["interface", "ipv4", "set", "subinterface", &net.adapter_name, &format!("mtu={mtu}"), "store=persistent"])?;
        Ok(crate::i18n::tw_mtu_reverted(mtu))
    }
}

// ---------------------------------------------------------------------------
// stack reset
// ---------------------------------------------------------------------------

pub struct StackReset;

impl Tweak for StackReset {
    fn id(&self) -> &'static str {
        "stack_reset"
    }
    fn title(&self) -> &'static str {
        crate::i18n::tw_reset_title()
    }
    fn what(&self) -> &'static str {
        "ipconfig /flushdns, /release, /renew, netsh winsock reset, netsh int ip reset."
    }
    fn why(&self) -> &'static str {
        crate::i18n::tw_reset_why()
    }
    fn risk(&self) -> Risk {
        Risk::Medium
    }
    fn category(&self) -> Category {
        Category::LastResort
    }
    fn reversible(&self) -> bool {
        false
    }
    fn needs_reboot(&self) -> bool {
        true
    }

    fn read(&self, _net: &NetState) -> State {
        State::new(crate::i18n::tw_reset_state(), None, Value::Null)
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        let steps: [(&str, &[&str]); 5] = [
            ("ipconfig", &["/flushdns"]),
            ("ipconfig", &["/release"]),
            ("ipconfig", &["/renew"]),
            ("netsh", &["winsock", "reset"]),
            ("netsh", &["int", "ip", "reset"]),
        ];
        let mut done = Vec::new();
        for (prog, args) in steps {
            let label = args.join(" ");
            done.push(match run(prog, args) {
                Ok(_) => format!("{label} {}", crate::i18n::tw_reset_step_ok()),
                Err(_) => format!("{label} {}", crate::i18n::tw_reset_step_failed()),
            });
        }
        Ok(crate::i18n::tw_reset_done(&done.join(", ")))
    }

    fn revert(&self, _net: &NetState, _snapshot: &Value) -> Result<String> {
        Err(anyhow!(crate::i18n::tw_reset_irreversible()))
    }
}

pub fn all() -> Vec<Box<dyn Tweak>> {
    let mut tweaks: Vec<Box<dyn Tweak>> = vec![
        Box::new(AdapterPowerSaving),
        Box::new(WlanPowerPlan),
        Box::new(FastDns),
        Box::new(TcpAutotuning),
        Box::new(NagleOff),
        Box::new(NetworkThrottling),
        Box::new(MtuFix),
        Box::new(StackReset),
    ];
    tweaks.extend(radio::all());
    tweaks.extend(stack::all());
    tweaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tweak_has_copy_and_a_unique_id() {
        let tweaks = all();
        let mut ids = std::collections::HashSet::new();
        for t in &tweaks {
            assert!(ids.insert(t.id()), "duplicate id {}", t.id());
            assert!(!t.title().is_empty());
            assert!(!t.what().is_empty(), "{} has no description", t.id());
            assert!(!t.why().is_empty(), "{} does not say why it helps", t.id());
        }
        // The number is here to catch a tweak silently dropped from `all()`:
        // eight core ones, plus the radio and stack modules.
        assert_eq!(tweaks.len(), 8 + radio::all().len() + stack::all().len());
        assert_eq!(tweaks.len(), 21);
    }

    #[test]
    fn every_tweak_lands_in_a_section_and_no_section_is_empty() {
        let tweaks = all();
        for category in Category::ALL {
            assert!(
                tweaks.iter().any(|t| t.category() == category),
                "{category:?} would render as an empty heading"
            );
        }
        let grouped: usize = Category::ALL
            .iter()
            .map(|c| tweaks.iter().filter(|t| t.category() == *c).count())
            .sum();
        assert_eq!(grouped, tweaks.len(), "a tweak belongs to no section and would vanish");
    }

    #[test]
    fn reading_state_never_panics_on_the_live_machine() {
        let net = crate::probe::netstate::read();
        for t in all() {
            let state = t.read(&net);
            assert!(!state.text.is_empty(), "{} produced no status text", t.id());
        }
    }

    #[test]
    fn irreversible_tweaks_refuse_to_revert() {
        let net = NetState::default();
        assert!(StackReset.revert(&net, &Value::Null).is_err());
    }

    #[test]
    fn revert_without_a_snapshot_is_refused() {
        // stack_reset is never snapshotted, so this exercises the guard
        // without depending on the user's real snapshot file.
        let net = NetState::default();
        let _guard = crate::i18n::test_lock();
        let err = revert(&StackReset, &net).unwrap_err().to_string();
        assert!(
            err == crate::i18n::tw_revert_needs_admin() || err == crate::i18n::tw_no_snapshot(),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn power_plan_index_parsing() {
        let sample = "  Current AC Power Setting Index: 0x00000000\n  \
                      Current DC Power Setting Index: 0x00000003\n";
        assert_eq!(extract_index(sample, AC_INDEX_LABELS, 0), Some(0));
        assert_eq!(extract_index(sample, DC_INDEX_LABELS, 1), Some(3));
        assert_eq!(extract_index("nothing here", AC_INDEX_LABELS, 0), None);
    }

    #[test]
    fn power_plan_index_survives_an_unknown_locale() {
        // A label we do not ship a translation for: the ordinal fallback has
        // to carry it, because powercfg always prints AC before DC.
        let sample = "  Index podesetavanja AC: 0x00000000\n  \
                      Index podesetavanja DC: 0x00000002\n";
        assert_eq!(extract_index(sample, AC_INDEX_LABELS, 0), Some(0));
        assert_eq!(extract_index(sample, DC_INDEX_LABELS, 1), Some(2));
    }

    #[test]
    fn autotuning_level_normalises_to_what_netsh_accepts() {
        assert_eq!(canonical_autotune_level("normal"), "normal");
        assert_eq!(canonical_autotune_level("normalny"), "normal");
        assert_eq!(canonical_autotune_level("wyłączone"), "disabled");
        assert_eq!(canonical_autotune_level("highly restricted"), "highlyrestricted");
        assert_eq!(canonical_autotune_level("something else"), "normal");
    }
}
