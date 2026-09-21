//! ICMP echo through the Windows IP Helper API.
//!
//! The obvious implementation shells out to `ping.exe`, but that costs ~40 ms
//! of process startup per sample, reports latency rounded to whole
//! milliseconds, and forces us to parse localised English/Polish text to find
//! out whether the reply was real. `IcmpSendEcho` needs no elevation, returns a
//! status code instead of prose, and lets us time the call ourselves at
//! microsecond resolution.

use std::mem::size_of;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, WAIT_TIMEOUT};
use windows::Win32::NetworkManagement::IpHelper::{
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY,
};

/// Status codes we translate into a human cause. The full list lives in
/// ipexport.h; these are the ones a home network actually produces.
const IP_SUCCESS: u32 = 0;
const IP_DEST_NET_UNREACHABLE: u32 = 11002;
const IP_DEST_HOST_UNREACHABLE: u32 = 11003;
const IP_DEST_PROT_UNREACHABLE: u32 = 11004;
const IP_DEST_PORT_UNREACHABLE: u32 = 11005;
const IP_REQ_TIMED_OUT: u32 = 11010;
const IP_TTL_EXPIRED_TRANSIT: u32 = 11013;
const IP_GENERAL_FAILURE: u32 = 11050;

/// 32 bytes of payload, the same as `ping.exe`, so middleboxes treat our
/// probes the way they treat everyone else's.
const PAYLOAD: &[u8; 32] = b"netdoctor-probe-payload-32bytes!";

#[derive(Debug, Clone, PartialEq)]
pub enum PingError {
    /// The host is up but something refused to forward us.
    Unreachable,
    /// No answer inside the timeout.
    TimedOut,
    /// No route at all — adapter down, no gateway.
    NoRoute,
    /// The API itself failed.
    Api(String),
}

impl PingError {
    pub fn describe(&self) -> String {
        match self {
            PingError::Unreachable => crate::i18n::ping_unreachable().into(),
            PingError::TimedOut => crate::i18n::ping_timeout().into(),
            PingError::NoRoute => crate::i18n::ping_no_route().into(),
            PingError::Api(e) => crate::i18n::ping_api_failed(&e.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PingResult {
    pub rtt_ms: Option<f64>,
    pub error: Option<PingError>,
}

impl PingResult {
    pub fn ok(&self) -> bool {
        self.rtt_ms.is_some()
    }

    fn failed(e: PingError) -> Self {
        PingResult { rtt_ms: None, error: Some(e) }
    }

    /// A probe that never ran: no handle to send it on, or the thread
    /// carrying it died. Recorded as a timeout because that is what it looks
    /// like from the outside — nothing came back — and inventing a reply
    /// would be worse than admitting to a missed sample.
    pub fn timeout() -> Self {
        PingResult::failed(PingError::TimedOut)
    }
}

/// Reply buffer for `IcmpSendEcho`.
///
/// The obvious `vec![0u8; n]` is aligned to 1, and reading an
/// `ICMP_ECHO_REPLY` back out of it is undefined behaviour however reliably
/// the allocator happens to hand out aligned blocks — the API asks for an
/// 8-byte boundary as well. Backing the buffer with `u64` puts that guarantee
/// in the type instead of in a hope.
struct ReplyBuf(Vec<u64>);

impl ReplyBuf {
    /// Room for the struct, the echoed payload, and any IP options a router
    /// tacks on.
    fn new(payload_len: usize) -> Self {
        let bytes = size_of::<ICMP_ECHO_REPLY>() + payload_len + 64;
        ReplyBuf(vec![0u64; bytes.div_ceil(size_of::<u64>())])
    }

    fn capacity_bytes(&self) -> u32 {
        (self.0.len() * size_of::<u64>()) as u32
    }

    fn as_mut_ptr(&mut self) -> *mut u64 {
        self.0.as_mut_ptr()
    }

    /// # Safety
    /// Only valid once `IcmpSendEcho` has reported at least one reply, which
    /// is what fills the buffer.
    unsafe fn reply(&self) -> &ICMP_ECHO_REPLY {
        &*(self.0.as_ptr() as *const ICMP_ECHO_REPLY)
    }
}

/// An open ICMP handle. Reusing one across pings avoids re-opening the
/// device for every sample.
pub struct Pinger {
    pub(crate) handle: HANDLE,
}

// Send, not Sync: a Pinger is moved onto the thread that uses it and the
// handle is never shared between threads. (`ping` takes `&self`, so `Sync`
// would additionally require IcmpSendEcho to tolerate concurrent calls on one
// handle — do not add it on the strength of this impl.)
unsafe impl Send for Pinger {}

impl Pinger {
    pub fn new() -> Result<Self, PingError> {
        let handle = unsafe { IcmpCreateFile() }.map_err(|e| PingError::Api(e.message()))?;
        if handle.is_invalid() {
            return Err(PingError::Api("IcmpCreateFile returned an invalid handle".into()));
        }
        Ok(Pinger { handle })
    }

    /// One echo request. `timeout` is in milliseconds.
    pub fn ping(&self, addr: Ipv4Addr, timeout_ms: u32) -> PingResult {
        let mut buf = ReplyBuf::new(PAYLOAD.len());
        let dest = u32::from_le_bytes(addr.octets());

        let started = Instant::now();
        let replies = unsafe {
            IcmpSendEcho(
                self.handle,
                dest,
                PAYLOAD.as_ptr() as *const _,
                PAYLOAD.len() as u16,
                None,
                buf.as_mut_ptr() as *mut _,
                buf.capacity_bytes(),
                timeout_ms,
            )
        };
        let elapsed = started.elapsed();

        if replies == 0 {
            // A zero count means no reply arrived; GetLastError carries why.
            let err = windows::core::Error::from_win32();
            return PingResult::failed(classify(err.code().0 as u32 & 0xFFFF, &err.message()));
        }

        let reply = unsafe { buf.reply() };
        if reply.Status != IP_SUCCESS {
            return PingResult::failed(classify(reply.Status, ""));
        }

        // The API rounds RoundTripTime down to whole milliseconds, which on a
        // LAN collapses every sample to 0 or 1. Our own measurement keeps the
        // sub-millisecond detail that makes jitter figures meaningful, but it
        // also includes the syscall overhead, so we never report less than
        // what the stack itself measured.
        let measured = elapsed.as_secs_f64() * 1000.0;
        let reported = reply.RoundTripTime as f64;
        let rtt = if measured >= reported { measured } else { reported };

        PingResult { rtt_ms: Some(rtt), error: None }
    }
}

impl Drop for Pinger {
    fn drop(&mut self) {
        unsafe {
            let _ = IcmpCloseHandle(self.handle);
        }
    }
}

fn classify(status: u32, message: &str) -> PingError {
    match status {
        IP_REQ_TIMED_OUT => PingError::TimedOut,
        x if x == WAIT_TIMEOUT.0 => PingError::TimedOut,
        IP_DEST_NET_UNREACHABLE
        | IP_DEST_HOST_UNREACHABLE
        | IP_DEST_PROT_UNREACHABLE
        | IP_DEST_PORT_UNREACHABLE
        | IP_TTL_EXPIRED_TRANSIT => PingError::Unreachable,
        IP_GENERAL_FAILURE => PingError::NoRoute,
        0 => PingError::TimedOut,
        _ => {
            if message.is_empty() {
                PingError::Api(format!("ICMP status {status}"))
            } else {
                PingError::Api(message.to_string())
            }
        }
    }
}

/// Sends one echo of `payload_len` bytes with the "don't fragment" bit set.
/// Returns true if it survived the path intact — the primitive behind the MTU
/// search, equivalent to `ping -f -l <size>`.
pub fn probe_df(addr: Ipv4Addr, payload_len: u16, timeout_ms: u32) -> bool {
    use windows::Win32::NetworkManagement::IpHelper::IP_OPTION_INFORMATION;

    let Ok(pinger) = Pinger::new() else {
        return false;
    };
    let payload = vec![0x61u8; payload_len as usize];
    let mut buf = ReplyBuf::new(payload_len as usize);
    let dest = u32::from_le_bytes(addr.octets());

    // IP_FLAG_DF = 0x02 in ipexport.h.
    let opts = IP_OPTION_INFORMATION {
        Ttl: 128,
        Tos: 0,
        Flags: 0x02,
        OptionsSize: 0,
        OptionsData: std::ptr::null_mut(),
    };

    let replies = unsafe {
        IcmpSendEcho(
            pinger.handle,
            dest,
            payload.as_ptr() as *const _,
            payload_len,
            Some(&opts),
            buf.as_mut_ptr() as *mut _,
            buf.capacity_bytes(),
            timeout_ms,
        )
    };
    if replies == 0 {
        return false;
    }
    let reply = unsafe { buf.reply() };
    reply.Status == IP_SUCCESS
}

#[derive(Debug, Clone)]
pub struct Hop {
    pub hop: u32,
    pub addr: Option<Ipv4Addr>,
    pub rtt_ms: Option<f64>,
}

/// Walks the path by sending echoes with an increasing TTL and reading which
/// router reports the expiry. Same idea as `tracert`, without the process.
pub fn traceroute(dest: Ipv4Addr, max_hops: u32, timeout_ms: u32) -> Vec<Hop> {
    use windows::Win32::NetworkManagement::IpHelper::IP_OPTION_INFORMATION;

    let Ok(pinger) = Pinger::new() else {
        return Vec::new();
    };
    let target = u32::from_le_bytes(dest.octets());
    let mut hops = Vec::new();

    for ttl in 1..=max_hops {
        let mut buf = ReplyBuf::new(PAYLOAD.len());
        let opts = IP_OPTION_INFORMATION {
            Ttl: ttl as u8,
            Tos: 0,
            Flags: 0,
            OptionsSize: 0,
            OptionsData: std::ptr::null_mut(),
        };
        let started = Instant::now();
        let replies = unsafe {
            IcmpSendEcho(
                pinger.handle,
                target,
                PAYLOAD.as_ptr() as *const _,
                PAYLOAD.len() as u16,
                Some(&opts),
                buf.as_mut_ptr() as *mut _,
                buf.capacity_bytes(),
                timeout_ms,
            )
        };
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;

        if replies == 0 {
            hops.push(Hop { hop: ttl, addr: None, rtt_ms: None });
            continue;
        }

        let reply = unsafe { buf.reply() };
        let addr = Ipv4Addr::from(reply.Address.to_le_bytes());
        hops.push(Hop { hop: ttl, addr: Some(addr), rtt_ms: Some(elapsed) });

        // Status 0 means the destination itself answered, so the path ends.
        if reply.Status == IP_SUCCESS {
            break;
        }
    }
    hops
}

/// One-off probe. Only the tests need this; everything else holds a `Pinger`
/// open across samples rather than re-opening the device each time.
#[cfg(test)]
pub fn ping_once(addr: Ipv4Addr, timeout_ms: u32) -> PingResult {
    match Pinger::new() {
        Ok(p) => p.ping(addr, timeout_ms),
        Err(e) => PingResult::failed(e),
    }
}

/// Several pings at a fixed cadence. `None` marks a lost packet.
///
/// `gap_ms` is the interval between sends, not a delay added after each reply:
/// a probe that took 40 ms to come back waits the remaining 20 of a 60 ms
/// cadence, and one that timed out waits not at all. Back-to-back probing
/// looks faster on paper and is worse, because several series running at once
/// then arrive as a burst and the measurement starts reporting its own
/// contention — a Wi-Fi link measured that way shows jitter it does not have.
pub fn ping_series(addr: Ipv4Addr, count: usize, timeout_ms: u32, gap_ms: u64) -> Vec<Option<f64>> {
    let pinger = match Pinger::new() {
        Ok(p) => p,
        Err(_) => return vec![None; count],
    };
    let gap = Duration::from_millis(gap_ms);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let started = Instant::now();
        out.push(pinger.ping(addr, timeout_ms).rtt_ms);
        if i + 1 < count {
            if let Some(rest) = gap.checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_answers_immediately() {
        let r = ping_once(Ipv4Addr::new(127, 0, 0, 1), 1000);
        assert!(r.ok(), "loopback should always reply: {:?}", r.error);
        assert!(r.rtt_ms.unwrap() < 50.0);
    }

    #[test]
    fn unrouteable_address_fails_without_hanging() {
        // 192.0.2.0/24 is reserved for documentation and is never routed.
        let r = ping_once(Ipv4Addr::new(192, 0, 2, 1), 300);
        assert!(!r.ok());
        assert!(r.error.is_some());
    }

    #[test]
    fn status_codes_map_to_causes() {
        assert_eq!(classify(IP_REQ_TIMED_OUT, ""), PingError::TimedOut);
        assert_eq!(classify(IP_DEST_HOST_UNREACHABLE, ""), PingError::Unreachable);
        assert_eq!(classify(IP_GENERAL_FAILURE, ""), PingError::NoRoute);
    }
}
