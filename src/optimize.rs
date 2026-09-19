//! Tweaks: each one can be inspected, applied and reverted.
//!
//! Nothing here runs by itself. Every tweak records the previous value to
//! `tweak_snapshots.json` before touching anything, so "Revert" restores the
//! exact state the machine was in — including after a reboot, which is when it
//! matters most, because several of these only take effect after one.

use std::collections::HashMap;
use std::process::Command;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::probe::netstate::NetState;
use crate::settings;
use crate::winreg::{self, Root};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Low,
    Medium,
}

impl Risk {
    pub fn label(&self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
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
        return Err(anyhow!("This change requires administrator rights."));
    }
    let state = tweak.read(net);
    let mut snaps = load_snapshots();
    snaps.insert(tweak.id().to_string(), state.snapshot.clone());
    save_snapshots(&snaps)?;

    tweak.apply(net)
}

pub fn revert(tweak: &dyn Tweak, net: &NetState) -> Result<String> {
    if tweak.needs_admin() && !is_elevated() {
        return Err(anyhow!("Reverting requires administrator rights."));
    }
    let snaps = load_snapshots();
    let Some(snapshot) = snaps.get(tweak.id()) else {
        return Err(anyhow!("No saved state for this change — nothing to revert to."));
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
fn run(program: &str, args: &[&str]) -> Result<String> {
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
fn adapter_class_key(net: &NetState) -> Option<String> {
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
        "Stop Windows powering down the network adapter"
    }
    fn what(&self) -> &'static str {
        "Clears \"Allow the computer to turn off this device to save power\" for the adapter."
    }
    fn why(&self) -> &'static str {
        "The most common cause of connections dropping \"for no reason\" on laptops. Windows \
         suspends the card when idle and waking it takes long enough for sessions to die."
    }
    fn risk(&self) -> Risk {
        Risk::Low
    }
    fn needs_reboot(&self) -> bool {
        true
    }

    fn read(&self, net: &NetState) -> State {
        let Some(key) = adapter_class_key(net) else {
            return State::new("adapter not found in the registry", None, Value::Null);
        };
        match winreg::read_dword(Root::LocalMachine, &key, "PnPCapabilities") {
            Ok(Some(v)) if v & PNP_DISABLE_POWER_DOWN == PNP_DISABLE_POWER_DOWN => State::new(
                format!("disabled (PnPCapabilities={v})"),
                Some(true),
                json!({ "key": key, "value": v }),
            ),
            Ok(current) => State::new(
                match current {
                    Some(v) => format!("enabled — Windows may suspend the card (PnPCapabilities={v})"),
                    None => "enabled — Windows may suspend the card (value not set)".to_string(),
                },
                Some(false),
                json!({ "key": key, "value": current }),
            ),
            Err(e) => State::new(format!("cannot read: {e}"), None, Value::Null),
        }
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let key = adapter_class_key(net).ok_or_else(|| anyhow!("adapter key not found"))?;
        winreg::write_dword(Root::LocalMachine, &key, "PnPCapabilities", PNP_DISABLE_POWER_DOWN)?;
        Ok("Power management disabled for the adapter. Takes effect after a restart \
            (or disabling and re-enabling the adapter)."
            .into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let key = snapshot["key"].as_str().ok_or_else(|| anyhow!("snapshot has no key"))?;
        match snapshot["value"].as_u64() {
            Some(v) => {
                winreg::write_dword(Root::LocalMachine, key, "PnPCapabilities", v as u32)?
            }
            None => winreg::delete_value(Root::LocalMachine, key, "PnPCapabilities")?,
        }
        Ok("Previous value restored. Takes effect after a restart.".into())
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
        "Wi-Fi radio at maximum performance in the power plan"
    }
    fn what(&self) -> &'static str {
        "Sets Wireless Adapter Settings → Power Saving Mode to Maximum Performance, on both \
         mains and battery."
    }
    fn why(&self) -> &'static str {
        "A second, independent throttle. Even with driver power management off, the power plan \
         can still cut transmit power and cause drops."
    }
    fn risk(&self) -> Risk {
        Risk::Low
    }

    fn read(&self, _net: &NetState) -> State {
        let out = match run("powercfg", &["/query", "SCHEME_CURRENT", SUB_WIRELESS, SETTING_POWER_SAVING])
        {
            Ok(t) => t,
            Err(e) => return State::new(format!("cannot read: {e}"), None, Value::Null),
        };
        let ac = extract_hex(&out, "Current AC Power Setting Index");
        let dc = extract_hex(&out, "Current DC Power Setting Index");
        let (Some(ac), Some(dc)) = (ac, dc) else {
            return State::new("setting not present in this power plan", None, Value::Null);
        };
        let name = |v: u32| match v {
            0 => "max performance",
            1 => "low saving",
            2 => "medium saving",
            _ => "max saving",
        };
        State::new(
            format!("mains: {} / battery: {}", name(ac), name(dc)),
            Some(ac == 0 && dc == 0),
            json!({ "ac": ac, "dc": dc }),
        )
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        set_power_indices(0, 0)?;
        Ok("Wi-Fi radio set to maximum performance on mains and battery.".into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let ac = snapshot["ac"].as_u64().unwrap_or(0) as u32;
        let dc = snapshot["dc"].as_u64().unwrap_or(3) as u32;
        set_power_indices(ac, dc)?;
        Ok("Previous power plan values restored.".into())
    }
}

fn set_power_indices(ac: u32, dc: u32) -> Result<()> {
    run("powercfg", &["/setacvalueindex", "SCHEME_CURRENT", SUB_WIRELESS, SETTING_POWER_SAVING, &ac.to_string()])?;
    run("powercfg", &["/setdcvalueindex", "SCHEME_CURRENT", SUB_WIRELESS, SETTING_POWER_SAVING, &dc.to_string()])?;
    run("powercfg", &["/setactive", "SCHEME_CURRENT"])?;
    Ok(())
}

fn extract_hex(text: &str, label: &str) -> Option<u32> {
    let line = text.lines().find(|l| l.contains(label))?;
    let hex = line.split("0x").nth(1)?.trim();
    u32::from_str_radix(hex, 16).ok()
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
        "Fast, independent DNS servers"
    }
    fn what(&self) -> &'static str {
        "Sets 1.1.1.1 and 8.8.8.8 on the active adapter instead of the DHCP-supplied servers."
    }
    fn why(&self) -> &'static str {
        "When the router is the only resolver, its hiccup looks exactly like \"the internet is \
         down\": pings by IP work, but nothing loads."
    }
    fn risk(&self) -> Risk {
        Risk::Low
    }

    fn read(&self, net: &NetState) -> State {
        let current: Vec<String> = net.dns_servers.iter().map(|d| d.to_string()).collect();
        let good = ["1.1.1.1", "1.0.0.1", "8.8.8.8", "8.8.4.4", "9.9.9.9"];
        let mut text = if current.is_empty() {
            "none / from DHCP".to_string()
        } else {
            current.join(", ")
        };
        if net.dns_is_router_only() {
            text.push_str("  (router only — single point of failure)");
        }
        let optimal = current.iter().any(|d| good.contains(&d.as_str()));
        State::new(text, Some(optimal), json!({ "servers": current }))
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let name = &net.adapter_name;
        run("netsh", &["interface", "ipv4", "set", "dnsservers", &format!("name={name}"), "source=static", "address=1.1.1.1", "register=primary", "validate=no"])?;
        run("netsh", &["interface", "ipv4", "add", "dnsservers", &format!("name={name}"), "address=8.8.8.8", "index=2", "validate=no"])?;
        let _ = run("ipconfig", &["/flushdns"]);
        Ok("DNS set to 1.1.1.1 and 8.8.8.8.".into())
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
            return Ok("DNS returned to DHCP.".into());
        }
        run("netsh", &["interface", "ipv4", "set", "dnsservers", &format!("name={name}"), "source=static", &format!("address={}", servers[0]), "register=primary", "validate=no"])?;
        for (i, s) in servers.iter().skip(1).enumerate() {
            let _ = run("netsh", &["interface", "ipv4", "add", "dnsservers", &format!("name={name}"), &format!("address={s}"), &format!("index={}", i + 2), "validate=no"]);
        }
        let _ = run("ipconfig", &["/flushdns"]);
        Ok("Previous DNS servers restored.".into())
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
        "TCP receive window auto-tuning = normal"
    }
    fn what(&self) -> &'static str {
        "netsh int tcp set global autotuninglevel=normal"
    }
    fn why(&self) -> &'static str {
        "\"Ping boost\" guides tell people to disable this, which cripples throughput on any \
         fast link. Normal is the correct value; this undoes that damage."
    }
    fn risk(&self) -> Risk {
        Risk::Low
    }

    fn read(&self, _net: &NetState) -> State {
        let out = match run("netsh", &["int", "tcp", "show", "global"]) {
            Ok(t) => t,
            Err(e) => return State::new(format!("cannot read: {e}"), None, Value::Null),
        };
        let Some(line) = out.lines().find(|l| l.contains("Auto-Tuning Level")) else {
            return State::new("cannot read", None, Value::Null);
        };
        let value = line.split(':').nth(1).unwrap_or("").trim().to_lowercase();
        State::new(value.clone(), Some(value == "normal"), json!({ "level": value }))
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        run("netsh", &["int", "tcp", "set", "global", "autotuninglevel=normal"])?;
        Ok("Auto-tuning set to normal.".into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let level = snapshot["level"].as_str().unwrap_or("normal");
        run("netsh", &["int", "tcp", "set", "global", &format!("autotuninglevel={level}")])?;
        Ok(format!("Auto-tuning restored to {level}."))
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
        "Disable Nagle's algorithm (for games)"
    }
    fn what(&self) -> &'static str {
        "Writes TcpAckFrequency=1 and TCPNoDelay=1 for the active interface."
    }
    fn why(&self) -> &'static str {
        "Windows buffers small packets and delays acknowledgements. In twitch games that is a \
         few to a dozen extra milliseconds. It makes no difference to downloads."
    }
    fn risk(&self) -> Risk {
        Risk::Medium
    }
    fn needs_reboot(&self) -> bool {
        true
    }

    fn read(&self, net: &NetState) -> State {
        let Some(key) = Self::key(net) else {
            return State::new("adapter GUID unknown", None, Value::Null);
        };
        let ack = winreg::read_dword(Root::LocalMachine, &key, "TcpAckFrequency").ok().flatten();
        let nodelay = winreg::read_dword(Root::LocalMachine, &key, "TCPNoDelay").ok().flatten();
        let text = format!(
            "TcpAckFrequency={}, TCPNoDelay={}",
            ack.map(|v| v.to_string()).unwrap_or_else(|| "not set".into()),
            nodelay.map(|v| v.to_string()).unwrap_or_else(|| "not set".into())
        );
        State::new(
            text,
            Some(ack == Some(1) && nodelay == Some(1)),
            json!({ "key": key, "ack": ack, "nodelay": nodelay }),
        )
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let key = Self::key(net).ok_or_else(|| anyhow!("adapter GUID unknown"))?;
        winreg::write_dword(Root::LocalMachine, &key, "TcpAckFrequency", 1)?;
        winreg::write_dword(Root::LocalMachine, &key, "TCPNoDelay", 1)?;
        Ok("Nagle disabled. Requires a restart.".into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let key = snapshot["key"].as_str().ok_or_else(|| anyhow!("snapshot has no key"))?;
        for (name, field) in [("TcpAckFrequency", "ack"), ("TCPNoDelay", "nodelay")] {
            match snapshot[field].as_u64() {
                Some(v) => winreg::write_dword(Root::LocalMachine, key, name, v as u32)?,
                None => winreg::delete_value(Root::LocalMachine, key, name)?,
            }
        }
        Ok("Previous state restored. Requires a restart.".into())
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
        "Lift the multimedia packet throttle"
    }
    fn what(&self) -> &'static str {
        "NetworkThrottlingIndex = 0xffffffff, SystemResponsiveness = 10."
    }
    fn why(&self) -> &'static str {
        "Windows caps network traffic at roughly 10k packets/s while any multimedia playback is \
         running. Gaming with a stream or music on shows this up as lag."
    }
    fn risk(&self) -> Risk {
        Risk::Medium
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
            nti.map(|v| format!("0x{v:x}")).unwrap_or_else(|| "default (10)".into()),
            sr.map(|v| v.to_string()).unwrap_or_else(|| "default (20)".into())
        );
        State::new(text, Some(nti == Some(0xFFFF_FFFF)), json!({ "nti": nti, "sr": sr }))
    }

    fn apply(&self, _net: &NetState) -> Result<String> {
        winreg::write_dword(Root::LocalMachine, MM_PROFILE, "NetworkThrottlingIndex", 0xFFFF_FFFF)?;
        winreg::write_dword(Root::LocalMachine, MM_PROFILE, "SystemResponsiveness", 10)?;
        Ok("Throttle lifted. Requires a restart.".into())
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
        Ok("Defaults restored. Requires a restart.".into())
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
        "Correct the adapter MTU"
    }
    fn what(&self) -> &'static str {
        "Sets MTU to the largest size that survives a fragmentation test (usually 1500, or 1492 \
         on PPPoE)."
    }
    fn why(&self) -> &'static str {
        "An MTU that is too large means packets get dropped somewhere along the path. The \
         symptom is pages that never finish loading while ping works fine."
    }
    fn risk(&self) -> Risk {
        Risk::Medium
    }

    fn read(&self, net: &NetState) -> State {
        match Self::current(net) {
            Some(mtu) => State::new(format!("MTU = {mtu}"), None, json!({ "mtu": mtu })),
            None => State::new("MTU unknown", None, Value::Null),
        }
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let best = Self::probe_best_mtu(std::net::Ipv4Addr::new(1, 1, 1, 1))
            .ok_or_else(|| anyhow!("MTU probe produced no result (is DF-flagged ICMP blocked?)"))?;
        run("netsh", &["interface", "ipv4", "set", "subinterface", &net.adapter_name, &format!("mtu={best}"), "store=persistent"])?;
        Ok(format!("MTU set to {best}."))
    }

    fn revert(&self, net: &NetState, snapshot: &Value) -> Result<String> {
        let mtu = snapshot["mtu"].as_u64().unwrap_or(1500);
        run("netsh", &["interface", "ipv4", "set", "subinterface", &net.adapter_name, &format!("mtu={mtu}"), "store=persistent"])?;
        Ok(format!("MTU restored to {mtu}."))
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
        "Reset the network stack (repair action)"
    }
    fn what(&self) -> &'static str {
        "ipconfig /flushdns, /release, /renew, netsh winsock reset, netsh int ip reset."
    }
    fn why(&self) -> &'static str {
        "For when the connection has already died and will not come back. Clears broken \
         Winsock/IP state that otherwise persists until a reboot."
    }
    fn risk(&self) -> Risk {
        Risk::Medium
    }
    fn reversible(&self) -> bool {
        false
    }
    fn needs_reboot(&self) -> bool {
        true
    }

    fn read(&self, _net: &NetState) -> State {
        State::new("one-off action — nothing is permanently changed", None, Value::Null)
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
                Ok(_) => format!("{label} ok"),
                Err(_) => format!("{label} failed"),
            });
        }
        Ok(format!("Ran: {}. A restart is recommended.", done.join(", ")))
    }

    fn revert(&self, _net: &NetState, _snapshot: &Value) -> Result<String> {
        Err(anyhow!("This action cannot be undone."))
    }
}

pub fn all() -> Vec<Box<dyn Tweak>> {
    vec![
        Box::new(AdapterPowerSaving),
        Box::new(WlanPowerPlan),
        Box::new(FastDns),
        Box::new(TcpAutotuning),
        Box::new(NagleOff),
        Box::new(NetworkThrottling),
        Box::new(MtuFix),
        Box::new(StackReset),
    ]
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
        assert_eq!(tweaks.len(), 8);
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
        let err = revert(&StackReset, &net).unwrap_err().to_string();
        assert!(
            err.contains("administrator") || err.contains("No saved state"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn power_plan_index_parsing() {
        let sample = "  Current AC Power Setting Index: 0x00000000\n  \
                      Current DC Power Setting Index: 0x00000003\n";
        assert_eq!(extract_hex(sample, "Current AC Power Setting Index"), Some(0));
        assert_eq!(extract_hex(sample, "Current DC Power Setting Index"), Some(3));
        assert_eq!(extract_hex(sample, "Nonexistent"), None);
    }
}
