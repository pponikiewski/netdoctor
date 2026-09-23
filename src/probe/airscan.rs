//! What else is on the air, and which channel the router should be asked for.
//!
//! Windows will not let a client change the router's channel — that setting
//! lives in the router, and no API on this side can reach it. What the client
//! *can* do is measure: every access point in range beacons its channel and
//! its signal, so the interference each candidate channel would suffer can be
//! computed rather than guessed. The output is therefore advice with numbers
//! attached, aimed at the router's admin page.
//!
//! The list comes from `WlanGetNetworkBssList`, which returns the scan cache.
//! `rescan()` asks the card for a fresh sweep; it takes a few seconds, so it
//! runs on its own thread and the cache is read again afterwards.

use std::ptr;

use windows::core::GUID;
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_any, WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanGetNetworkBssList,
    WlanOpenHandle, WlanScan, WLAN_BSS_LIST, WLAN_INTERFACE_INFO_LIST,
};

use crate::probe::netstate::NetState;

/// The 2.4 GHz channels worth recommending: the only three that do not
/// overlap each other.
pub const CLEAN_24: [u32; 3] = [1, 6, 11];

/// 5 GHz channels that carry no radar duty. A DFS channel can be perfectly
/// quiet and still drop the network for a minute when the router thinks it
/// heard radar, which is exactly the kind of outage this app exists to
/// explain, so they are never recommended.
pub const NON_DFS_5: [u32; 9] = [36, 40, 44, 48, 149, 153, 157, 161, 165];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    B24,
    B5,
    B6,
}

impl Band {
    pub fn label(&self) -> &'static str {
        match self {
            Band::B24 => "2.4 GHz",
            Band::B5 => "5 GHz",
            Band::B6 => "6 GHz",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ap {
    pub ssid: String,
    pub bssid: String,
    pub channel: u32,
    pub band: Band,
    pub rssi_dbm: i32,
    /// True when this beacon belongs to the network we are connected to —
    /// our own radios are not interference, and must not count against the
    /// channel we are about to recommend to ourselves.
    pub ours: bool,
}

#[derive(Debug, Clone)]
pub struct ChannelLoad {
    pub channel: u32,
    /// Foreign access points overlapping this channel.
    pub aps: usize,
    /// Interference from those access points, summed as power and expressed
    /// in dBm. Lower is quieter; `None` when nothing was heard at all.
    pub noise_dbm: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct AirScan {
    pub aps: Vec<Ap>,
    pub load_24: Vec<ChannelLoad>,
    pub load_5: Vec<ChannelLoad>,
    /// Quietest of channels 1, 6 and 11.
    pub best_24: Option<u32>,
    /// Quietest non-DFS 5 GHz channel.
    pub best_5: Option<u32>,
    /// The channel we are on right now, if any.
    pub current: Option<u32>,
    /// Set when the scan could not run at all (no Wi-Fi card, radio off).
    pub error: Option<String>,
}

impl AirScan {
    /// True when moving the router to `best_24` would buy a worthwhile amount
    /// of quiet — at least 6 dB, which is a quarter of the interfering power.
    pub fn worth_moving_24(&self) -> bool {
        let (Some(best), Some(cur)) = (self.best_24, self.current) else {
            return false;
        };
        if !(1..=14).contains(&cur) || best == cur {
            return false;
        }
        match (noise_of(&self.load_24, best), noise_of(&self.load_24, cur)) {
            (Some(b), Some(c)) => c - b >= 6.0,
            (None, Some(_)) => true,
            _ => false,
        }
    }

    /// True when the router has parked us on a 5 GHz channel that shares its
    /// frequency with weather and military radar. Nothing is wrong with the
    /// channel until the router believes it heard a radar pulse, at which
    /// point it must vacate within ten seconds and stay off for half an hour
    /// — a minute or more of outage with no cause visible from the PC, and a
    /// leading suspect whenever the history shows drops that nothing else
    /// explains.
    pub fn on_dfs(&self) -> bool {
        let Some(cur) = self.current else { return false };
        (36..=177).contains(&cur) && !NON_DFS_5.contains(&cur)
    }

    /// Foreign networks sharing our exact channel.
    pub fn co_channel(&self) -> usize {
        let Some(cur) = self.current else { return 0 };
        self.aps.iter().filter(|a| !a.ours && a.channel == cur).count()
    }
}

fn noise_of(loads: &[ChannelLoad], channel: u32) -> Option<f64> {
    loads.iter().find(|l| l.channel == channel)?.noise_dbm
}

/// Reads the card's scan cache and works out the advice. Returns instantly;
/// the entries may be a few minutes old, which is fine for neighbours that
/// rarely move.
pub fn scan(net: &NetState) -> AirScan {
    let mut out = AirScan { current: net.channel, ..Default::default() };
    match collect(net, false) {
        Ok(aps) => out.aps = aps,
        Err(e) => {
            out.error = Some(e);
            return out;
        }
    }
    // Loudest first: that is the order the list is useful in.
    out.aps.sort_by_key(|a| std::cmp::Reverse(a.rssi_dbm));
    out.load_24 = load_for(&out.aps, &(1..=13).collect::<Vec<_>>(), Band::B24);
    out.load_5 = load_for(&out.aps, &NON_DFS_5, Band::B5);
    out.best_24 = quietest(&out.load_24, &CLEAN_24);
    out.best_5 = quietest(&out.load_5, &NON_DFS_5);
    out
}

/// Asks the card for a fresh sweep, waits for it to finish, then reads the
/// cache. Takes about four seconds — call it off the UI thread.
pub fn rescan(net: &NetState) -> AirScan {
    if let Err(e) = collect(net, true) {
        return AirScan { current: net.channel, error: Some(e), ..Default::default() };
    }
    std::thread::sleep(std::time::Duration::from_millis(4000));
    scan(net)
}

/// Picks the quietest channel out of `candidates`, preferring the earlier one
/// on a tie so the answer is stable between scans.
fn quietest(loads: &[ChannelLoad], candidates: &[u32]) -> Option<u32> {
    candidates
        .iter()
        .filter_map(|c| loads.iter().find(|l| l.channel == *c))
        .min_by(|a, b| {
            let (x, y) = (a.noise_dbm.unwrap_or(-120.0), b.noise_dbm.unwrap_or(-120.0));
            x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|l| l.channel)
}

/// Sums interference as power, not as a count: one neighbour at -45 dBm hurts
/// more than six at -85, and counting rows would say the opposite.
fn load_for(aps: &[Ap], candidates: &[u32], band: Band) -> Vec<ChannelLoad> {
    candidates
        .iter()
        .map(|&channel| {
            let mut power = 0.0f64;
            let mut count = 0usize;
            for ap in aps.iter().filter(|a| !a.ours && a.band == band) {
                let w = overlap(band, channel, ap.channel);
                if w <= 0.0 {
                    continue;
                }
                count += 1;
                power += w * 10f64.powf(ap.rssi_dbm as f64 / 10.0);
            }
            ChannelLoad {
                channel,
                aps: count,
                noise_dbm: (power > 0.0).then(|| 10.0 * power.log10()),
            }
        })
        .collect()
}

/// How much of a transmission on `other` lands on `candidate`, 0.0 to 1.0.
fn overlap(band: Band, candidate: u32, other: u32) -> f64 {
    let d = candidate.abs_diff(other);
    match band {
        // 2.4 GHz channels are 5 MHz apart and 20 MHz wide, so anything
        // within four channels spills into ours, less the further it sits.
        Band::B24 => {
            if d >= 5 {
                0.0
            } else {
                1.0 - d as f64 / 5.0
            }
        }
        // 5 and 6 GHz channels do not overlap at 20 MHz, but most access
        // points there run 80 MHz wide: a neighbour on 36 also sits on 40, 44
        // and 48. The scan does not say how wide each one is, so anything in
        // the same 80 MHz block counts for half, and only the same channel in
        // full. Counting only the paired channel called 44 the quietest under
        // an 80 MHz neighbour on 36.
        // ponytail: assumes 80 MHz for everyone; read the VHT/HE operation
        // element from the beacon's IEs if the recommendation needs to be exact.
        Band::B5 | Band::B6 => {
            if d == 0 {
                1.0
            } else if block_80(band, candidate).is_some()
                && block_80(band, candidate) == block_80(band, other)
            {
                0.5
            } else {
                0.0
            }
        }
    }
}

/// Which 80 MHz block a 5 or 6 GHz channel falls in, for channels that have
/// one: 36-48, 52-64, ... 132-144 and 149-161 on 5 GHz, 1-13, 17-29, ... on 6.
fn block_80(band: Band, channel: u32) -> Option<u32> {
    match (band, channel) {
        (Band::B5, 36..=144) => Some((channel - 36) / 16),
        (Band::B5, 149..=161) => Some(100),
        (Band::B6, 1..=233) => Some((channel - 1) / 16),
        _ => None,
    }
}

/// Maps a centre frequency in kHz onto a channel number and its band.
fn channel_of(khz: u32) -> Option<(u32, Band)> {
    let mhz = khz / 1000;
    match mhz {
        2412..=2472 => Some(((mhz - 2407) / 5, Band::B24)),
        2484 => Some((14, Band::B24)),
        5160..=5885 => Some(((mhz - 5000) / 5, Band::B5)),
        5955..=7115 => Some(((mhz - 5950) / 5, Band::B6)),
        _ => None,
    }
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.0, None);
        }
    }
}

/// Walks the Wi-Fi interfaces, optionally kicking off a sweep, and returns
/// every beacon the card is holding.
fn collect(net: &NetState, trigger_scan: bool) -> Result<Vec<Ap>, String> {
    unsafe {
        let mut version = 0u32;
        let mut raw = HANDLE::default();
        if WlanOpenHandle(2, None, &mut version, &mut raw) != ERROR_SUCCESS.0 {
            return Err(crate::i18n::air_no_service().into());
        }
        let handle = Handle(raw);

        let mut list: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
        if WlanEnumInterfaces(handle.0, None, &mut list) != ERROR_SUCCESS.0 || list.is_null() {
            return Err(crate::i18n::air_no_adapter().into());
        }

        let count = (*list).dwNumberOfItems as usize;
        let mut aps = Vec::new();
        let mut seen_interface = false;

        for i in 0..count {
            let info = (*list).InterfaceInfo.as_ptr().add(i);
            let guid: GUID = (*info).InterfaceGuid;
            seen_interface = true;

            if trigger_scan {
                WlanScan(handle.0, &guid, None, None, None);
                continue;
            }

            let mut bss: *mut WLAN_BSS_LIST = ptr::null_mut();
            let rc = WlanGetNetworkBssList(
                handle.0,
                &guid,
                None,
                dot11_BSS_type_any,
                false,
                None,
                &mut bss,
            );
            if rc != ERROR_SUCCESS.0 || bss.is_null() {
                continue;
            }

            let n = (*bss).dwNumberOfItems as usize;
            for j in 0..n {
                let e = &*(*bss).wlanBssEntries.as_ptr().add(j);
                let Some((channel, band)) = channel_of(e.ulChCenterFrequency) else {
                    continue;
                };
                let len = (e.dot11Ssid.uSSIDLength as usize).min(32);
                let ssid = String::from_utf8_lossy(&e.dot11Ssid.ucSSID[..len]).to_string();
                let m = e.dot11Bssid;
                let bssid = format!(
                    "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    m[0], m[1], m[2], m[3], m[4], m[5]
                );
                let ours = !net.ssid.is_empty() && ssid == net.ssid;
                aps.push(Ap { ssid, bssid, channel, band, rssi_dbm: e.lRssi, ours });
            }
            WlanFreeMemory(bss as *const _);
        }

        WlanFreeMemory(list as *const _);
        if !seen_interface {
            return Err(crate::i18n::air_no_adapter().into());
        }
        Ok(aps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequencies_map_to_the_usual_channels() {
        assert_eq!(channel_of(2_412_000), Some((1, Band::B24)));
        assert_eq!(channel_of(2_437_000), Some((6, Band::B24)));
        assert_eq!(channel_of(2_462_000), Some((11, Band::B24)));
        assert_eq!(channel_of(5_180_000), Some((36, Band::B5)));
        assert_eq!(channel_of(5_745_000), Some((149, Band::B5)));
        assert_eq!(channel_of(6_055_000), Some((21, Band::B6)));
        assert_eq!(channel_of(900_000), None);
    }

    #[test]
    fn a_loud_neighbour_outweighs_several_quiet_ones() {
        let ap = |channel, rssi| Ap {
            ssid: "x".into(),
            bssid: "00:00:00:00:00:00".into(),
            channel,
            band: Band::B24,
            rssi_dbm: rssi,
            ours: false,
        };
        let aps = vec![ap(1, -40), ap(6, -85), ap(6, -85), ap(6, -85)];
        let load = load_for(&aps, &CLEAN_24, Band::B24);
        assert_eq!(quietest(&load, &CLEAN_24), Some(11));
        assert!(noise_of(&load, 1).unwrap() > noise_of(&load, 6).unwrap());
    }

    #[test]
    fn an_80_mhz_neighbour_on_36_is_not_missed_on_44() {
        let ap = |channel| Ap {
            ssid: "x".into(),
            bssid: "00:00:00:00:00:00".into(),
            channel,
            band: Band::B5,
            rssi_dbm: -50,
            ours: false,
        };
        let load = load_for(&[ap(36)], &NON_DFS_5, Band::B5);
        assert!(noise_of(&load, 44).is_some(), "44 shares the 80 MHz block with 36");
        assert!(noise_of(&load, 36).unwrap() > noise_of(&load, 44).unwrap());
        assert_eq!(quietest(&load, &NON_DFS_5), Some(149), "the empty block wins");
        // 165 has no 80 MHz block and 161 is not its neighbour.
        assert_eq!(overlap(Band::B5, 165, 161), 0.0);
    }

    #[test]
    fn our_own_access_points_are_not_interference() {
        let mine = Ap {
            ssid: "home".into(),
            bssid: "00:00:00:00:00:01".into(),
            channel: 1,
            band: Band::B24,
            rssi_dbm: -30,
            ours: true,
        };
        let load = load_for(&[mine], &CLEAN_24, Band::B24);
        assert!(load.iter().all(|l| l.aps == 0 && l.noise_dbm.is_none()));
    }

    #[test]
    fn reading_the_scan_cache_on_the_live_machine_answers_one_way_or_the_other() {
        // No assertion about what is in the air — there may be nothing, and
        // the machine may have no Wi-Fi at all. What is being checked is that
        // the WLAN call either produces access points or says why it cannot,
        // and never both and never neither.
        let net = crate::probe::netstate::read();
        let scan = scan(&net);
        assert!(scan.error.is_some() || scan.aps.is_empty() || !scan.load_24.is_empty());
        if scan.error.is_some() {
            assert!(scan.aps.is_empty());
        }
    }

    #[test]
    fn dfs_channels_are_never_recommended() {
        assert!(!NON_DFS_5.contains(&52));
        assert!(!NON_DFS_5.contains(&100));
    }

    #[test]
    fn sitting_on_a_radar_channel_is_recognised() {
        let on = |channel| AirScan { current: Some(channel), ..Default::default() }.on_dfs();
        assert!(on(52));
        assert!(on(108));
        assert!(on(140));
        assert!(!on(36));
        assert!(!on(149));
        assert!(!on(6));
        assert!(!AirScan::default().on_dfs());
    }
}
