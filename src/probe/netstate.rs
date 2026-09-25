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
    GetAdaptersAddresses, GetBestInterface, GetIfEntry2, GAA_FLAG_INCLUDE_GATEWAYS,
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2,
};
use windows::Win32::NetworkManagement::Ndis::IF_OPER_STATUS;
use windows::Win32::NetworkManagement::WiFi::{
    WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WlanQueryInterface,
    DOT11_PHY_TYPE, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST, WLAN_INTERFACE_STATE,
    WLAN_OPCODE_VALUE_TYPE,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN};

// Interface types from ifdef.h.
const IF_TYPE_IEEE80211: u32 = 71;
const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const IF_TYPE_PPP: u32 = 23;
const IF_TYPE_PROP_VIRTUAL: u32 = 53;
const IF_TYPE_TUNNEL: u32 = 131;

/// Words in an adapter's description that name a VPN. Most VPN clients
/// install an adapter that reports itself as Ethernet, so the interface type
/// alone misses them.
const VPN_WORDS: [&str; 12] = [
    "vpn",
    "wireguard",
    "wintun",
    "tap-windows",
    "tap adapter",
    "openvpn",
    "nordlynx",
    "anyconnect",
    "fortinet",
    "globalprotect",
    "zerotier",
    "tailscale",
];

/// Whether the adapter traffic leaves through is a tunnel rather than the
/// card itself. Then the "router" a scan pings is the far end of the tunnel,
/// and every number it takes includes the trip there.
fn is_tunnel(if_type: u32, desc: &str) -> bool {
    let desc = desc.to_lowercase();
    matches!(if_type, IF_TYPE_PPP | IF_TYPE_PROP_VIRTUAL | IF_TYPE_TUNNEL)
        || VPN_WORDS.iter().any(|w| desc.contains(w))
}

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
            Medium::Unknown => crate::i18n::medium_unknown(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NetState {
    pub adapter_name: String,
    pub adapter_desc: String,
    pub adapter_guid: String,
    /// The IPv4 interface index, for reading the adapter's counters.
    pub if_index: u32,
    pub medium: Medium,
    pub link_speed_mbps: u64,
    pub local_ip: Option<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
    pub dns_servers: Vec<Ipv4Addr>,
    pub up: bool,
    /// The routed adapter is a VPN or another tunnel. See [`is_tunnel`].
    pub tunnel: bool,
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

    /// A resolver on the user's own network that is not the router: a
    /// Pi-hole, an AdGuard Home, a company server. Someone chose it, and
    /// swapping it for a public one would switch off its filtering or its
    /// internal names, so it is never offered as the fix.
    pub fn dns_is_own_resolver(&self) -> bool {
        self.dns_servers.iter().any(|d| d.is_private() && Some(*d) != self.gateway)
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

/// The interface index Windows itself would route the internet through.
///
/// `GetBestInterface` answers the question the enumeration order only guessed
/// at. On a docked laptop with Wi-Fi and Ethernet both up, or with a VPN or a
/// Hyper-V switch in the list, the first adapter carrying a gateway is not
/// necessarily the one the traffic takes; the routing table knows, and this
/// asks it. `None` when there is no route at all, which is exactly the case
/// the caller has to survive rather than give up on.
fn best_route_interface() -> Option<u32> {
    // A public address rather than 0.0.0.0: the question is "which way out",
    // and an all-zero destination is not a destination.
    let dest = u32::from(Ipv4Addr::new(1, 1, 1, 1)).to_be();
    let mut index = 0u32;
    let rc = unsafe { GetBestInterface(dest, &mut index) };
    (rc == ERROR_SUCCESS.0 && index != 0).then_some(index)
}

/// How good a candidate an adapter is, highest first.
///
/// Read as: the one the routing table named, then anything with a gateway,
/// then anything that is up, and among equals the lower interface metric —
/// which is how Windows breaks the same tie.
fn rank(is_best_route: bool, has_gateway: bool, up: bool, metric: u32) -> (u8, u8, u8, i64) {
    (is_best_route as u8, has_gateway as u8, up as u8, -(metric as i64))
}

/// Walks the adapter list and returns the connection worth describing.
///
/// It used to return the first adapter that had a gateway *and* was up, and
/// nothing at all otherwise. That second half was the bug: during an outage
/// there is no gateway, so the read came back empty, the monitor kept the last
/// good state, and the app showed the SSID, the signal and the channel from
/// before the failure — for the whole length of it, which is precisely when
/// somebody is looking. Now an adapter with no gateway is still an adapter.
///
/// Measured on a live drop: `up` goes false and the Wi-Fi fields empty within
/// a second, while the gateway and the local address linger for several more,
/// because Windows holds the lease and the route entry for a while after the
/// radio disassociates. So `up` and the empty SSID are the prompt facts here;
/// `gateway.is_none()` is a slower one, and nothing should wait on it to
/// notice that a link has gone.
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

    let routed = best_route_interface();
    let mut best = NetState::default();
    let mut best_rank = (0u8, 0u8, 0u8, i64::MIN);
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
                if_index: a.Anonymous1.Anonymous.IfIndex,
                link_speed_mbps: a.TransmitLinkSpeed / 1_000_000,
                up: a.OperStatus == IF_OPER_STATUS(1),
                tunnel: is_tunnel(a.IfType, &wide_to_string(a.Description.0)),
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
                if let Some(ip) = sockaddr_to_ipv4((*dns).Address.lpSockaddr as *const SOCKADDR_IN)
                {
                    st.dns_servers.push(ip);
                }
                dns = (*dns).Next;
            }

            // The loopback describes nothing about the link and would win on
            // metric alone once adapters without a gateway are candidates.
            if a.IfType != IF_TYPE_SOFTWARE_LOOPBACK {
                let index = a.Anonymous1.Anonymous.IfIndex;
                let score = rank(routed == Some(index), st.gateway.is_some(), st.up, a.Ipv4Metric);
                if score > best_rank {
                    best_rank = score;
                    best = st;
                }
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

/// # Safety
/// `handle` must be an open WLAN client handle and `guid` one of the interface
/// GUIDs it enumerated. On `Some`, the caller owns the returned block and must
/// hand it to `WlanFreeMemory`.
unsafe fn query_raw(
    handle: HANDLE,
    guid: &GUID,
    opcode: i32,
) -> Option<(*mut std::ffi::c_void, u32)> {
    let mut size = 0u32;
    let mut data: *mut std::ffi::c_void = ptr::null_mut();
    let mut value_type = WLAN_OPCODE_VALUE_TYPE::default();
    let rc = unsafe {
        WlanQueryInterface(
            handle,
            guid,
            windows::Win32::NetworkManagement::WiFi::WLAN_INTF_OPCODE(opcode),
            None,
            &mut size,
            &mut data,
            Some(&mut value_type),
        )
    };
    if rc != ERROR_SUCCESS.0 || data.is_null() {
        return None;
    }
    Some((data, size))
}

/// # Safety
/// As `query_raw`: `handle` open, `guid` enumerated from it.
unsafe fn read_connection(handle: HANDLE, guid: &GUID, st: &mut NetState) {
    let Some((data, _)) = (unsafe { query_raw(handle, guid, WLAN_INTF_OPCODE_CURRENT_CONNECTION) })
    else {
        return;
    };
    // The opcode decides the shape of what comes back, and this is the one
    // Windows documents for CURRENT_CONNECTION.
    let conn = unsafe { &*(data as *const WLAN_CONNECTION_ATTRIBUTES) };
    let assoc = &conn.wlanAssociationAttributes;

    let ssid_len = assoc.dot11Ssid.uSSIDLength as usize;
    st.ssid = String::from_utf8_lossy(&assoc.dot11Ssid.ucSSID[..ssid_len.min(32)]).to_string();

    let m = assoc.dot11Bssid;
    st.bssid =
        format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5]);
    st.signal_pct = Some(assoc.wlanSignalQuality);
    st.rx_mbps = Some(assoc.ulRxRate / 1000);
    st.tx_mbps = Some(assoc.ulTxRate / 1000);
    st.phy = phy_name(assoc.dot11PhyType);
    st.security = security_name(&conn.wlanSecurityAttributes);

    unsafe { WlanFreeMemory(data as *const _) };
}

/// # Safety
/// As `query_raw`: `handle` open, `guid` enumerated from it.
unsafe fn read_channel(handle: HANDLE, guid: &GUID, st: &mut NetState) {
    if let Some((data, _)) = unsafe { query_raw(handle, guid, WLAN_INTF_OPCODE_CHANNEL_NUMBER) } {
        st.channel = Some(unsafe { *(data as *const u32) });
        unsafe { WlanFreeMemory(data as *const _) };
    }
}

/// # Safety
/// As `query_raw`: `handle` open, `guid` enumerated from it.
unsafe fn read_rssi(handle: HANDLE, guid: &GUID, st: &mut NetState) {
    if let Some((data, _)) = unsafe { query_raw(handle, guid, WLAN_INTF_OPCODE_RSSI) } {
        st.rssi_dbm = Some(unsafe { *(data as *const i32) });
        unsafe { WlanFreeMemory(data as *const _) };
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

/// Whether the Wi-Fi card `adapter_guid` (as in [`NetState::adapter_guid`])
/// reports an active association, or `None` when that could not be read.
/// Used to tell a dropped association apart from a router that stopped
/// answering, so "unknown" must not be read as either.
pub fn wifi_associated(adapter_guid: &str) -> Option<bool> {
    association_of(wlan_interfaces(), adapter_guid)
}

/// Every WLAN interface as (GUID, associated), or `None` when the WLAN
/// service could not be asked.
fn wlan_interfaces() -> Option<Vec<(String, bool)>> {
    unsafe {
        let mut version = 0u32;
        let mut raw = HANDLE::default();
        if WlanOpenHandle(2, None, &mut version, &mut raw) != ERROR_SUCCESS.0 {
            return None;
        }
        let handle = WlanHandle(raw);
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
        if WlanEnumInterfaces(handle.0, None, &mut list) != ERROR_SUCCESS.0 || list.is_null() {
            return None;
        }
        let mut out = Vec::new();
        for i in 0..(*list).dwNumberOfItems as usize {
            let info = (*list).InterfaceInfo.as_ptr().add(i);
            out.push((
                guid_text(&(*info).InterfaceGuid),
                (*info).isState == WLAN_INTERFACE_STATE(1),
            ));
        }
        WlanFreeMemory(list as *const _);
        Some(out)
    }
}

/// A GUID in the form `GetAdaptersAddresses` names adapters by.
fn guid_text(g: &GUID) -> String {
    let d = g.data4;
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1, g.data2, g.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

/// Whether the interface `wanted` is associated, from the WLAN listing.
///
/// `None` when that could not be read: no listing, or no entry for this card.
/// Both used to come back as "not associated", and any other card that was
/// connected answered for this one.
fn association_of(listing: Option<Vec<(String, bool)>>, wanted: &str) -> Option<bool> {
    let bare = |g: &str| g.trim_matches(|c| c == '{' || c == '}').to_ascii_lowercase();
    let wanted = bare(wanted);
    listing?.into_iter().find(|(guid, _)| bare(guid) == wanted).map(|(_, connected)| connected)
}

/// Resolve a hostname, timing how long the resolver took.
pub fn dns_lookup_ms(host: &str) -> (Option<f64>, String) {
    use std::net::ToSocketAddrs;
    let started = std::time::Instant::now();
    match (host, 80u16).to_socket_addrs() {
        Ok(mut it) => {
            let found: Option<SocketAddr> = it.next();
            if found.is_none() {
                return (None, crate::i18n::dns_no_addresses().into());
            }
            (Some(started.elapsed().as_secs_f64() * 1000.0), String::new())
        }
        Err(e) => (None, e.to_string()),
    }
}

/// How long one resolver gets to answer a direct query.
const DNS_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Frames the adapter counted, and the ones it counted as broken. Running
/// totals since the adapter came up.
///
/// Discards are left out on purpose: a card discards frames for a protocol
/// nobody here speaks, which is housekeeping, not damage. Errors are frames
/// that arrived or left corrupted — the CRC failures of a bad cable or port.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkCounters {
    pub packets: u64,
    pub errors: u64,
    /// Bytes received and sent.
    pub bytes: u64,
}

impl LinkCounters {
    /// What happened between `self` and a later reading. `None` when the
    /// counters went backwards: the adapter was reset in between, and the
    /// difference would be meaningless.
    pub fn since(&self, earlier: &LinkCounters) -> Option<LinkCounters> {
        Some(LinkCounters {
            packets: self.packets.checked_sub(earlier.packets)?,
            errors: self.errors.checked_sub(earlier.errors)?,
            bytes: self.bytes.checked_sub(earlier.bytes)?,
        })
    }

    /// Traffic through the adapter between `earlier` and `self`, `secs`
    /// apart, in Mbit/s both ways together. `None` across a reset.
    pub fn mbps_since(&self, earlier: &LinkCounters, secs: f64) -> Option<f64> {
        if secs <= 0.0 {
            return None;
        }
        let d = self.since(earlier)?;
        Some(d.bytes as f64 * 8.0 / secs / 1_000_000.0)
    }
}

/// Reads the counters of interface `if_index`. `None` when there is no such
/// interface any more, or the call failed.
pub fn link_counters(if_index: u32) -> Option<LinkCounters> {
    if if_index == 0 {
        return None;
    }
    let mut row = MIB_IF_ROW2 { InterfaceIndex: if_index, ..Default::default() };
    let rc = unsafe { GetIfEntry2(&mut row) };
    if rc != ERROR_SUCCESS {
        return None;
    }
    Some(LinkCounters {
        packets: row.InUcastPkts + row.InNUcastPkts + row.OutUcastPkts + row.OutNUcastPkts,
        errors: row.InErrors + row.OutErrors,
        bytes: row.InOctets + row.OutOctets,
    })
}

/// Asks every resolver in `servers` for `host` directly, all at once, and
/// times the fastest answer. The error names each one that failed.
///
/// Not `getaddrinfo`, which answers from the Windows DNS cache for as long as
/// the record lives: measured, a cached `example.com` comes back in 0.7 ms
/// against 29 ms for a real query, so a dead resolver went unnoticed until
/// the entry expired. A failed lookup can be held the same way by the
/// negative cache after the resolver is back. The question here is whether
/// the resolver answers. With no resolver known, the system lookup is all
/// there is.
pub fn resolvers_answer(servers: &[Ipv4Addr], host: &str) -> (Option<f64>, String) {
    if servers.is_empty() {
        return dns_lookup_ms(host);
    }
    let results: Vec<Result<f64, String>> = std::thread::scope(|s| {
        let asked: Vec<_> = servers
            .iter()
            .enumerate()
            .map(|(i, server)| s.spawn(move || query_resolver(*server, host, i as u16)))
            .collect();
        asked.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(String::new()))).collect()
    });
    let fastest = results.iter().filter_map(|r| r.as_ref().ok()).fold(f64::NAN, |a, b| a.min(*b));
    if fastest.is_finite() {
        return (Some(fastest), String::new());
    }
    let why: Vec<String> =
        results.into_iter().filter_map(Result::err).filter(|e| !e.is_empty()).collect();
    (None, why.join("; "))
}

/// One A query to `server`, timed from send to reply.
fn query_resolver(server: Ipv4Addr, host: &str, salt: u16) -> Result<f64, String> {
    use std::net::UdpSocket;
    let failed = |e: std::io::Error| format!("{server}: {e}");

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let id = (nanos as u16) ^ salt.rotate_left(8);

    let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(failed)?;
    socket.connect((server, 53)).map_err(failed)?;
    let started = std::time::Instant::now();
    socket.send(&dns_query(id, host)).map_err(failed)?;

    let mut buf = [0u8; 512];
    loop {
        let left = DNS_QUERY_TIMEOUT.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Err(crate::i18n::dns_server_silent(&server.to_string()));
        }
        socket.set_read_timeout(Some(left)).map_err(failed)?;
        match socket.recv(&mut buf) {
            Ok(n) => match dns_reply(id, &buf[..n]) {
                // Somebody else's datagram, or a late reply to an earlier
                // query: keep waiting for ours.
                DnsReply::NotOurs => continue,
                DnsReply::Answered => return Ok(started.elapsed().as_secs_f64() * 1000.0),
                DnsReply::Refused(code) => {
                    return Err(crate::i18n::dns_server_error(
                        &server.to_string(),
                        &rcode_name(code),
                    ))
                }
            },
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(crate::i18n::dns_server_silent(&server.to_string()))
            }
            Err(e) => return Err(failed(e)),
        }
    }
}

/// A standard recursive query for the A record of `host`.
fn dns_query(id: u16, host: &str) -> Vec<u8> {
    let mut q = Vec::with_capacity(18 + host.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // recursion desired
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // one question
    for label in host.split('.').filter(|l| !l.is_empty()) {
        let label = &label.as_bytes()[..label.len().min(63)];
        q.push(label.len() as u8);
        q.extend_from_slice(label);
    }
    q.push(0);
    q.extend_from_slice(&[0, 1, 0, 1]); // type A, class IN
    q
}

#[derive(Debug, PartialEq)]
enum DnsReply {
    NotOurs,
    /// The resolver answered the question, whatever the answer was.
    Answered,
    /// The resolver answered with an error code.
    Refused(u8),
}

fn dns_reply(id: u16, reply: &[u8]) -> DnsReply {
    if reply.len() < 12 || reply[..2] != id.to_be_bytes() || reply[2] & 0x80 == 0 {
        return DnsReply::NotOurs;
    }
    match reply[3] & 0x0f {
        0 => DnsReply::Answered,
        code => DnsReply::Refused(code),
    }
}

fn rcode_name(code: u8) -> String {
    match code {
        1 => "FORMERR".into(),
        2 => "SERVFAIL".into(),
        3 => "NXDOMAIN".into(),
        5 => "REFUSED".into(),
        other => format!("RCODE {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vpn_is_told_apart_by_its_type_or_its_name_and_a_card_is_not() {
        assert!(is_tunnel(IF_TYPE_TUNNEL, "Microsoft Teredo"));
        assert!(is_tunnel(IF_TYPE_PROP_VIRTUAL, "Some virtual adapter"));
        // Reports itself as Ethernet, and is a VPN.
        assert!(is_tunnel(IF_TYPE_ETHERNET_CSMACD, "TAP-Windows Adapter V9"));
        assert!(is_tunnel(IF_TYPE_ETHERNET_CSMACD, "WireGuard Tunnel"));
        assert!(is_tunnel(IF_TYPE_ETHERNET_CSMACD, "NordLynx Tunnel"));
        assert!(!is_tunnel(IF_TYPE_ETHERNET_CSMACD, "Realtek PCIe GbE Family Controller"));
        assert!(!is_tunnel(IF_TYPE_IEEE80211, "Intel(R) Wi-Fi 6 AX201 160MHz"));
    }

    #[test]
    fn a_dns_query_is_built_the_way_the_wire_expects() {
        let q = dns_query(0xbeef, "example.com");
        assert_eq!(&q[..12], &[0xbe, 0xef, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&q[12..], b"\x07example\x03com\x00\x00\x01\x00\x01");
    }

    #[test]
    fn a_reply_is_read_for_its_id_and_its_code_only() {
        let mut ok = vec![0xbe, 0xef, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
        assert_eq!(dns_reply(0xbeef, &ok), DnsReply::Answered);
        assert_eq!(dns_reply(0x1234, &ok), DnsReply::NotOurs, "another query's reply");
        ok[3] = 0x82;
        assert_eq!(dns_reply(0xbeef, &ok), DnsReply::Refused(2), "SERVFAIL");
        let query = dns_query(0xbeef, "example.com");
        assert_eq!(dns_reply(0xbeef, &query), DnsReply::NotOurs, "our own query echoed back");
        assert_eq!(dns_reply(0xbeef, &ok[..5]), DnsReply::NotOurs, "truncated");
    }

    #[test]
    #[ignore = "asks the live resolvers"]
    fn show_resolvers_answer() {
        let st = read();
        println!("servers {:?}", st.dns_servers);
        for _ in 0..3 {
            println!("direct: {:?}", resolvers_answer(&st.dns_servers, "example.com"));
            println!("system: {:?}", dns_lookup_ms("example.com"));
        }
        println!("dead: {:?}", resolvers_answer(&[Ipv4Addr::new(192, 0, 2, 1)], "example.com"));
    }

    #[test]
    fn the_association_asked_about_is_the_routed_cards_and_can_be_unknown() {
        const MINE: &str = "{6B29FC40-CA47-1067-B31D-00DD010662DA}";
        const OTHER: &str = "{0F8FAD5B-D9CB-469F-A165-70867728950E}";
        // The WLAN service did not answer: that is not "not associated".
        assert_eq!(association_of(None, MINE), None);
        // A second card that is connected says nothing about this one.
        let two = vec![(OTHER.to_string(), true), (MINE.to_string(), false)];
        assert_eq!(association_of(Some(two.clone()), MINE), Some(false));
        assert_eq!(association_of(Some(two), &MINE.to_lowercase()), Some(false), "case");
        // A card the listing does not know is unknown, not disconnected.
        assert_eq!(association_of(Some(vec![(OTHER.to_string(), true)]), MINE), None);
        assert_eq!(association_of(Some(vec![(MINE.to_string(), true)]), MINE), Some(true));
    }

    #[test]
    fn a_wlan_guid_is_written_the_way_the_adapter_list_writes_it() {
        let g = GUID::from_u128(0x6b29fc40_ca47_1067_b31d_00dd010662da);
        assert_eq!(guid_text(&g), "{6B29FC40-CA47-1067-B31D-00DD010662DA}");
    }

    #[test]
    fn band_follows_channel() {
        let mut st = NetState { channel: Some(6), ..Default::default() };
        assert_eq!(st.band(), Some("2.4 GHz"));
        st.channel = Some(108);
        assert_eq!(st.band(), Some("5 GHz"));
        st.channel = None;
        assert_eq!(st.band(), None);
    }

    #[test]
    fn router_only_dns_is_detected() {
        let mut st = NetState {
            gateway: Some(Ipv4Addr::new(192, 168, 50, 1)),
            dns_servers: vec![Ipv4Addr::new(192, 168, 50, 1)],
            ..Default::default()
        };
        assert!(st.dns_is_router_only());
        st.dns_servers.push(Ipv4Addr::new(1, 1, 1, 1));
        assert!(!st.dns_is_router_only());
    }

    #[test]
    fn a_pi_hole_is_an_own_resolver_and_the_router_is_not() {
        let mut st = NetState {
            gateway: Some(Ipv4Addr::new(192, 168, 50, 1)),
            dns_servers: vec![Ipv4Addr::new(192, 168, 50, 1), Ipv4Addr::new(1, 1, 1, 1)],
            ..Default::default()
        };
        assert!(!st.dns_is_own_resolver());
        st.dns_servers.insert(0, Ipv4Addr::new(192, 168, 50, 5));
        assert!(st.dns_is_own_resolver());
    }

    #[test]
    fn reading_state_does_not_panic() {
        let st = read();
        // On a machine with no network this is all empty, which is fine; the
        // point is that the FFI walk stays inside its buffers.
        let _ = st.band();
    }

    #[test]
    fn the_routing_table_outranks_every_other_signal() {
        // The docked-laptop case: Ethernet and Wi-Fi both up, both with a
        // gateway. Enumeration order used to decide, which is how an app can
        // describe the Wi-Fi while the traffic goes over the cable.
        let routed_wifi = rank(true, true, true, 30);
        let idle_ethernet = rank(false, true, true, 5);
        assert!(routed_wifi > idle_ethernet, "what Windows routes through wins on its own");
    }

    #[test]
    fn an_adapter_without_a_gateway_still_beats_nothing() {
        // The outage case. Before this, a state with no gateway was not a
        // candidate at all and the read came back empty.
        let stranded = rank(false, false, true, 30);
        let nothing = (0u8, 0u8, 0u8, i64::MIN);
        assert!(stranded > nothing);
    }

    #[test]
    fn the_lower_interface_metric_breaks_a_tie() {
        assert!(rank(false, true, true, 5) > rank(false, true, true, 30));
        // But only as a tie-break: a gateway is worth more than a low metric.
        assert!(rank(false, true, true, 30) > rank(false, false, true, 5));
        // And being up is worth more than a low metric.
        assert!(rank(false, false, true, 30) > rank(false, false, false, 5));
    }

    /// What the routing table answers on this machine, next to what `read()`
    /// picked. Ignored: it describes the live machine.
    #[test]
    fn traffic_is_read_off_the_byte_counters_and_not_across_a_reset() {
        let at = |bytes| LinkCounters { bytes, ..Default::default() };
        let mbps = at(3_000_000).mbps_since(&at(0), 10.0).unwrap_or(-1.0);
        assert!((mbps - 2.4).abs() < 1e-9, "{mbps}");
        assert_eq!(at(5).mbps_since(&at(10), 10.0), None, "counters went backwards");
        assert_eq!(at(10).mbps_since(&at(0), 0.0), None);
    }

    #[test]
    #[ignore = "watches the live machine"]
    fn show_link_counters() {
        let st = read();
        println!("adapter={} if_index={} {:?}", st.adapter_name, st.if_index, st.medium);
        println!("counters {:?}", link_counters(st.if_index));
        println!("no such interface {:?}", link_counters(u32::MAX));
    }

    #[test]
    #[ignore = "watches the live machine"]
    fn show_best_route_interface() {
        println!("GetBestInterface -> {:?}", best_route_interface());
        let st = read();
        println!("read() picked adapter={} guid={}", st.adapter_name, st.adapter_guid);
        println!(
            "WLAN lists {:?}; associated(picked) = {:?}",
            wlan_interfaces(),
            wifi_associated(&st.adapter_guid)
        );
    }

    /// Prints what `read()` makes of this machine, once a second.
    ///
    /// Ignored by default: it is a window onto the live machine, not an
    /// assertion. Run it with
    /// `cargo test -- --ignored watch_netstate --nocapture` and pull the cable
    /// or drop the Wi-Fi while it runs. The question it answers is whether a
    /// state without a default gateway still describes the adapter.
    #[test]
    #[ignore = "watches the live machine"]
    fn watch_netstate() {
        for i in 0..25 {
            let st = read();
            println!(
                "{i:>2}s adapter={:<20} up={} gw={:<15} ip={:<15} ssid={} rssi={:?}",
                if st.adapter_name.is_empty() { "<none>" } else { &st.adapter_name },
                st.up,
                st.gateway.map(|g| g.to_string()).unwrap_or_else(|| "-".into()),
                st.local_ip.map(|g| g.to_string()).unwrap_or_else(|| "-".into()),
                if st.ssid.is_empty() { "-" } else { &st.ssid },
                st.rssi_dbm,
            );
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}
