//! Adapter, routing and Wi-Fi state, read straight from Windows.
//!
//! The Python prototype scraped `ipconfig`, `netsh` and PowerShell. That works
//! until Windows is installed in another language, at which point the parser
//! silently finds nothing and the app cheerfully reports a healthy connection
//! during an outage. These APIs return structs, so there is nothing to
//! mistranslate.

use std::net::{Ipv4Addr, SocketAddr};
use std::ptr;

use windows::core::GUID;
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS, HANDLE};
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::NetworkManagement::Ndis::IF_OPER_STATUS;
use windows::Win32::NetworkManagement::WiFi::{
    WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WlanQueryInterface,
    DOT11_PHY_TYPE, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST,
    WLAN_INTERFACE_STATE, WLAN_OPCODE_VALUE_TYPE,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN};

// Interface types from ifdef.h.
const IF_TYPE_IEEE80211: u32 = 71;
const IF_TYPE_ETHERNET_CSMACD: u32 = 6;

// wlan_intf_opcode_* from wlanapi.h. The windows crate exposes these as a
// struct-wrapped i32 whose constants are not all generated, so we name them.
const WLAN_INTF_OPCODE_CURRENT_CONNECTION: i32 = 7;
const WLAN_INTF_OPCODE_CHANNEL_NUMBER: i32 = 8;
const WLAN_INTF_OPCODE_RSSI: i32 = 0x1000_0102;

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Medium {
    Wifi,
    Ethernet,
    #[default]
    Unknown,
}

impl Medium {
    pub fn label(&self) -> &'static str {
        match self {
            Medium::Wifi => "Wi-Fi",
            Medium::Ethernet => "Ethernet",
            Medium::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NetState {
    pub adapter_name: String,
    pub adapter_desc: String,
    pub adapter_guid: String,
    pub medium: Medium,
    pub link_speed_mbps: u64,
    pub local_ip: Option<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
    pub dns_servers: Vec<Ipv4Addr>,
    pub up: bool,
    // Wi-Fi only
    pub ssid: String,
    pub bssid: String,
    pub signal_pct: Option<u32>,
    pub rssi_dbm: Option<i32>,
    pub channel: Option<u32>,
    pub phy: String,
    pub rx_mbps: Option<u32>,
    pub tx_mbps: Option<u32>,
    pub security: String,
}

impl NetState {
    pub fn band(&self) -> Option<&'static str> {
        match self.channel? {
            1..=14 => Some("2.4 GHz"),
            36..=177 => Some("5 GHz"),
            _ => Some("6 GHz"),
        }
    }

    pub fn dns_is_router_only(&self) -> bool {
        !self.dns_servers.is_empty()
            && self.gateway.is_some()
            && self.dns_servers.iter().all(|d| Some(*d) == self.gateway)
    }
}

/// Snapshot of the connection carrying the default route.
pub fn read() -> NetState {
    let mut st = read_adapters();
    if st.medium == Medium::Wifi {
        fill_wifi(&mut st);
    }
    st
}

fn sockaddr_to_ipv4(sa: *const SOCKADDR_IN) -> Option<Ipv4Addr> {
    if sa.is_null() {
        return None;
    }
    unsafe {
        if (*sa).sin_family != AF_INET {
            return None;
        }
        let bytes = (*sa).sin_addr.S_un.S_addr.to_le_bytes();
        Some(Ipv4Addr::from(bytes))
    }
}

fn wide_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe {
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

/// Walks the adapter list and returns the one holding a default gateway.
fn read_adapters() -> NetState {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u8> = Vec::new();

    // The documented dance: ask, grow, ask again.
    for _ in 0..4 {
        buf.resize(size as usize, 0);
        let rc = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                flags,
                None,
                Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut size,
            )
        };
        if rc == ERROR_SUCCESS.0 {
            break;
        }
        if rc != ERROR_BUFFER_OVERFLOW.0 {
            return NetState::default();
        }
    }

    let mut best = NetState::default();
    let mut cur = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;

    unsafe {
        while !cur.is_null() {
            let a = &*cur;
            let mut st = NetState {
                adapter_name: wide_to_string(a.FriendlyName.0),
                adapter_desc: wide_to_string(a.Description.0),
                adapter_guid: {
                    let s = std::ffi::CStr::from_ptr(a.AdapterName.0 as *const i8);
                    s.to_string_lossy().to_string()
                },
                medium: match a.IfType {
                    IF_TYPE_IEEE80211 => Medium::Wifi,
                    IF_TYPE_ETHERNET_CSMACD => Medium::Ethernet,
                    _ => Medium::Unknown,
                },
                link_speed_mbps: a.TransmitLinkSpeed / 1_000_000,
                up: a.OperStatus == IF_OPER_STATUS(1),
                ..Default::default()
            };

            // First IPv4 gateway on this adapter.
            let mut gw = a.FirstGatewayAddress;
            while !gw.is_null() {
                if let Some(ip) = sockaddr_to_ipv4((*gw).Address.lpSockaddr as *const SOCKADDR_IN) {
                    st.gateway = Some(ip);
                    break;
                }
                gw = (*gw).Next;
            }

            let mut ua = a.FirstUnicastAddress;
            while !ua.is_null() {
                if let Some(ip) = sockaddr_to_ipv4((*ua).Address.lpSockaddr as *const SOCKADDR_IN) {
                    st.local_ip = Some(ip);
                    break;
                }
                ua = (*ua).Next;
            }

            let mut dns = a.FirstDnsServerAddress;
            while !dns.is_null() {
                if let Some(ip) = sockaddr_to_ipv4((*dns).Address.lpSockaddr as *const SOCKADDR_IN) {
                    st.dns_servers.push(ip);
                }
                dns = (*dns).Next;
            }

            // The adapter that owns the default route is the one we care
            // about; prefer an up interface with a gateway.
            if st.gateway.is_some() && st.up && best.gateway.is_none() {
                best = st;
            }

            cur = a.Next;
        }
    }

    best
}

struct WlanHandle(HANDLE);

impl Drop for WlanHandle {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.0, None);
        }
    }
}

fn fill_wifi(st: &mut NetState) {
    unsafe {
        let mut version = 0u32;
        let mut raw = HANDLE::default();
        if WlanOpenHandle(2, None, &mut version, &mut raw) != ERROR_SUCCESS.0 {
            return;
        }
        let handle = WlanHandle(raw);

        let mut list: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
        if WlanEnumInterfaces(handle.0, None, &mut list) != ERROR_SUCCESS.0 || list.is_null() {
            return;
        }

        let count = (*list).dwNumberOfItems as usize;
        for i in 0..count {
            let info = (*list).InterfaceInfo.as_ptr().add(i);
            let guid: GUID = (*info).InterfaceGuid;

            // Only describe the interface we are actually routed through.
            let name = wide_to_string((*info).strInterfaceDescription.as_ptr());
            if !st.adapter_desc.is_empty() && !name.eq_ignore_ascii_case(&st.adapter_desc) {
                continue;
            }
            if (*info).isState != WLAN_INTERFACE_STATE(1) {
                // not connected
                continue;
            }

            read_connection(handle.0, &guid, st);
            read_channel(handle.0, &guid, st);
            read_rssi(handle.0, &guid, st);
            break;
        }

        WlanFreeMemory(list as *const _);
    }
}

unsafe fn query_raw(
    handle: HANDLE,
    guid: &GUID,
    opcode: i32,
) -> Option<(*mut std::ffi::c_void, u32)> {
    let mut size = 0u32;
    let mut data: *mut std::ffi::c_void = ptr::null_mut();
    let mut value_type = WLAN_OPCODE_VALUE_TYPE::default();
    let rc = WlanQueryInterface(
        handle,
        guid,
        windows::Win32::NetworkManagement::WiFi::WLAN_INTF_OPCODE(opcode),
        None,
        &mut size,
        &mut data,
        Some(&mut value_type),
    );
    if rc != ERROR_SUCCESS.0 || data.is_null() {
        return None;
    }
    Some((data, size))
}

unsafe fn read_connection(handle: HANDLE, guid: &GUID, st: &mut NetState) {
    let Some((data, _)) = query_raw(handle, guid, WLAN_INTF_OPCODE_CURRENT_CONNECTION) else {
        return;
    };
    let conn = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
    let assoc = &conn.wlanAssociationAttributes;

    let ssid_len = assoc.dot11Ssid.uSSIDLength as usize;
    st.ssid = String::from_utf8_lossy(&assoc.dot11Ssid.ucSSID[..ssid_len.min(32)]).to_string();

    let m = assoc.dot11Bssid;
    st.bssid = format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        m[0], m[1], m[2], m[3], m[4], m[5]
    );
    st.signal_pct = Some(assoc.wlanSignalQuality);
    st.rx_mbps = Some(assoc.ulRxRate / 1000);
    st.tx_mbps = Some(assoc.ulTxRate / 1000);
    st.phy = phy_name(assoc.dot11PhyType);
    st.security = security_name(&conn.wlanSecurityAttributes);

    WlanFreeMemory(data as *const _);
}

unsafe fn read_channel(handle: HANDLE, guid: &GUID, st: &mut NetState) {
    if let Some((data, _)) = query_raw(handle, guid, WLAN_INTF_OPCODE_CHANNEL_NUMBER) {
        st.channel = Some(*(data as *const u32));
        WlanFreeMemory(data as *const _);
    }
}

unsafe fn read_rssi(handle: HANDLE, guid: &GUID, st: &mut NetState) {
    if let Some((data, _)) = query_raw(handle, guid, WLAN_INTF_OPCODE_RSSI) {
        st.rssi_dbm = Some(*(data as *const i32));
        WlanFreeMemory(data as *const _);
    }
}

fn phy_name(t: DOT11_PHY_TYPE) -> String {
    // Values from wlantypes.h.
    match t.0 {
        1 => "802.11 FHSS".into(),
        2 => "802.11 DSSS".into(),
        3 => "802.11 IR".into(),
        4 => "802.11a (OFDM)".into(),
        5 => "802.11b (HR-DSSS)".into(),
        6 => "802.11g (ERP)".into(),
        7 => "802.11n (HT)".into(),
        8 => "802.11ac (VHT)".into(),
        9 => "802.11ad (DMG)".into(),
        10 => "802.11ax (HE)".into(),
        11 => "802.11be (EHT)".into(),
        _ => "unknown".into(),
    }
}

fn security_name(
    sec: &windows::Win32::NetworkManagement::WiFi::WLAN_SECURITY_ATTRIBUTES,
) -> String {
    // DOT11_AUTH_ALGORITHM values.
    match sec.dot11AuthAlgorithm.0 {
        1 => "Open".into(),
        2 => "Shared".into(),
        3 => "WPA".into(),
        4 => "WPA-PSK".into(),
        6 => "WPA2".into(),
        7 => "WPA2-PSK".into(),
        8 => "WPA3".into(),
        9 | 10 => "WPA3-SAE".into(),
        _ => {
            if sec.bSecurityEnabled.as_bool() {
                "secured".into()
            } else {
                "open".into()
            }
        }
    }
}

/// True when the Wi-Fi radio reports an active association. Used to tell a
/// dropped association apart from a router that stopped answering.
pub fn wifi_associated() -> bool {
    unsafe {
        let mut version = 0u32;
        let mut raw = HANDLE::default();
        if WlanOpenHandle(2, None, &mut version, &mut raw) != ERROR_SUCCESS.0 {
            return false;
        }
        let handle = WlanHandle(raw);
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
        if WlanEnumInterfaces(handle.0, None, &mut list) != ERROR_SUCCESS.0 || list.is_null() {
            return false;
        }
        let mut connected = false;
        for i in 0..(*list).dwNumberOfItems as usize {
            let info = (*list).InterfaceInfo.as_ptr().add(i);
            if (*info).isState == WLAN_INTERFACE_STATE(1) {
                connected = true;
                break;
            }
        }
        WlanFreeMemory(list as *const _);
        connected
    }
}

/// Resolve a hostname, timing how long the resolver took.
pub fn dns_lookup_ms(host: &str) -> (Option<f64>, String) {
    use std::net::ToSocketAddrs;
    let started = std::time::Instant::now();
    match (host, 80u16).to_socket_addrs() {
        Ok(mut it) => {
            let found: Option<SocketAddr> = it.next();
            if found.is_none() {
                return (None, "resolver returned no addresses".into());
            }
            (Some(started.elapsed().as_secs_f64() * 1000.0), String::new())
        }
        Err(e) => (None, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_follows_channel() {
        let mut st = NetState::default();
        st.channel = Some(6);
        assert_eq!(st.band(), Some("2.4 GHz"));
        st.channel = Some(108);
        assert_eq!(st.band(), Some("5 GHz"));
        st.channel = None;
        assert_eq!(st.band(), None);
    }

    #[test]
    fn router_only_dns_is_detected() {
        let mut st = NetState::default();
        st.gateway = Some(Ipv4Addr::new(192, 168, 50, 1));
        st.dns_servers = vec![Ipv4Addr::new(192, 168, 50, 1)];
        assert!(st.dns_is_router_only());
        st.dns_servers.push(Ipv4Addr::new(1, 1, 1, 1));
        assert!(!st.dns_is_router_only());
    }

    #[test]
    fn reading_state_does_not_panic() {
        let st = read();
        // On a machine with no network this is all empty, which is fine; the
        // point is that the FFI walk stays inside its buffers.
        let _ = st.band();
    }
}
