//! User settings, persisted as JSON next to the database.
//!
//! Unknown keys in an existing file are dropped and missing ones fall back to
//! the default, so a settings file written by an older build keeps working.

use std::net::Ipv4Addr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::{self, Lang};

pub const APP_NAME: &str = "NetDoctor";
pub const DNS_TEST_HOST: &str = "example.com";

/// The longest interval between sweeps that still lets an outage open. A gap
/// wider than [`crate::store::OBSERVATION_GAP_S`] is read as a machine that
/// slept, which resets the failure streak, so at that interval every sweep
/// would start the count again. Half of it leaves room for a sweep that runs
/// late by its own timeout and an adapter read.
pub const MAX_PROBE_INTERVAL_MS: u64 = (crate::store::OBSERVATION_GAP_S * 1000.0 / 2.0) as u64;

/// Where a probe sits in the chain, which is what lets us blame the right
/// party when it stops answering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Lan,
    Isp,
    Internet,
}

/// A corner of the screen for the game overlay. No corner is free in every
/// game (CS2 and VALORANT keep the minimap top-left, the kill feed
/// top-right), so the player picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 4] =
        [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => i18n::corner_top_left(),
            Corner::TopRight => i18n::corner_top_right(),
            Corner::BottomLeft => i18n::corner_bottom_left(),
            Corner::BottomRight => i18n::corner_bottom_right(),
        }
    }
}

/// The least opaque the overlay may be: below this it cannot be read.
pub const OVERLAY_OPACITY_MIN: u8 = 20;

/// How big the overlay's text is; the window grows with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverlaySize {
    Small,
    Medium,
    Large,
}

impl OverlaySize {
    pub const ALL: [OverlaySize; 3] = [OverlaySize::Small, OverlaySize::Medium, OverlaySize::Large];

    /// The font's height in pixels.
    pub fn font_px(self) -> i32 {
        match self {
            OverlaySize::Small => 12,
            OverlaySize::Medium => 15,
            OverlaySize::Large => 20,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OverlaySize::Small => i18n::size_small(),
            OverlaySize::Medium => i18n::size_medium(),
            OverlaySize::Large => i18n::size_large(),
        }
    }
}

/// What the overlay shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverlayContent {
    /// The router's and the internet's ping, and whose side a spike is on.
    Full,
    /// Only the router's and the internet's ping.
    Ping,
    /// Only the internet's ping.
    Internet,
}

impl OverlayContent {
    pub const ALL: [OverlayContent; 3] =
        [OverlayContent::Full, OverlayContent::Ping, OverlayContent::Internet];

    pub fn label(self) -> &'static str {
        match self {
            OverlayContent::Full => i18n::content_full(),
            OverlayContent::Ping => i18n::content_ping(),
            OverlayContent::Internet => i18n::content_internet(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub key: String,
    pub label: String,
    /// `None` means "resolve at runtime" (the router, the ISP resolver).
    pub host: Option<Ipv4Addr>,
    pub scope: Scope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub probe_interval_ms: u64,
    pub ping_timeout_ms: u32,
    pub outage_after_fails: u32,
    pub extra_targets: Vec<String>,

    pub ping_good_ms: f64,
    pub ping_ok_ms: f64,
    pub ping_bad_ms: f64,
    pub jitter_good_ms: f64,
    pub jitter_ok_ms: f64,
    pub loss_good_pct: f64,
    pub loss_ok_pct: f64,

    pub notify_on_outage: bool,
    pub start_minimised: bool,
    pub keep_days: i64,

    /// Ask GitHub once per launch whether there is a newer release. Off means
    /// the app never reaches out on its own; the check is still available by
    /// hand in the settings tab.
    pub check_updates: bool,

    /// Show the ping overlay while a game is running.
    pub game_overlay: bool,
    /// Which corner of the game's screen it sits in.
    pub game_overlay_corner: Corner,
    /// How opaque it is, in percent; clamped to [`OVERLAY_OPACITY_MIN`]..=100
    /// where it is used, so a hand-edited 0 cannot make it invisible.
    pub overlay_opacity: u8,
    pub overlay_size: OverlaySize,
    pub overlay_content: OverlayContent,

    /// The user's own OpenRouter key for the optional AI explanation. Empty
    /// means the feature is off.
    // ponytail: stored in plain text in settings.json under the user's own
    // profile, like any per-user CLI config. Wrap it with DPAPI
    // (CryptProtectData) if the file ever needs to be shared or synced.
    pub ai_key: String,
    /// Which OpenRouter model answers. Empty means `ai::DEFAULT_MODEL`.
    pub ai_model: String,

    /// `None` until the user picks one, which lets the first run follow the
    /// Windows UI language without freezing that choice in the file.
    pub lang: Option<Lang>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            probe_interval_ms: 1000,
            ping_timeout_ms: 1000,
            outage_after_fails: 3,
            extra_targets: Vec::new(),

            ping_good_ms: 30.0,
            ping_ok_ms: 60.0,
            ping_bad_ms: 120.0,
            jitter_good_ms: 5.0,
            jitter_ok_ms: 15.0,
            loss_good_pct: 0.5,
            loss_ok_pct: 2.0,

            notify_on_outage: true,
            start_minimised: false,
            keep_days: 14,

            check_updates: true,

            game_overlay: true,
            game_overlay_corner: Corner::TopLeft,
            overlay_opacity: 85,
            overlay_size: OverlaySize::Medium,
            overlay_content: OverlayContent::Full,

            ai_key: String::new(),
            ai_model: String::new(),

            lang: None,
        }
    }
}

pub fn data_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(APP_NAME)
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn db_path() -> PathBuf {
    data_dir().join("history.db")
}

pub fn snapshot_path() -> PathBuf {
    data_dir().join("tweak_snapshots.json")
}

/// Set when `load` had to fall back to defaults because the file was there
/// but unreadable. The UI shows it; without it the user sees every setting
/// reset to default and no reason why.
static LOAD_ISSUE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn load_issue() -> Option<String> {
    LOAD_ISSUE.lock().ok().and_then(|g| g.clone())
}

fn set_load_issue(msg: String) {
    if let Ok(mut g) = LOAD_ISSUE.lock() {
        *g = Some(msg);
    }
}

impl Settings {
    /// A missing file is a first run. A file that exists but does not parse
    /// is a fault: the old contents are moved aside before defaults take over,
    /// so the next `save` cannot destroy the only copy of the user's setup.
    pub fn load() -> Self {
        let (settings, issue) = Self::load_from(&settings_path());
        if let Some(msg) = issue {
            set_load_issue(msg);
        }
        settings
    }

    /// The load, with the path and the outcome in the open so it can be
    /// tested without touching the user's profile.
    fn load_from(path: &std::path::Path) -> (Self, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Settings::default(), None)
            }
            Err(e) => return (Settings::default(), Some(format!("{}: {e}", path.display()))),
        };
        match serde_json::from_str(&text) {
            Ok(s) => (s, None),
            Err(e) => {
                let kept = path.with_extension("json.corrupt");
                let where_ = match std::fs::rename(path, &kept) {
                    Ok(()) => kept.display().to_string(),
                    Err(_) => path.display().to_string(),
                };
                (Settings::default(), Some(format!("{e} ({where_})")))
            }
        }
    }

    /// Temp file plus rename, so an interrupted write cannot leave a
    /// half-truncated settings file behind.
    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(data_dir())?;
        let path = settings_path();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// The language to run in: the user's choice, or what Windows suggests.
    pub fn effective_lang(&self) -> Lang {
        self.lang.unwrap_or_else(i18n::detect)
    }

    /// Held to the range [`Settings::refusal`] allows, for a file edited by
    /// hand: past the ceiling no outage could be recorded at all.
    /// Takes the overlay's settings from `other`, leaving the rest. Returns
    /// whether anything changed.
    pub fn take_overlay(&mut self, other: &Settings) -> bool {
        let fields = |s: &Settings| {
            (
                s.game_overlay,
                s.game_overlay_corner,
                s.overlay_opacity,
                s.overlay_size,
                s.overlay_content,
            )
        };
        let wanted = fields(other);
        if fields(self) == wanted {
            return false;
        }
        (
            self.game_overlay,
            self.game_overlay_corner,
            self.overlay_opacity,
            self.overlay_size,
            self.overlay_content,
        ) = wanted;
        true
    }

    pub fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.probe_interval_ms.clamp(300, MAX_PROBE_INTERVAL_MS))
    }

    /// Why these settings must not be saved, if they must not.
    pub fn refusal(&self) -> Option<String> {
        if self.probe_interval_ms < 300 {
            return Some(i18n::set_err_interval().into());
        }
        if self.probe_interval_ms > MAX_PROBE_INTERVAL_MS {
            return Some(i18n::set_err_interval_max(MAX_PROBE_INTERVAL_MS / 1000));
        }
        if self.ping_ok_ms >= self.ping_bad_ms {
            return Some(i18n::set_err_thresholds().into());
        }
        None
    }

    /// The ping timeout a sweep may actually use: the configured one, but
    /// never so long that the sweep outlasts the interval it belongs to.
    ///
    /// The defaults are a second each, and a probe still waiting when the next
    /// sweep is due has already answered the only question being asked —
    /// nothing came back in time. Letting it run to the full second anyway
    /// just thins out the recording, and during an outage that is exactly when
    /// the samples are worth having. Only the monitor's cadence is capped;
    /// a one-off scan or a traceroute uses the configured value as given.
    pub fn sweep_timeout_ms(&self) -> u32 {
        let ceiling = (self.interval().as_millis() as u64) * 4 / 5;
        self.ping_timeout_ms.min(ceiling.max(100) as u32)
    }

    /// Built-in targets plus whatever the user added. Invalid entries are
    /// skipped rather than failing the whole list.
    ///
    /// Resolves every hostname the user added, so it blocks on DNS. The
    /// monitor must not call it: see [`Settings::targets_with`].
    pub fn targets(&self) -> Vec<Target> {
        self.targets_with(resolve_target)
    }

    /// The same list, with the user's entries resolved by `resolve` instead
    /// of by a live lookup. The monitor passes a cache kept by its own
    /// thread, because a lookup inside the sweep stalls it for seconds
    /// exactly when DNS is what broke.
    pub fn targets_with(&self, resolve: impl Fn(&str) -> Option<Ipv4Addr>) -> Vec<Target> {
        let mut out = vec![
            Target {
                key: "gateway".into(),
                label: i18n::word_router().into(),
                host: None,
                scope: Scope::Lan,
            },
            Target {
                key: "dns_isp".into(),
                label: i18n::word_isp_resolver().into(),
                host: None,
                scope: Scope::Isp,
            },
            Target {
                key: "cloudflare".into(),
                label: "1.1.1.1".into(),
                host: Some(Ipv4Addr::new(1, 1, 1, 1)),
                scope: Scope::Internet,
            },
            Target {
                key: "google".into(),
                label: "8.8.8.8".into(),
                host: Some(Ipv4Addr::new(8, 8, 8, 8)),
                scope: Scope::Internet,
            },
        ];

        for (i, raw) in self.extra_targets.iter().enumerate() {
            let text = raw.trim();
            if text.is_empty() {
                continue;
            }
            if let Some(addr) = resolve(text) {
                out.push(Target {
                    key: format!("custom{i}"),
                    label: text.to_string(),
                    host: Some(addr),
                    scope: Scope::Internet,
                });
            }
        }
        out
    }
}

/// Accepts a literal IPv4 address or a hostname, resolved now (blocking).
pub fn resolve_target(text: &str) -> Option<Ipv4Addr> {
    if let Ok(addr) = text.parse::<Ipv4Addr>() {
        return Some(addr);
    }
    use std::net::ToSocketAddrs;
    (text, 80u16).to_socket_addrs().ok()?.find_map(|sa| match sa.ip() {
        std::net::IpAddr::V4(v4) => Some(v4),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_applies_alone_and_leaves_the_rest_of_the_draft() {
        let mut live = Settings::default();
        let mut draft = Settings::default();
        assert!(!live.take_overlay(&draft), "nothing changed");

        draft.overlay_size = OverlaySize::Large;
        draft.overlay_opacity = 40;
        draft.ping_ok_ms = 999.0;
        assert!(live.take_overlay(&draft));
        assert_eq!((live.overlay_size, live.overlay_opacity), (OverlaySize::Large, 40));
        assert_ne!(live.ping_ok_ms, 999.0, "a threshold waits for Save");
    }

    #[test]
    fn a_sweep_timeout_never_outlasts_its_own_interval() {
        // The defaults are a second each, so an unanswered sweep used to take
        // the whole interval before the next one could start, and the
        // sampling cadence slipped by exactly as much as the outage was worth
        // recording.
        let s = Settings::default();
        assert_eq!(s.probe_interval_ms, 1000);
        assert_eq!(s.ping_timeout_ms, 1000);
        assert!(
            (s.sweep_timeout_ms() as u128) < s.interval().as_millis(),
            "a sweep has to finish inside the interval it belongs to"
        );

        // A timeout shorter than the ceiling is left exactly as configured.
        let quick = Settings { ping_timeout_ms: 200, ..Settings::default() };
        assert_eq!(quick.sweep_timeout_ms(), 200);

        // And the floor holds at the shortest interval the settings allow.
        let fast = Settings { probe_interval_ms: 300, ..Settings::default() };
        assert!(fast.sweep_timeout_ms() >= 100);
        assert!((fast.sweep_timeout_ms() as u128) < fast.interval().as_millis());
    }

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert!(s.ping_good_ms < s.ping_ok_ms);
        assert!(s.ping_ok_ms < s.ping_bad_ms);
        assert!(s.jitter_good_ms < s.jitter_ok_ms);
        assert_eq!(s.targets().len(), 4);
    }

    #[test]
    fn the_language_choice_survives_a_round_trip() {
        let mut s = Settings::default();
        assert_eq!(s.lang, None, "a fresh install must follow Windows, not a hard-coded default");

        s.lang = Some(Lang::Pl);
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.lang, Some(Lang::Pl));
        assert_eq!(back.effective_lang(), Lang::Pl);
    }

    #[test]
    fn a_settings_file_from_an_older_build_still_loads() {
        // Written before `lang` existed: it must default to None so the user
        // gets their Windows language rather than an error or English.
        let json = r#"{"ping_ok_ms": 45.0}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.lang, None);
    }

    #[test]
    fn unknown_keys_are_ignored_and_missing_ones_default() {
        let json = r#"{"ping_ok_ms": 45.0, "obsolete": 1}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.ping_ok_ms, 45.0);
        assert_eq!(s.ping_bad_ms, Settings::default().ping_bad_ms);
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("netdoctor-settings-{name}.json"));
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(p.with_extension("json.corrupt"));
        p
    }

    #[test]
    fn a_missing_settings_file_is_a_first_run_not_a_fault() {
        let (s, issue) = Settings::load_from(&scratch("absent"));
        assert!(issue.is_none());
        assert_eq!(s.keep_days, Settings::default().keep_days);
    }

    #[test]
    fn a_damaged_settings_file_is_kept_aside_and_reported() {
        let path = scratch("damaged");
        std::fs::write(&path, "{ \"keep_days\": 30, ").unwrap();

        let (s, issue) = Settings::load_from(&path);

        assert_eq!(s.keep_days, Settings::default().keep_days, "defaults take over");
        let issue = issue.expect("the user has to be told why their settings reset");
        let kept = path.with_extension("json.corrupt");
        assert!(kept.exists(), "the damaged file is preserved, not overwritten");
        assert!(issue.contains("corrupt"), "the message says where it went: {issue}");
        let _ = std::fs::remove_file(kept);
    }

    #[test]
    fn a_partial_file_loads_without_being_reported_as_damage() {
        let path = scratch("partial");
        // Only one known key: `serde(default)` fills the rest.
        std::fs::write(&path, r#"{"keep_days": 3}"#).unwrap();

        let (s, issue) = Settings::load_from(&path);

        assert!(issue.is_none(), "missing keys are normal, not damage");
        assert_eq!(s.keep_days, 3);
        assert_eq!(s.ping_timeout_ms, Settings::default().ping_timeout_ms);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_interval_that_would_hide_every_outage_is_refused() {
        // A gap between sweeps wider than `OBSERVATION_GAP_S` is read as a
        // machine that slept: it resets the failure streak, so at a minute
        // per sweep no outage could ever open. The setting was accepted.
        let gap_ms = (crate::store::OBSERVATION_GAP_S * 1000.0) as u64;
        for ms in [gap_ms, gap_ms + 1, 10 * gap_ms] {
            let s = Settings { probe_interval_ms: ms, ..Default::default() };
            assert!(s.refusal().is_some(), "{ms} ms was accepted");
        }
        assert!(Settings::default().refusal().is_none());
        let slow = Settings { probe_interval_ms: MAX_PROBE_INTERVAL_MS, ..Default::default() };
        assert!(slow.refusal().is_none(), "the ceiling itself is allowed");
        let over = Settings { probe_interval_ms: MAX_PROBE_INTERVAL_MS * 3, ..Default::default() };
        assert_eq!(
            over.interval().as_millis() as u64,
            MAX_PROBE_INTERVAL_MS,
            "a file edited by hand"
        );
        let ceiling = format!("{} s", MAX_PROBE_INTERVAL_MS / 1000);
        assert!(i18n::set_interval_hint().contains(&ceiling), "the hint names the ceiling");
    }

    #[test]
    fn interval_never_drops_below_the_floor() {
        let s = Settings { probe_interval_ms: 5, ..Default::default() };
        assert_eq!(s.interval(), std::time::Duration::from_millis(300));
    }

    #[test]
    fn literal_addresses_become_targets_and_junk_is_skipped() {
        let s = Settings {
            extra_targets: vec!["8.8.4.4".into(), "   ".into(), "!!!not a host!!!".into()],
            ..Default::default()
        };
        let hosts: Vec<_> = s.targets().iter().filter_map(|t| t.host).collect();
        assert!(hosts.contains(&Ipv4Addr::new(8, 8, 4, 4)));
        assert_eq!(s.targets().len(), 5);
    }
}
