//! Game mode: notices a game running, tells the overlay whose side a ping
//! spike is on, and, when asked, clears this machine's own traffic out of the
//! way for the length of the game.
//!
//! What it can reach is this computer and nothing else. It cannot make the
//! router, the other devices in the house or the provider's route favour a
//! game, and it does not pretend to: the overlay says where a spike is, and
//! the only changes it makes are ones on this machine that are undone when
//! the game ends.

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::monitor::Shared;
use crate::optimize;
use crate::probe::netstate::NetState;
use crate::store::Store;

/// The games watched for, by process name, with the name shown for them.
///
/// The in-match processes, not the launchers: the overlay has nothing to say
/// in a lobby. Hearthstone is turn-based and was first left out, but its
/// player wanted the overlay there too; it has no separate match process, so
/// it counts from the moment it starts.
pub const GAMES: [(&str, &str); 4] = [
    ("League of Legends.exe", "League of Legends"),
    ("cs2.exe", "Counter-Strike 2"),
    ("VALORANT-Win64-Shipping.exe", "VALORANT"),
    ("Hearthstone.exe", "Hearthstone"),
];

/// The launchers and clients that stay up between matches. A session lasts
/// while any of these or a game is running: League of Legends closes its
/// game process after every match, and a session undone each time would have
/// to be asked for again before every game.
const CLIENTS: [&str; 4] =
    ["LeagueClient.exe", "LeagueClientUx.exe", "RiotClientServices.exe", "VALORANT.exe"];

/// How long nothing from the game family has to be running before a session
/// ends: the gap between a match closing and the client's next screen, and a
/// client restarting to update, are both shorter.
const SESSION_GRACE: Duration = Duration::from_secs(90);

/// How often the process list is read.
const WATCH_EVERY: Duration = Duration::from_secs(3);

/// Whether any of `names` is a game or one of its clients.
fn family_running(names: &[String]) -> bool {
    names
        .iter()
        .any(|n| game_name(n).is_some() || CLIENTS.iter().any(|c| c.eq_ignore_ascii_case(n)))
}

/// Whether an active session should end now. `quiet_for` is how long nothing
/// from the game family has been running, `None` while something is. A
/// session found at start with nothing running is one a crash left behind,
/// and ends at once.
fn session_over(first_pass: bool, quiet_for: Option<Duration>) -> bool {
    match quiet_for {
        None => false,
        Some(_) if first_pass => true,
        Some(q) => q >= SESSION_GRACE,
    }
}

/// Whether `exe` is one of [`GAMES`]. Windows file names ignore case.
#[cfg(test)]
pub fn is_game(exe: &str) -> bool {
    game_name(exe).is_some()
}

fn game_name(exe: &str) -> Option<&'static str> {
    GAMES.iter().find(|(file, _)| file.eq_ignore_ascii_case(exe)).map(|(_, name)| *name)
}

/// The first running game from [`GAMES`] among `names`.
fn game_in(names: &[String]) -> Option<&'static str> {
    names.iter().find_map(|exe| game_name(exe))
}

/// Every running process's executable name. Empty when the list cannot be
/// read, which reads as "no game" rather than as an error: game mode is a
/// convenience, and nothing else depends on it.
#[cfg(windows)]
pub fn process_names() -> Vec<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut out = Vec::new();
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return out;
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut more = unsafe { Process32FirstW(snap, &mut entry) }.is_ok();
    while more {
        let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
        out.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
        more = unsafe { Process32NextW(snap, &mut entry) }.is_ok();
    }
    unsafe {
        let _ = CloseHandle(snap);
    }
    out
}

#[cfg(not(windows))]
pub fn process_names() -> Vec<String> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Whose side a spike is on
// ---------------------------------------------------------------------------

/// One sweep as the overlay keeps it: the router's and the fastest public
/// target's reply, `None` for a reply that did not come.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub router: Option<f64>,
    pub internet: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blame {
    /// No spike in the latest sweeps.
    Clean,
    /// The router spiked with the internet: the Wi-Fi, the cable or the
    /// router, on this side.
    Local,
    /// The internet spiked while the router did not: the provider or beyond.
    Beyond,
    /// Too few sweeps to tell.
    Unknown,
}

/// Fewest sweeps a verdict is read from.
const BLAME_MIN: usize = 10;
/// The newest sweeps a spike is looked for in.
const BLAME_RECENT: usize = 3;
/// How far back, in seconds, the sweeps a spike is measured against reach.
/// In seconds rather than sweeps, so the overlay's faster cadence during a
/// game and the monitor's usual one judge the same stretch of time.
pub const BLAME_SPAN_S: f64 = 120.0;
/// Where in the older sweeps' replies the baseline sits: low, so a lag that
/// fills most of the span is still measured against the calm part of it.
/// The median did not do that: a lag held for half the window became the
/// median and was then called stable.
// ponytail: a lag that outlasts ~80% of the span becomes the baseline and
// reads as clean again; the first line's absolute ping still shows it. A
// baseline kept across the whole game would be the upgrade.
const BASELINE_AT: f64 = 0.2;

/// Reads the spike, if any, off the window of sweeps, oldest first.
///
/// A spike is a reply in the latest few sweeps well above the baseline of
/// the older ones, or no reply at all. It is on this side when the router
/// spiked in the same sweeps: every packet crosses the Wi-Fi and the router
/// first, so a delay there shows on both, and a delay past them shows only
/// on the internet.
pub fn blame(window: &[Reading]) -> Blame {
    if window.len() < BLAME_MIN {
        return Blame::Unknown;
    }
    blame_with(window, &TUNING)
}

/// The thresholds `blame` reads a spike with.
#[derive(Debug, Clone, Copy)]
struct Tuning {
    /// Milliseconds above the window's median an internet reply has to be.
    net_floor_ms: f64,
    /// Or this share of the median, whichever is more.
    net_ratio: f64,
    /// The same for the router, which is one hop and much steadier.
    router_floor_ms: f64,
    /// Of the newest [`BLAME_RECENT`] sweeps, how many have to be over.
    need: usize,
}

/// Chosen on a real week of this machine's sweeps (the ignored
/// `blame_on_recorded_history` test prints the comparison). The first guess,
/// +30 ms on any one sweep, would have spoken 27 times an hour on a line
/// that was sound 97% of the time: nobody trusts a warning every two
/// minutes. +50 ms held for two of the last three sweeps spoke 3.5 times an
/// hour, and that is a delay a player feels, not one lost ping. Against the
/// low baseline over [`BLAME_SPAN_S`] it speaks 6.4 times an hour on 118,942
/// recorded sweeps, where the median over 20 sweeps spoke 5.0 but called 29% of the
/// sweeps inside a lag over 100 ms stable. +80 ms would be 3.6 an hour, and
/// would let a spike from 25 to 90 ms, which a player feels, pass unsaid.
const TUNING: Tuning =
    Tuning { net_floor_ms: 50.0, net_ratio: 0.5, router_floor_ms: 15.0, need: 2 };

fn blame_with(window: &[Reading], t: &Tuning) -> Blame {
    let recent = &window[window.len() - BLAME_RECENT..];
    if !spiked(window, recent, |r| r.internet, t.net_floor_ms, t.net_ratio, t.need) {
        return Blame::Clean;
    }
    if spiked(window, recent, |r| r.router, t.router_floor_ms, 1.0, t.need) {
        Blame::Local
    } else {
        Blame::Beyond
    }
}

/// Whether at least `need` of the `recent` sweeps are well above the
/// baseline of the sweeps before them, a lost reply counting as above.
fn spiked(
    window: &[Reading],
    recent: &[Reading],
    pick: impl Fn(&Reading) -> Option<f64>,
    floor_ms: f64,
    ratio: f64,
    need: usize,
) -> bool {
    let older = &window[..window.len() - recent.len()];
    let mut all: Vec<f64> = older.iter().filter_map(&pick).collect();
    if all.is_empty() {
        return false;
    }
    all.sort_by(|a, b| a.total_cmp(b));
    let base = all[(all.len() as f64 * BASELINE_AT) as usize];
    let limit = base + floor_ms.max(base * ratio);
    recent.iter().filter(|r| pick(r).is_none_or(|v| v > limit)).count() >= need
}

// ---------------------------------------------------------------------------
// "Prepare the connection": changes for the length of one game
// ---------------------------------------------------------------------------

/// Tweaks worth having for a game and effective at once, without a restart.
const SESSION_TWEAKS: [&str; 1] = ["wlan_power_plan"];
/// Services that download in the background: Windows Update, the transfer
/// service it and the Store use, and Delivery Optimization.
const SESSION_SERVICES: [&str; 3] = ["wuauserv", "BITS", "DoSvc"];

/// What one game session changed, and so what ending it must undo. Written
/// to disk after every change, so a crash mid-game still leaves the list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub tweaks: Vec<String>,
    pub services: Vec<String>,
}

impl Session {
    fn is_empty(&self) -> bool {
        self.tweaks.is_empty() && self.services.is_empty()
    }
}

/// What a session would change: tweaks that are worth changing, and services
/// that are running. A tweak already set, or one that could not be read, and
/// a service already stopped, are not the session's to undo later.
pub fn plan(tweaks: &[(&str, Option<bool>)], services: &[(&str, bool)]) -> Session {
    Session {
        tweaks: tweaks
            .iter()
            .filter(|(_, optimal)| *optimal == Some(false))
            .map(|(id, _)| id.to_string())
            .collect(),
        services: services
            .iter()
            .filter(|(_, running)| *running)
            .map(|(name, _)| name.to_string())
            .collect(),
    }
}

fn session_path() -> std::path::PathBuf {
    crate::settings::data_dir().join("game_session.json")
}

/// Whether a session's changes are in place.
pub fn session_active() -> bool {
    session_path().exists()
}

fn read_session(path: &std::path::Path) -> Option<Session> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Written beside and renamed over, like the tweak snapshots: a crash
/// mid-write loses the newest entry, never the list.
fn save_session(path: &std::path::Path, s: &Session) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The store the watcher was started with, for logging what a session
/// changed: the outage analysis suspects a recent change first.
// ponytail: a process-wide handle rather than threading the store through
// the tray; pass it explicitly if a second caller ever needs another store.
static STORE: OnceLock<Arc<Store>> = OnceLock::new();

fn log(tweak_id: &str, action: &str, result: &str) {
    if let Some(store) = STORE.get() {
        store.log_tweak(tweak_id, action, "game", result);
    }
}

/// Makes the session's changes and records each one as it lands. Needs
/// administrator rights, which the caller is expected to have checked.
pub fn prepare(net: &NetState) -> Result<Session> {
    if !optimize::is_elevated() {
        return Err(anyhow!(crate::i18n::game_needs_admin()));
    }
    if session_active() {
        return Err(anyhow!(crate::i18n::game_already()));
    }
    let tweaks = optimize::all();
    let states: Vec<(&str, Option<bool>)> = SESSION_TWEAKS
        .iter()
        .filter_map(|id| tweaks.iter().find(|t| t.id() == *id))
        .map(|t| (t.id(), t.read(net).optimal))
        .collect();
    let running: Vec<(&str, bool)> =
        SESSION_SERVICES.iter().map(|name| (*name, service::running(name))).collect();
    let wanted = plan(&states, &running);

    let path = session_path();
    let mut done = Session::default();
    save_session(&path, &done)?;
    for id in &wanted.tweaks {
        let Some(t) = tweaks.iter().find(|t| t.id() == id) else { continue };
        match optimize::apply(t.as_ref(), net) {
            Ok(msg) => {
                log(id, "apply", &msg);
                done.tweaks.push(id.clone());
                save_session(&path, &done)?;
            }
            Err(e) => log(id, "apply_failed", &e.to_string()),
        }
    }
    for name in &wanted.services {
        if service::stop(name).is_ok() {
            done.services.push(name.clone());
            save_session(&path, &done)?;
        }
    }
    if done.is_empty() {
        let _ = std::fs::remove_file(&path);
    }
    Ok(done)
}

/// Undoes what the recorded session changed. What could not be undone stays
/// on the list, so a later start (with rights) tries again.
pub fn restore(net: &NetState) -> Result<()> {
    let path = session_path();
    let Some(session) = read_session(&path).filter(|s| !s.is_empty()) else {
        // Nothing recorded to undo, which needs no rights to clear.
        let _ = std::fs::remove_file(&path);
        return Ok(());
    };
    if !optimize::is_elevated() {
        return Err(anyhow!(crate::i18n::game_needs_admin()));
    }
    let tweaks = optimize::all();
    let mut left = Session::default();
    for id in &session.tweaks {
        let reverted = tweaks
            .iter()
            .find(|t| t.id() == id)
            .map(|t| optimize::revert(t.as_ref(), net))
            .unwrap_or_else(|| Err(anyhow!("unknown tweak {id}")));
        match reverted {
            Ok(msg) => log(id, "revert", &msg),
            Err(e) => {
                log(id, "revert_failed", &e.to_string());
                left.tweaks.push(id.clone());
            }
        }
    }
    for name in &session.services {
        if service::start(name).is_err() {
            left.services.push(name.clone());
        }
    }
    if left.is_empty() {
        let _ = std::fs::remove_file(&path);
        Ok(())
    } else {
        save_session(&path, &left)?;
        Err(anyhow!(crate::i18n::game_restore_partial()))
    }
}

/// Starts the watcher: reads the process list every few seconds, publishes
/// the running game, and ends a session when its game closes. A session left
/// behind by a crash, with no game running, is ended on the first pass.
pub fn start_watcher(shared: Arc<Shared>, store: Arc<Store>) {
    let _ = STORE.set(store);
    let spawned = std::thread::Builder::new().name("netdoctor-game".into()).spawn(move || {
        let mut first = true;
        let mut quiet_since: Option<std::time::Instant> = None;
        loop {
            let names = process_names();
            let game = game_in(&names);
            shared.gaming.store(game.is_some(), Ordering::Relaxed);
            *shared.game.lock().unwrap_or_else(|p| p.into_inner()) = game;

            if family_running(&names) {
                quiet_since = None;
            } else if quiet_since.is_none() {
                quiet_since = Some(std::time::Instant::now());
            }

            if session_active() {
                let quiet_for = quiet_since.map(|t| t.elapsed());
                if session_over(first, quiet_for) {
                    let net = shared.last.lock().unwrap_or_else(|p| p.into_inner()).net.clone();
                    let msg = match restore(&net) {
                        Ok(()) => crate::i18n::game_restored().to_string(),
                        Err(e) => e.to_string(),
                    };
                    *shared.game_msg.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
                } else {
                    keep_stopped();
                }
            }
            first = false;
            std::thread::sleep(WATCH_EVERY);
        }
    });
    // ponytail: no watcher means no game mode, and the rest of the app runs
    // as before; nothing to report.
    let _ = spawned;
}

/// Stops again any service the session stopped that has come back. Windows
/// starts Windows Update and BITS on its own triggers, and a session that
/// stopped them once and then looked away would claim a quiet link it no
/// longer has.
fn keep_stopped() {
    if !optimize::is_elevated() {
        return;
    }
    let Some(session) = read_session(&session_path()) else { return };
    for name in restart_needed(&session.services, service::running) {
        let _ = service::stop(&name);
    }
}

/// The session's services that are running again.
fn restart_needed(services: &[String], running: impl Fn(&str) -> bool) -> Vec<String> {
    services.iter().filter(|s| running(s)).cloned().collect()
}

/// Runs [`prepare`] off the calling thread and leaves the outcome for the
/// tray to show. The tray's own thread must not wait on service control.
pub fn prepare_in_background(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("netdoctor-game-prep".into()).spawn(move || {
        let net = shared.last.lock().unwrap_or_else(|p| p.into_inner()).net.clone();
        let msg = match prepare(&net) {
            Ok(s) if s.is_empty() => crate::i18n::game_nothing_to_do().to_string(),
            Ok(s) => crate::i18n::game_prepared(s.tweaks.len() + s.services.len()),
            Err(e) => e.to_string(),
        };
        *shared.game_msg.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
    });
}

/// And [`restore`], for the tray's "end game mode".
pub fn restore_in_background(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("netdoctor-game-end".into()).spawn(move || {
        let net = shared.last.lock().unwrap_or_else(|p| p.into_inner()).net.clone();
        let msg = match restore(&net) {
            Ok(()) => crate::i18n::game_restored().to_string(),
            Err(e) => e.to_string(),
        };
        *shared.game_msg.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
    });
}

/// Windows services, through the service control manager rather than
/// `sc.exe`, whose output is translated.
mod service {
    use anyhow::Result;

    #[cfg(windows)]
    fn with_service<T>(
        name: &str,
        access: u32,
        f: impl FnOnce(windows::Win32::System::Services::SC_HANDLE) -> Result<T>,
    ) -> Result<T> {
        use windows::core::PCWSTR;
        use windows::Win32::System::Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT,
        };
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)?;
            let svc = OpenServiceW(scm, PCWSTR(wide.as_ptr()), access);
            let out = match svc {
                Ok(h) => {
                    let r = f(h);
                    let _ = CloseServiceHandle(h);
                    r
                }
                Err(e) => Err(e.into()),
            };
            let _ = CloseServiceHandle(scm);
            out
        }
    }

    #[cfg(windows)]
    pub fn running(name: &str) -> bool {
        use windows::Win32::System::Services::{
            QueryServiceStatus, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS,
        };
        with_service(name, SERVICE_QUERY_STATUS, |h| {
            let mut st = SERVICE_STATUS::default();
            unsafe { QueryServiceStatus(h, &mut st) }?;
            Ok(st.dwCurrentState == SERVICE_RUNNING)
        })
        .unwrap_or(false)
    }

    #[cfg(windows)]
    pub fn stop(name: &str) -> Result<()> {
        use windows::Win32::System::Services::{
            ControlService, SERVICE_CONTROL_STOP, SERVICE_STATUS, SERVICE_STOP,
        };
        with_service(name, SERVICE_STOP, |h| {
            let mut st = SERVICE_STATUS::default();
            unsafe { ControlService(h, SERVICE_CONTROL_STOP, &mut st) }?;
            Ok(())
        })
    }

    /// Starting a service that is already running (Windows restarts some of
    /// these on its own) is success: the state wanted is "running".
    #[cfg(windows)]
    pub fn start(name: &str) -> Result<()> {
        use windows::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;
        use windows::Win32::System::Services::{StartServiceW, SERVICE_START};
        with_service(name, SERVICE_START, |h| match unsafe { StartServiceW(h, None) } {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERROR_SERVICE_ALREADY_RUNNING.to_hresult() => Ok(()),
            Err(e) => Err(e.into()),
        })
    }

    #[cfg(not(windows))]
    pub fn running(_name: &str) -> bool {
        false
    }
    #[cfg(not(windows))]
    pub fn stop(_name: &str) -> Result<()> {
        Ok(())
    }
    #[cfg(not(windows))]
    pub fn start(_name: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_are_matched_by_process_name_whatever_the_case() {
        assert!(is_game("league of legends.exe"));
        assert!(is_game("CS2.EXE"));
        assert!(is_game("VALORANT-Win64-Shipping.exe"));
        assert!(is_game("Hearthstone.exe"));
        assert!(!is_game("chrome.exe"), "a browser is not a game");
        assert!(!is_game("LeagueClient.exe"), "the lobby, not the match");
    }

    fn steady(n: usize, router: f64, internet: f64) -> Vec<Reading> {
        vec![Reading { router: Some(router), internet: Some(internet) }; n]
    }

    #[test]
    fn a_spike_is_placed_on_the_side_it_happened() {
        assert_eq!(blame(&steady(9, 3.0, 25.0)), Blame::Unknown, "too few sweeps");
        assert_eq!(blame(&steady(20, 3.0, 25.0)), Blame::Clean);

        let held = |r: Reading| {
            let mut w = steady(20, 3.0, 25.0);
            w.extend([r, r]);
            w
        };
        // Router and internet spike together, and hold: this side.
        let both = Reading { router: Some(80.0), internet: Some(110.0) };
        assert_eq!(blame(&held(both)), Blame::Local);

        // Only the internet: past the router.
        let far = Reading { router: Some(3.5), internet: Some(140.0) };
        assert_eq!(blame(&held(far)), Blame::Beyond);

        // Lost replies count as a spike, on whichever side lost them.
        assert_eq!(blame(&held(Reading { router: None, internet: None })), Blame::Local);

        // One sweep alone is not a spike: one lost or slow ping is noise.
        let mut once = steady(20, 3.0, 25.0);
        once.push(far);
        assert_eq!(blame(&once), Blame::Clean);

        // Below +50 ms is not a spike either.
        assert_eq!(blame(&held(Reading { router: Some(4.0), internet: Some(60.0) })), Blame::Clean);
    }

    #[test]
    fn a_lag_that_fills_most_of_the_window_is_still_a_lag() {
        // Half a minute calm, then a lag held longer than the calm part: the
        // median of the window is the lag by now, and it is still one.
        let far = Reading { router: Some(3.5), internet: Some(140.0) };
        let mut w = steady(30, 3.0, 25.0);
        w.extend(vec![far; 60]);
        assert_eq!(blame(&w), Blame::Beyond);
    }

    #[test]
    fn a_session_outlives_the_gap_between_matches() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // In the LoL lobby between matches: the game process is gone, the
        // client is not, and the session stays.
        assert!(family_running(&names(&["explorer.exe", "LeagueClientUx.exe"])));
        assert!(!family_running(&names(&["explorer.exe", "steam.exe"])));

        assert!(!session_over(false, None), "something from the game is running");
        assert!(!session_over(false, Some(Duration::from_secs(30))), "a client restarting");
        assert!(session_over(false, Some(SESSION_GRACE)));
        assert!(session_over(true, Some(Duration::ZERO)), "left behind by a crash");
        assert!(!session_over(true, None), "restarted mid-game: carry on");
    }

    #[test]
    fn services_windows_restarted_are_stopped_again() {
        let session = vec!["wuauserv".to_string(), "BITS".to_string()];
        let again = restart_needed(&session, |name| name == "wuauserv");
        assert_eq!(again, ["wuauserv"]);
    }

    #[test]
    fn a_session_undoes_only_what_it_changed() {
        let s = plan(
            &[("wlan_power_plan", Some(false)), ("already_set", Some(true)), ("unread", None)],
            &[("wuauserv", true), ("BITS", false), ("DoSvc", true)],
        );
        assert_eq!(s.tweaks, ["wlan_power_plan"]);
        assert_eq!(s.services, ["wuauserv", "DoSvc"]);
    }

    #[test]
    fn the_session_record_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("netdoctor-game-{}", std::process::id()));
        let path = dir.join("game_session.json");
        let s = Session { tweaks: vec!["wlan_power_plan".into()], services: vec!["BITS".into()] };
        save_session(&path, &s).unwrap();
        assert_eq!(read_session(&path), Some(s));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs `blame` over this machine's recorded sweeps, the way the overlay
    /// would have, and prints how often it would have spoken. The thresholds
    /// are judged here, on a real line, not on made-up numbers.
    #[test]
    #[ignore = "reads the live history database"]
    fn blame_on_recorded_history() {
        let store = Store::open_default().expect("history database");
        let to = crate::store::now();
        let rows = store.samples_between(to - 7.0 * 86400.0, to);
        // One sweep is every row sharing its timestamp.
        let mut sweeps: std::collections::BTreeMap<i64, Reading> = Default::default();
        for (ts, key, rtt, ok) in rows {
            let r = sweeps
                .entry((ts * 1000.0) as i64)
                .or_insert(Reading { router: None, internet: None });
            let v = if ok { rtt } else { None };
            match key.as_str() {
                "gateway" => r.router = v,
                "cloudflare" | "google" => {
                    r.internet = match (r.internet, v) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    }
                }
                _ => {}
            }
        }
        let seq: Vec<(i64, Reading)> = sweeps.into_iter().collect();
        let mut tunings = vec![TUNING];
        for net in [30.0, 50.0, 80.0, 120.0] {
            for need in [1, 2] {
                tunings.push(Tuning {
                    net_floor_ms: net,
                    net_ratio: 0.5,
                    router_floor_ms: 15.0,
                    need,
                });
            }
        }
        println!("sweeps {}", seq.len());
        for t in tunings {
            let (mut judged, mut local, mut beyond, mut alerts) = (0usize, 0usize, 0usize, 0usize);
            // Sweeps in a lag a player feels (two of the last three internet
            // replies over 100 ms) that were still called clean.
            let (mut lagging, mut missed) = (0usize, 0usize);
            let mut prev = Blame::Clean;
            let mut start = 0;
            for i in 0..seq.len() {
                // The overlay's window: the sweeps of the last span.
                while seq[i].0 - seq[start].0 > (BLAME_SPAN_S * 1000.0) as i64 {
                    start += 1;
                }
                // Only an unbroken run of sweeps is a window: a gap is the
                // app closed or asleep, not a spike.
                let window = &seq[start..=i];
                if window.len() < BLAME_MIN || !window.windows(2).all(|w| w[1].0 - w[0].0 <= 3000) {
                    continue;
                }
                let readings: Vec<Reading> = window.iter().map(|(_, r)| *r).collect();
                let b = blame_with(&readings, &t);
                judged += 1;
                let recent = &readings[readings.len() - BLAME_RECENT..];
                if recent.iter().filter(|r| r.internet.is_some_and(|v| v > 100.0)).count() >= 2 {
                    lagging += 1;
                    if b == Blame::Clean {
                        missed += 1;
                    }
                }
                match b {
                    Blame::Local => local += 1,
                    Blame::Beyond => beyond += 1,
                    _ => {}
                }
                let loud = matches!(b, Blame::Local | Blame::Beyond);
                if loud && !matches!(prev, Blame::Local | Blame::Beyond) {
                    alerts += 1;
                }
                prev = b;
            }
            let hours = judged as f64 / 3600.0;
            println!(
                "net +{:>3} ms need {}: local {:.2}% beyond {:.2}% alerts/h {:.1} \
                 clean in lag {:.0}% of {}",
                t.net_floor_ms,
                t.need,
                local as f64 * 100.0 / judged.max(1) as f64,
                beyond as f64 * 100.0 / judged.max(1) as f64,
                alerts as f64 / hours.max(1e-9),
                missed as f64 * 100.0 / lagging.max(1) as f64,
                lagging
            );
        }
    }

    #[test]
    #[ignore = "watches the live machine"]
    fn finds_a_running_process_by_name() {
        // Explorer rather than a process started for the test: starting one
        // opens a window on whoever's screen this runs on.
        let names = process_names();
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("explorer.exe")),
            "{} processes",
            names.len()
        );
    }
}
