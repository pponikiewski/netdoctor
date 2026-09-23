//! The router's own account of its internet connection, over UPnP IGD.
//!
//! Pings from this machine can say the provider stopped answering; only the
//! router can say whether it still considers its WAN link up, how long it has
//! been running, and whether it came back with a new public address. Those
//! three are what tell "the router restarted" apart from "the provider
//! dropped the line", which is the argument a support call turns on.
//!
//! Read-only: discovery, one description fetch, and the two status queries
//! every IGD answers without authentication. Nothing is ever mapped or set.
//! A router with UPnP switched off simply has no readings, and the analysis
//! then says nothing about it.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const SSDP_ADDR: &str = "239.255.255.250:1900";
const SEARCH_TARGET: &str = "urn:schemas-upnp-org:device:InternetGatewayDevice:1";
const WAIT: Duration = Duration::from_secs(2);
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);

/// Where to ask: the WAN connection service's control URL and its type.
#[derive(Debug, Clone, PartialEq)]
pub struct Igd {
    pub control_url: String,
    pub service: String,
}

/// One answer from the router.
#[derive(Debug, Clone, PartialEq)]
pub struct RouterReading {
    pub ts: f64,
    /// `NewConnectionStatus` as the router words it: "Connected",
    /// "Disconnected", "Connecting", ...
    pub status: String,
    /// Seconds the router's UPnP service has been running. On most routers
    /// that restarts with the router or with its WAN connection, and the
    /// analysis says exactly that much.
    pub uptime_s: Option<u64>,
    /// A fingerprint of the public address, not the address: telling "the
    /// same" from "a new one" is all the analysis needs, and the database
    /// then holds no address to leak.
    pub ip_tag: Option<String>,
}

impl RouterReading {
    /// The wall-clock moment the uptime counter started from.
    pub fn counter_start(&self) -> Option<f64> {
        self.uptime_s.map(|u| self.ts - u as f64)
    }

    pub fn connected(&self) -> bool {
        self.status.eq_ignore_ascii_case("Connected")
    }
}

/// Finds the gateway's IGD. Only an answer from `gateway` itself counts: a
/// second router or a NAS on the network advertising IGD is not the one this
/// machine's traffic goes through.
pub fn discover(gateway: Ipv4Addr) -> Option<Igd> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    let search = format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_ADDR}\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\n\
         ST: {SEARCH_TARGET}\r\n\r\n"
    );
    let to: SocketAddr = SSDP_ADDR.parse().ok()?;
    socket.send_to(search.as_bytes(), to).ok()?;

    let deadline = Instant::now() + WAIT;
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        let Ok((n, from)) = socket.recv_from(&mut buf) else { continue };
        if from.ip() != gateway {
            continue;
        }
        let reply = String::from_utf8_lossy(&buf[..n]);
        let Some(location) = header(&reply, "location") else { continue };
        // Only a description served by the gateway itself: the address in
        // the reply is the router's word, and it is asked again every 30 s.
        if !on_host(&location, gateway) {
            continue;
        }
        let description =
            ureq::get(&location).timeout(HTTP_TIMEOUT).call().ok()?.into_string().ok()?;
        return wan_service(&description, &location);
    }
    None
}

/// Asks for the connection status and the public address.
pub fn read(igd: &Igd) -> Option<RouterReading> {
    let status = soap(igd, "GetStatusInfo")?;
    let ip = soap(igd, "GetExternalIPAddress")
        .and_then(|r| element(&r, "NewExternalIPAddress"))
        .and_then(|a| a.trim().parse::<Ipv4Addr>().ok())
        .filter(|a| !a.is_unspecified());
    Some(RouterReading {
        ts: crate::store::now(),
        status: element(&status, "NewConnectionStatus")?.trim().to_string(),
        uptime_s: element(&status, "NewUptime").and_then(|u| u.trim().parse().ok()),
        ip_tag: ip.map(tag),
    })
}

fn soap(igd: &Igd, action: &str) -> Option<String> {
    let body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
         <u:{action} xmlns:u=\"{}\"/></s:Body></s:Envelope>",
        igd.service
    );
    ureq::post(&igd.control_url)
        .timeout(HTTP_TIMEOUT)
        .set("Content-Type", "text/xml; charset=\"utf-8\"")
        .set("SOAPAction", &format!("\"{}#{action}\"", igd.service))
        .send_string(&body)
        .ok()?
        .into_string()
        .ok()
}

/// FNV-1a over the address, as hex. Stable across runs and Rust versions,
/// which `DefaultHasher` does not promise.
// ponytail: an IPv4 address fingerprint can be brute-forced by anyone holding
// the database; it keeps the address out of casual view, not out of reach.
fn tag(ip: Ipv4Addr) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in ip.octets() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Whether `url` is plain HTTP on `host`, with or without a port.
fn on_host(url: &str, host: Ipv4Addr) -> bool {
    url.strip_prefix(&format!("http://{host}"))
        .is_some_and(|rest| rest.starts_with(':') || rest.starts_with('/'))
}

/// A header's value from an HTTP-style reply, by case-insensitive name.
fn header(reply: &str, name: &str) -> Option<String> {
    reply.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
    })
}

/// The text of the first `<name>` element. The documents here are small and
/// flat enough that this is all the XML reading they need.
// ponytail: string search, not an XML parser; a router that namespaces its
// response elements (`<m:NewUptime>`) is not read. Add a prefix-tolerant
// match if one turns up.
fn element(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(xml[start..end].to_string())
}

/// The WANIPConnection or WANPPPConnection service in a device description,
/// with its control URL made absolute against where the description came
/// from.
fn wan_service(description: &str, location: &str) -> Option<Igd> {
    let base = {
        let after_scheme = location.find("://")? + 3;
        let host_end =
            location[after_scheme..].find('/').map_or(location.len(), |i| i + after_scheme);
        &location[..host_end]
    };
    for block in description.split("<service>").skip(1) {
        let Some(service) = element(block, "serviceType") else { continue };
        let service = service.trim().to_string();
        if !(service.contains(":service:WANIPConnection:")
            || service.contains(":service:WANPPPConnection:"))
        {
            continue;
        }
        let control = element(block, "controlURL")?.trim().to_string();
        let control_url = if control.starts_with("http") {
            // An absolute URL elsewhere is not followed: see `discover`.
            if !control.starts_with(base) {
                continue;
            }
            control
        } else if control.starts_with('/') {
            format!("{base}{control}")
        } else {
            format!("{base}/{control}")
        };
        return Some(Igd { control_url, service });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIPTION: &str = r#"<root><device><serviceList>
        <service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType>
        <controlURL>/ctl/L3F</controlURL></service>
        <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
        <controlURL>/ctl/IPConn</controlURL></service>
        </serviceList></device></root>"#;

    #[test]
    fn the_wan_service_is_found_and_its_url_made_absolute() {
        let igd = wan_service(DESCRIPTION, "http://192.168.50.1:44751/rootDesc.xml").unwrap();
        assert_eq!(igd.control_url, "http://192.168.50.1:44751/ctl/IPConn");
        assert_eq!(igd.service, "urn:schemas-upnp-org:service:WANIPConnection:1");
        assert_eq!(wan_service("<root/>", "http://192.168.50.1/"), None);
    }

    #[test]
    fn only_a_description_on_the_gateway_is_followed() {
        let gw = Ipv4Addr::new(192, 168, 1, 1);
        assert!(on_host("http://192.168.1.1:5000/desc.xml", gw));
        assert!(on_host("http://192.168.1.1/desc.xml", gw));
        assert!(!on_host("http://192.168.1.10:5000/desc.xml", gw), "a prefix is not the host");
        assert!(!on_host("http://example.com/desc.xml", gw));
        let elsewhere = DESCRIPTION.replace("/ctl/IPConn", "http://example.com/ctl");
        assert_eq!(wan_service(&elsewhere, "http://192.168.1.1:5000/d.xml"), None);
    }

    #[test]
    fn the_ssdp_location_header_is_read_whatever_its_case() {
        let reply = "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.1:5000/desc.xml\r\nST: x\r\n";
        assert_eq!(header(reply, "location").as_deref(), Some("http://192.168.1.1:5000/desc.xml"));
    }

    #[test]
    fn a_status_answer_is_read_into_its_fields() {
        let xml = "<s:Body><u:GetStatusInfoResponse><NewConnectionStatus>Connected\
                   </NewConnectionStatus><NewUptime>1650412</NewUptime></u:GetStatusInfoResponse>";
        assert_eq!(element(xml, "NewConnectionStatus").as_deref(), Some("Connected"));
        assert_eq!(element(xml, "NewUptime").as_deref(), Some("1650412"));
        assert_eq!(element(xml, "Missing"), None);
    }

    #[test]
    fn the_address_fingerprint_tells_addresses_apart_and_is_stable() {
        let a = tag(Ipv4Addr::new(203, 0, 113, 7));
        assert_eq!(a, tag(Ipv4Addr::new(203, 0, 113, 7)));
        assert_ne!(a, tag(Ipv4Addr::new(203, 0, 113, 8)));
        assert!(!a.contains("203"));
    }

    /// Against the router this machine is on. Ignored: it needs a network,
    /// and a router that has UPnP switched off is not a failure.
    #[test]
    #[ignore]
    fn the_live_router_answers() {
        let net = crate::probe::netstate::read();
        let gw = net.gateway.expect("a gateway");
        let igd = discover(gw).expect("the router answers SSDP");
        let r = read(&igd).expect("the router answers GetStatusInfo");
        println!("{} uptime={:?} ip_tag={:?}", r.status, r.uptime_s, r.ip_tag.is_some());
        assert!(!r.status.is_empty());
    }
}
