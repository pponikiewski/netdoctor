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

    /// The name this tweak's "before" value is filed under.
    ///
    /// Defaults to the id, which is right for everything that changes one
    /// machine-wide setting. A tweak that writes to a key belonging to *this
    /// adapter* has to say so, because the same id then means a different
    /// value on every card: apply it on the Wi-Fi, dock the laptop, apply it
    /// on the Ethernet, and the second machine-state was never recorded —
    /// `record_first` saw the id already there and kept quiet, leaving Revert
    /// pointing at the first card and the second changed for good.
    fn scope_key(&self, _net: &NetState) -> String {
        self.id().to_string()
    }

    fn read(&self, net: &NetState) -> State;
    fn apply(&self, net: &NetState) -> Result<String>;
    fn revert(&self, net: &NetState, snapshot: &Value) -> Result<String>;
}

// ---------------------------------------------------------------------------
// snapshot persistence
// ---------------------------------------------------------------------------

/// A missing file means "nothing has been applied yet", which is normal. A
/// file that exists but does not parse is an error the caller has to see:
/// treating it as empty would hide every recorded "before" value and let the
/// next apply overwrite the only copy of it.
fn read_snapshots() -> Result<HashMap<String, Value>> {
    read_snapshots_at(&settings::snapshot_path())
}

fn read_snapshots_at(path: &std::path::Path) -> Result<HashMap<String, Value>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => {
            return Err(anyhow!(crate::i18n::tw_snapshots_unreadable(
                &path.display().to_string(),
                &e.to_string()
            )))
        }
    };
    serde_json::from_str(&text).map_err(|e| {
        anyhow!(crate::i18n::tw_snapshots_unreadable(&path.display().to_string(), &e.to_string()))
    })
}

/// `None` when the file is fine. The UI shows this instead of quietly
/// dropping every Revert button.
pub fn snapshots_error() -> Option<String> {
    read_snapshots().err().map(|e| e.to_string())
}

/// Write to a sibling temp file and rename over the original. A crash or a
/// power cut mid-write then loses the new entry rather than the whole file,
/// which is the only record of what the machine looked like before.
fn save_snapshots(map: &HashMap<String, Value>) -> Result<()> {
    std::fs::create_dir_all(settings::data_dir())?;
    save_snapshots_at(&settings::snapshot_path(), map)
}

fn save_snapshots_at(path: &std::path::Path, map: &HashMap<String, Value>) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(map)?)?;
    // Windows `rename` replaces the destination, so this is atomic enough:
    // a reader sees either the old file or the new one, never a truncated one.
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The rule that makes Revert trustworthy: the *first* recorded state wins.
///
/// Returns true if this call is what stored it. A second Apply of the same
/// tweak must not overwrite the entry, or the value it recorded would be the
/// tweak's own, and Revert would restore the tweak instead of undoing it.
fn record_first(snaps: &mut HashMap<String, Value>, id: &str, snapshot: Value) -> bool {
    if snaps.contains_key(id) {
        return false;
    }
    snaps.insert(id.to_string(), snapshot);
    true
}

/// Builds the per-adapter scope key, or falls back to the bare id when there
/// is no adapter to name. A snapshot filed under the bare id is still a
/// snapshot; see `snapshot_key_for`.
pub(crate) fn per_adapter_key(id: &str, net: &NetState) -> String {
    if net.adapter_guid.is_empty() {
        return id.to_string();
    }
    format!("{id}@{}", net.adapter_guid)
}

/// Which key in the snapshot file this tweak's "before" value is under.
///
/// The scoped name first, then the bare id. The fallback is for entries
/// written before snapshots knew about adapters: dropping them would make
/// Revert vanish for anyone who had already applied something, and each of
/// those tweaks records the registry key it read inside its own snapshot, so
/// reverting one still writes to the card it was taken from.
fn snapshot_key_for(
    snaps: &HashMap<String, Value>,
    tweak: &dyn Tweak,
    net: &NetState,
) -> Option<String> {
    let scoped = tweak.scope_key(net);
    if snaps.contains_key(&scoped) {
        return Some(scoped);
    }
    let bare = tweak.id().to_string();
    snaps.contains_key(&bare).then_some(bare)
}

pub fn has_snapshot(tweak: &dyn Tweak, net: &NetState) -> bool {
    read_snapshots().map(|m| snapshot_key_for(&m, tweak, net).is_some()).unwrap_or(false)
}

/// Apply, then record the state it replaced — and only the *first* time.
///
/// Both halves of that matter. Snapshotting after the change means a failed
/// apply leaves no Revert button for something that never happened. Keeping
/// the first snapshot means a second Apply cannot record the already-tweaked
/// value as the original, which would turn Revert into a no-op and strand the
/// machine on the tweak for good.
pub fn apply(tweak: &dyn Tweak, net: &NetState) -> Result<String> {
    if tweak.needs_admin() && !is_elevated() {
        return Err(anyhow!(crate::i18n::tw_needs_admin()));
    }
    // Refuses rather than starting from an empty map: see `read_snapshots`.
    let mut snaps = read_snapshots()?;
    let state = tweak.read(net);
    if !before_is_known(tweak.reversible(), &state.snapshot) {
        return Err(anyhow!(crate::i18n::tw_before_unknown()));
    }

    let msg = tweak.apply(net)?;

    if !record_first(&mut snaps, &tweak.scope_key(net), state.snapshot) {
        return Ok(msg);
    }
    match save_snapshots(&snaps) {
        Ok(()) => Ok(msg),
        // The change is already live, so this is a warning, not a failure.
        Err(e) => Ok(format!(
            "{msg}
{}",
            crate::i18n::tw_snapshot_save_failed(&e.to_string())
        )),
    }
}

/// Whether an apply would leave something to revert to.
///
/// A read that failed returns a `Null` snapshot, and recording that as the
/// "before" made Revert either refuse (no key to write back to) or restore
/// made-up defaults. A tweak that cannot be reverted anyway has nothing to
/// record, so it is not held to this. Keyed on the snapshot rather than on
/// `optimal`: the MTU reads a real value without judging it.
fn before_is_known(reversible: bool, snapshot: &Value) -> bool {
    !reversible || !snapshot.is_null()
}

pub fn revert(tweak: &dyn Tweak, net: &NetState) -> Result<String> {
    if tweak.needs_admin() && !is_elevated() {
        return Err(anyhow!(crate::i18n::tw_revert_needs_admin()));
    }
    let mut snaps = read_snapshots()?;
    let Some(key) = snapshot_key_for(&snaps, tweak, net) else {
        return Err(anyhow!(crate::i18n::tw_no_snapshot()));
    };
    let snapshot = &snaps[&key];
    let msg = tweak.revert(net, snapshot)?;
    snaps.remove(&key);
    save_snapshots(&snaps)?;
    Ok(msg)
}

pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
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

/// Resolves a Windows console tool to its full path under System32.
///
/// `Command::new("netsh")` would let `CreateProcess` search the application
/// directory and the working directory before `PATH`. Every one of these
/// tools runs from an elevated process, so a `netsh.exe` dropped next to our
/// executable would inherit administrator rights. Naming the real one closes
/// that door.
#[cfg(windows)]
fn system_tool(program: &str) -> std::path::PathBuf {
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

    let mut buf = [0u16; 260];
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    let dir = if len == 0 || len > buf.len() {
        // Only reachable if the call fails outright; an absolute fallback is
        // still better than letting the search path decide.
        std::path::PathBuf::from(r"C:\Windows\System32")
    } else {
        std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len]))
    };
    // A caller that already spelled the extension gets the same path as one
    // that did not: appending blindly produced `wevtutil.exe.exe`, which does
    // not exist, and the failure was silent at every call site.
    let stem = match program.len().checked_sub(4) {
        Some(cut) if program[cut..].eq_ignore_ascii_case(".exe") => &program[..cut],
        _ => program,
    };
    dir.join(format!("{stem}.exe"))
}

#[cfg(not(windows))]
fn system_tool(program: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(program)
}

/// Run a console tool without flashing a window.
pub(crate) fn run(program: &str, args: &[&str]) -> Result<String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    let mut cmd = Command::new(system_tool(program));
    cmd.args(args);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd.output()?;
    let text =
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
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

    fn scope_key(&self, net: &NetState) -> String {
        per_adapter_key(self.id(), net)
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
        let key =
            adapter_class_key(net).ok_or_else(|| anyhow!(crate::i18n::tw_no_adapter_key()))?;
        winreg::write_dword(Root::LocalMachine, &key, "PnPCapabilities", PNP_DISABLE_POWER_DOWN)?;
        Ok(crate::i18n::tw_power_applied().into())
    }

    fn revert(&self, _net: &NetState, snapshot: &Value) -> Result<String> {
        let key =
            snapshot["key"].as_str().ok_or_else(|| anyhow!(crate::i18n::tw_snapshot_no_key()))?;
        match snapshot["value"].as_u64() {
            Some(v) => winreg::write_dword(Root::LocalMachine, key, "PnPCapabilities", v as u32)?,
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
        let out = match run(
            "powercfg",
            &["/query", "SCHEME_CURRENT", SUB_WIRELESS, SETTING_POWER_SAVING],
        ) {
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
    run(
        "powercfg",
        &[
            "/setacvalueindex",
            "SCHEME_CURRENT",
            SUB_WIRELESS,
            SETTING_POWER_SAVING,
            &ac.to_string(),
        ],
    )?;
    run(
        "powercfg",
        &[
            "/setdcvalueindex",
            "SCHEME_CURRENT",
            SUB_WIRELESS,
            SETTING_POWER_SAVING,
            &dc.to_string(),
        ],
    )?;
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
    /// Not Low. It hands every lookup to two third parties, and on a company
    /// network or a VPN it breaks split-horizon DNS — at which point internal
    /// names stop resolving and nothing about the symptom points back here.
    /// Medium keeps it out of "apply everything safe".
    fn risk(&self) -> Risk {
        Risk::Medium
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
        // The servers alone do not say where they came from: Windows lists
        // DHCP-assigned and typed-in resolvers alike. Restoring a DHCP list
        // as static pins the router's address, and every other network the
        // laptop joins then fails to resolve. Unknown origin, no snapshot:
        // `apply` refuses rather than guess.
        let snapshot = match dns_set_by_hand(net) {
            Some(by_hand) => json!({ "servers": current, "dhcp": !by_hand }),
            None => Value::Null,
        };
        State::new(text, Some(optimal), snapshot)
    }

    fn apply(&self, net: &NetState) -> Result<String> {
        let name = &net.adapter_name;
        run(
            "netsh",
            &[
                "interface",
                "ipv4",
                "set",
                "dnsservers",
                &format!("name={name}"),
                "source=static",
                "address=1.1.1.1",
                "register=primary",
                "validate=no",
            ],
        )?;
        run(
            "netsh",
            &[
                "interface",
                "ipv4",
                "add",
                "dnsservers",
                &format!("name={name}"),
                "address=8.8.8.8",
                "index=2",
                "validate=no",
            ],
        )?;
        let _ = run("ipconfig", &["/flushdns"]);
        Ok(crate::i18n::tw_dns_applied().into())
    }

    fn revert(&self, net: &NetState, snapshot: &Value) -> Result<String> {
        let name = &net.adapter_name;
        let servers: Vec<String> = snapshot["servers"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        if restore_dhcp(snapshot, &servers, &dhcp_offered(net)) {
            run(
                "netsh",
                &["interface", "ipv4", "set", "dnsservers", &format!("name={name}"), "source=dhcp"],
            )?;
            let _ = run("ipconfig", &["/flushdns"]);
            return Ok(crate::i18n::tw_dns_reverted_dhcp().into());
        }
        run(
            "netsh",
            &[
                "interface",
                "ipv4",
                "set",
                "dnsservers",
                &format!("name={name}"),
                "source=static",
                &format!("address={}", servers[0]),
                "register=primary",
                "validate=no",
            ],
        )?;
        for (i, s) in servers.iter().skip(1).enumerate() {
            let _ = run(
                "netsh",
                &[
                    "interface",
                    "ipv4",
                    "add",
                    "dnsservers",
                    &format!("name={name}"),
                    &format!("address={s}"),
                    &format!("index={}", i + 2),
                    "validate=no",
                ],
            );
        }
        let _ = run("ipconfig", &["/flushdns"]);
        Ok(crate::i18n::tw_dns_reverted().into())
    }
}

/// Splits a resolver list as Tcpip stores it: comma- or space-separated.
fn split_servers(raw: &str) -> Vec<String> {
    raw.split([',', ' ']).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Whether the adapter's resolvers were typed in (`NameServer` set) rather
/// than handed out by DHCP. `None` when the key cannot be read.
fn dns_set_by_hand(net: &NetState) -> Option<bool> {
    let key = NagleOff::key(net)?;
    let raw = winreg::read_string(Root::LocalMachine, &key, "NameServer").ok()?;
    Some(raw.is_some_and(|r| !split_servers(&r).is_empty()))
}

/// What DHCP offers this adapter right now, kept by Windows even while a
/// static list overrides it.
fn dhcp_offered(net: &NetState) -> Vec<String> {
    NagleOff::key(net)
        .and_then(|key| winreg::read_string(Root::LocalMachine, &key, "DhcpNameServer").ok())
        .flatten()
        .map(|r| split_servers(&r))
        .unwrap_or_default()
}

/// Whether Revert should hand DNS back to DHCP instead of pinning `servers`.
///
/// Snapshots written before the origin was recorded carry only the list. For
/// those, a list identical to what DHCP offers now is read as DHCP: pinning
/// it would be the very fault, and a hand-typed copy of the DHCP answer loses
/// nothing by going back to DHCP while on this network.
fn restore_dhcp(snapshot: &Value, servers: &[String], dhcp_now: &[String]) -> bool {
    match snapshot["dhcp"].as_bool() {
        Some(dhcp) => dhcp,
        None => servers.is_empty() || servers == dhcp_now,
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

    fn scope_key(&self, net: &NetState) -> String {
        per_adapter_key(self.id(), net)
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
        let key =
            snapshot["key"].as_str().ok_or_else(|| anyhow!(crate::i18n::tw_snapshot_no_key()))?;
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

const MM_PROFILE: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile";

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
        for (name, field) in [("NetworkThrottlingIndex", "nti"), ("SystemResponsiveness", "sr")] {
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
        run(
            "netsh",
            &[
                "interface",
                "ipv4",
                "set",
                "subinterface",
                &net.adapter_name,
                &format!("mtu={best}"),
                "store=persistent",
            ],
        )?;
        Ok(crate::i18n::tw_mtu_applied(best))
    }

    fn revert(&self, net: &NetState, snapshot: &Value) -> Result<String> {
        let mtu = snapshot["mtu"].as_u64().unwrap_or(1500);
        run(
            "netsh",
            &[
                "interface",
                "ipv4",
                "set",
                "subinterface",
                &net.adapter_name,
                &format!("mtu={mtu}"),
                "store=persistent",
            ],
        )?;
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
    fn dns_from_dhcp_goes_back_to_dhcp_not_pinned() {
        let router = vec!["192.168.1.1".to_string()];
        // The case that used to pin the router: a DHCP list, recorded as such.
        let snap = json!({ "servers": router, "dhcp": true });
        assert!(restore_dhcp(&snap, &router, &[]));
        // Typed in by hand, even when it matches what DHCP offers.
        let snap = json!({ "servers": router, "dhcp": false });
        assert!(!restore_dhcp(&snap, &router, &router));
        // Snapshots from before the origin was recorded: judged by DHCP now.
        let legacy = json!({ "servers": router });
        assert!(restore_dhcp(&legacy, &router, &router));
        assert!(!restore_dhcp(&legacy, &["9.9.9.9".to_string()], &router));
        assert!(restore_dhcp(&json!({ "servers": [] }), &[], &router));
    }

    #[test]
    fn tcpip_resolver_lists_split_on_commas_and_spaces() {
        assert_eq!(split_servers("1.1.1.1,8.8.8.8"), ["1.1.1.1", "8.8.8.8"]);
        assert_eq!(split_servers("192.168.1.1 192.168.1.2"), ["192.168.1.1", "192.168.1.2"]);
        assert!(split_servers("").is_empty());
    }

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

    #[cfg(windows)]
    #[test]
    fn system_tools_resolve_to_a_real_absolute_path() {
        // If this ever resolves to something that is not there, every tweak
        // silently stops working — and pinning the path is the whole defence
        // against an elevated `netsh.exe` being picked up from elsewhere.
        for tool in ["netsh", "powercfg", "ipconfig"] {
            let path = system_tool(tool);
            assert!(path.is_absolute(), "{tool} must not go through PATH: {path:?}");
            assert!(path.exists(), "{tool} not found at {path:?}");
        }
    }

    /// A scratch file that cleans up after itself, so these tests never touch
    /// the real snapshot file in the user's profile.
    fn scratch(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("netdoctor-test-{name}.json"));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn the_first_recorded_state_is_the_one_that_is_kept() {
        let mut snaps = HashMap::new();

        assert!(record_first(&mut snaps, "dns", json!({ "servers": ["192.168.1.1"] })));
        // The second Apply sees the machine already tweaked. Recording that
        // would make Revert restore the tweak.
        assert!(!record_first(&mut snaps, "dns", json!({ "servers": ["1.1.1.1"] })));

        assert_eq!(snaps["dns"], json!({ "servers": ["192.168.1.1"] }));
    }

    #[test]
    fn a_missing_snapshot_file_is_not_an_error() {
        let path = scratch("absent");
        let map = read_snapshots_at(&path).expect("a first run has no file yet");
        assert!(map.is_empty());
    }

    #[test]
    fn a_damaged_snapshot_file_is_an_error_not_an_empty_map() {
        let path = scratch("damaged");
        // What a half-finished write leaves behind.
        std::fs::write(&path, r#"{"dns": {"servers""#).unwrap();

        let err = read_snapshots_at(&path).expect_err("truncated JSON must not read as empty");
        assert!(err.to_string().contains("damaged"), "the message names the file: {err}");

        // And the damaged file is still there to be repaired by hand.
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_replaces_the_file_and_leaves_no_temp_behind() {
        let path = scratch("roundtrip");
        let mut first = HashMap::new();
        first.insert("power".to_string(), json!({ "value": 0 }));
        save_snapshots_at(&path, &first).unwrap();

        let mut second = HashMap::new();
        second.insert("power".to_string(), json!({ "value": 24 }));
        save_snapshots_at(&path, &second).unwrap();

        assert_eq!(read_snapshots_at(&path).unwrap()["power"], json!({ "value": 24 }));
        assert!(!path.with_extension("json.tmp").exists(), "the temp file is renamed, not left");
        let _ = std::fs::remove_file(&path);
    }

    /// A tweak that writes to whichever adapter is current, like every one in
    /// `radio.rs` and like `adapter_power`.
    struct PerAdapter;

    impl Tweak for PerAdapter {
        fn id(&self) -> &'static str {
            "per_adapter"
        }
        fn title(&self) -> &'static str {
            "t"
        }
        fn what(&self) -> &'static str {
            "w"
        }
        fn why(&self) -> &'static str {
            "y"
        }
        fn risk(&self) -> Risk {
            Risk::Low
        }
        fn category(&self) -> Category {
            Category::Power
        }
        fn scope_key(&self, net: &NetState) -> String {
            per_adapter_key(self.id(), net)
        }
        fn read(&self, _net: &NetState) -> State {
            State::new("", Some(false), Value::Null)
        }
        fn apply(&self, _net: &NetState) -> Result<String> {
            Ok(String::new())
        }
        fn revert(&self, _net: &NetState, _snapshot: &Value) -> Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn an_unreadable_before_blocks_a_reversible_apply() {
        // What `read` returns when it fails: no snapshot to write back.
        assert!(!before_is_known(true, &Value::Null));
        assert!(before_is_known(true, &json!({ "mtu": 1500 })));
        // An irreversible tweak records nothing, so there is nothing to lose.
        assert!(before_is_known(false, &Value::Null));
    }

    fn on_adapter(guid: &str) -> NetState {
        NetState { adapter_guid: guid.to_string(), ..Default::default() }
    }

    #[test]
    fn a_second_adapter_gets_its_own_recorded_state() {
        // The bug this guards: snapshots were filed under the tweak id alone.
        // Apply on the Wi-Fi, dock the laptop, apply on the Ethernet, and
        // `record_first` saw the id already there and recorded nothing — so
        // Revert restored the Wi-Fi and left the Ethernet changed for good.
        let tweak = PerAdapter;
        let (a, b) = (on_adapter("{AAA}"), on_adapter("{BBB}"));
        let mut snaps = HashMap::new();

        assert!(record_first(&mut snaps, &tweak.scope_key(&a), json!({ "value": 1 })));
        assert!(
            record_first(&mut snaps, &tweak.scope_key(&b), json!({ "value": 2 })),
            "the second card's state was never seen before and has to be recorded"
        );

        let key = snapshot_key_for(&snaps, &tweak, &b).expect("card B has a snapshot");
        assert_eq!(snaps[&key], json!({ "value": 2 }), "revert on B must restore B");
        let key = snapshot_key_for(&snaps, &tweak, &a).expect("card A still has its own");
        assert_eq!(snaps[&key], json!({ "value": 1 }));
    }

    #[test]
    fn applying_twice_on_one_adapter_still_keeps_the_first_reading() {
        let tweak = PerAdapter;
        let a = on_adapter("{AAA}");
        let mut snaps = HashMap::new();

        assert!(record_first(&mut snaps, &tweak.scope_key(&a), json!({ "value": 1 })));
        assert!(
            !record_first(&mut snaps, &tweak.scope_key(&a), json!({ "value": 99 })),
            "the second apply must not record the tweak's own value as the original"
        );
        assert_eq!(snaps[&tweak.scope_key(&a)], json!({ "value": 1 }));
    }

    #[test]
    fn a_snapshot_from_before_this_change_is_still_revertible() {
        // Entries written by an earlier build are filed under the bare id.
        // Dropping them would make Revert vanish for anyone who had already
        // applied something.
        let tweak = PerAdapter;
        let a = on_adapter("{AAA}");
        let mut snaps = HashMap::new();
        snaps.insert("per_adapter".to_string(), json!({ "value": 7 }));

        let key = snapshot_key_for(&snaps, &tweak, &a).expect("the legacy entry counts");
        assert_eq!(key, "per_adapter");
        assert_eq!(snaps[&key], json!({ "value": 7 }));
    }

    #[test]
    fn the_tweaks_that_write_to_a_card_say_so_and_the_rest_do_not() {
        // The assertion that outlives this change: whether a real tweak is
        // filed per adapter is decided here, not in a test double. Every
        // tweak in radio.rs writes under this card's driver key, and so do
        // adapter_power and nagle_off.
        let net = on_adapter("{AAA}");
        let tweaks = all();
        let key_of = |id: &str| {
            tweaks
                .iter()
                .find(|t| t.id() == id)
                .unwrap_or_else(|| panic!("{id} is gone"))
                .scope_key(&net)
        };

        for id in ["adapter_power", "nagle_off", "radio_power_save", "nic_green_ethernet"] {
            assert_eq!(key_of(id), format!("{id}@{{AAA}}"), "{id} belongs to one card");
        }
        for id in ["net_throttling", "fast_dns", "tcp_autotuning"] {
            assert_eq!(key_of(id), id, "{id} is machine-wide and keeps its old snapshot key");
        }
    }

    #[test]
    fn a_tweak_falls_back_to_its_id_when_no_adapter_is_known() {
        // Nothing to scope by: an id@ with an empty guid would be a third key
        // space that matches neither the old entries nor the new ones.
        assert_eq!(per_adapter_key("adapter_power", &NetState::default()), "adapter_power");
    }

    /// What one pass of `refresh_tweaks` costs, tweak by tweak.
    ///
    /// Ignored by default: it shells out to `netsh` and `powercfg` and its
    /// output is a measurement, not an assertion. Run it with
    /// `cargo test --release -- --ignored read_cost --nocapture` before and
    /// after touching anything on this path.
    #[cfg(windows)]
    #[test]
    #[ignore = "measures the machine, not the code"]
    fn read_cost_per_tweak() {
        let net = NetState::default();
        let mut total = std::time::Duration::ZERO;
        let mut rows: Vec<(std::time::Duration, &'static str)> = Vec::new();
        for t in all() {
            let at = std::time::Instant::now();
            let _ = t.read(&net);
            let took = at.elapsed();
            total += took;
            rows.push((took, t.id()));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        for (took, id) in &rows {
            println!("{id:<20} {:>8.1} ms", took.as_secs_f64() * 1000.0);
        }
        println!("refresh_tweaks total: {:.1} ms", total.as_secs_f64() * 1000.0);
    }

    /// Tool names reached through a variable rather than a literal, so the
    /// scanner below cannot see them.
    ///
    /// ponytail: a hand-kept list. `StackReset::apply` iterates a `[(&str,
    /// &[&str]); 5]` table; if a third such table shows up, register the
    /// tools through a macro instead of adding a line here.
    const DYNAMIC_TOOLS: [&str; 2] = ["ipconfig", "netsh"];

    /// Every literal handed to `run`, gathered by scanning the tree.
    fn tools_named_in_source() -> std::collections::BTreeSet<String> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }

        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);

        let mut found: std::collections::BTreeSet<String> =
            DYNAMIC_TOOLS.iter().map(|s| (*s).to_string()).collect();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            for (at, _) in text.match_indices("run(") {
                // Two things this must not read as a call site: a longer
                // identifier ending in `run`, and the `"run("` literal this
                // very scanner is written with. Neither can be preceded by a
                // word character or a quote; a real call site is preceded by
                // `:`, `.`, whitespace or the start of a line.
                let before = text[..at].chars().next_back();
                if before.is_some_and(|c| c == '"' || c.is_alphanumeric() || c == '_') {
                    continue;
                }
                // `fn run(`, `bandwidth::run(` and friends name no console
                // tool; they are skipped because what follows is not a quote.
                let rest = text[at + 4..].trim_start();
                let Some(body) = rest.strip_prefix('"') else { continue };
                let Some(end) = body.find('"') else { continue };
                found.insert(body[..end].to_string());
            }
        }
        found
    }

    #[test]
    fn every_console_tool_we_name_resolves_to_a_real_file() {
        let found = tools_named_in_source();

        // A scanner that quietly matches nothing would pass every assertion
        // below, so it has to prove it read the tree first.
        for expected in ["wevtutil", "netsh", "ipconfig", "powercfg"] {
            assert!(found.contains(expected), "the scanner missed {expected}: {found:?}");
        }

        for name in &found {
            let path = system_tool(name);
            #[cfg(windows)]
            assert!(
                path.exists(),
                "run({name:?}) resolves to {}, which does not exist",
                path.display()
            );
            #[cfg(not(windows))]
            assert_eq!(path, std::path::PathBuf::from(name));
        }
    }
}
