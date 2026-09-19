"""Low level probes: ping, DNS timing, adapter and Wi-Fi state.

Everything shells out to Windows tools so the app needs no admin rights and no
third party packages. All subprocess calls are hidden (no console flash).
"""

import ctypes
import re
import socket
import subprocess
import time
from dataclasses import dataclass, field

from . import config

_NO_WINDOW = 0x08000000


def run(cmd, timeout=15):
    """Run a command, return (rc, combined output). Never raises."""
    try:
        p = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            creationflags=_NO_WINDOW,
            errors="replace",
        )
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except subprocess.TimeoutExpired:
        return -1, "<timeout>"
    except OSError as e:
        return -1, "<error: %s>" % e


def powershell(script, timeout=25):
    return run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
        timeout=timeout,
    )


def is_admin():
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:
        return False


# --------------------------------------------------------------------------
# ping
# --------------------------------------------------------------------------

# Matches "time=12ms", "time<1ms" and the localised "czas=12ms".
_RTT_RE = re.compile(r"(?:time|czas)[=<]\s*(\d+)\s*ms", re.IGNORECASE)
_TTL_RE = re.compile(r"TTL[=\s]*(\d+)", re.IGNORECASE)


@dataclass
class PingResult:
    host: str
    ok: bool
    rtt_ms: float | None = None
    error: str = ""


def ping(host, timeout_ms=config.PING_TIMEOUT_MS):
    """Single ICMP echo via the system ping tool."""
    if not host:
        return PingResult(host or "?", False, error="brak adresu")
    rc, out = run(
        ["ping", "-n", "1", "-w", str(timeout_ms), host],
        timeout=(timeout_ms / 1000.0) + 4,
    )
    # A real reply always carries a TTL. Windows returns rc 0 even for
    # "Destination host unreachable", so the return code alone lies.
    if _TTL_RE.search(out):
        m = _RTT_RE.search(out)
        return PingResult(host, True, float(m.group(1)) if m else 0.0)
    low = out.lower()
    if "unreachable" in low or "nieosi" in low:
        return PingResult(host, False, error="host nieosiagalny")
    if "transmit failed" in low or "general failure" in low:
        return PingResult(host, False, error="brak trasy / karta offline")
    return PingResult(host, False, error="brak odpowiedzi")


def ping_series(host, count=10, timeout_ms=config.PING_TIMEOUT_MS):
    """Several pings in a row -> list of rtt values (None for a lost packet)."""
    out = []
    for _ in range(count):
        r = ping(host, timeout_ms)
        out.append(r.rtt_ms if r.ok else None)
    return out


# --------------------------------------------------------------------------
# DNS
# --------------------------------------------------------------------------

def dns_lookup_ms(domain=config.DNS_TEST_DOMAIN):
    """Time a DNS resolution. Returns (milliseconds, error)."""
    t0 = time.perf_counter()
    try:
        socket.getaddrinfo(domain, None)
        return (time.perf_counter() - t0) * 1000, ""
    except socket.gaierror as e:
        return None, str(e)


# --------------------------------------------------------------------------
# adapter / routing state
# --------------------------------------------------------------------------

@dataclass
class NetState:
    gateway: str | None = None
    dns_servers: list = field(default_factory=list)
    adapter: str = ""
    adapter_desc: str = ""
    adapter_type: str = ""          # "Wi-Fi" | "Ethernet" | ""
    link_speed: str = ""
    local_ip: str = ""
    # Wi-Fi only
    ssid: str = ""
    bssid: str = ""
    signal_pct: int | None = None
    channel: str = ""
    radio: str = ""
    rx_rate: str = ""
    tx_rate: str = ""
    auth: str = ""


_GW_RE = re.compile(r"(?:Default Gateway|Brama domy[^:]*)[ .]*:\s*(\d{1,3}(?:\.\d{1,3}){3})")
_IP_RE = re.compile(r"IPv4[^:]*:\s*(\d{1,3}(?:\.\d{1,3}){3})")

_PS_STATE = (
    "$r = Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue |"
    " Sort-Object RouteMetric | Select-Object -First 1;"
    " if ($r) {"
    " $a = Get-NetAdapter -InterfaceIndex $r.InterfaceIndex -ErrorAction SilentlyContinue;"
    " $d = (Get-DnsClientServerAddress -InterfaceIndex $r.InterfaceIndex"
    " -AddressFamily IPv4 -ErrorAction SilentlyContinue).ServerAddresses;"
    " 'NAME=' + $a.Name; 'DESC=' + $a.InterfaceDescription;"
    " 'MEDIA=' + $a.MediaType; 'SPEED=' + $a.LinkSpeed;"
    " 'GW=' + $r.NextHop; 'DNS=' + ($d -join ',') }"
)


def read_net_state():
    """Snapshot of the active connection."""
    st = NetState()

    rc, out = run(["ipconfig", "/all"], timeout=15)
    m = _GW_RE.search(out)
    if m and m.group(1) != "0.0.0.0":
        st.gateway = m.group(1)
    m = _IP_RE.search(out)
    if m:
        st.local_ip = m.group(1)

    rc, out = powershell(_PS_STATE, timeout=25)
    fields = {}
    for line in out.splitlines():
        if "=" in line:
            k, _, v = line.partition("=")
            fields[k.strip()] = v.strip()
    st.adapter = fields.get("NAME", "")
    st.adapter_desc = fields.get("DESC", "")
    st.link_speed = fields.get("SPEED", "")
    if fields.get("GW") and fields["GW"] != "0.0.0.0":
        st.gateway = fields["GW"]
    if fields.get("DNS"):
        st.dns_servers = [d for d in fields["DNS"].split(",") if d]

    media = (fields.get("MEDIA", "") + " " + st.adapter_desc + " " + st.adapter).lower()
    if "802.11" in media or "wireless" in media or "wi-fi" in media or "wifi" in media:
        st.adapter_type = "Wi-Fi"
    elif media.strip():
        st.adapter_type = "Ethernet"

    if st.adapter_type == "Wi-Fi":
        fill_wifi(st)
    return st


def fill_wifi(st):
    rc, out = run(["netsh", "wlan", "show", "interfaces"], timeout=15)
    for line in out.splitlines():
        if ":" not in line:
            continue
        key, _, val = line.partition(":")
        key = key.strip().lower()
        val = val.strip()
        if key == "ssid":
            st.ssid = val
        elif key.endswith("bssid"):      # netsh prints it as "AP BSSID"
            st.bssid = val
        elif key.startswith("signal") or key.startswith("sygna"):
            m = re.search(r"(\d+)", val)
            if m:
                st.signal_pct = int(m.group(1))
        elif key.startswith("channel") or key.startswith("kana"):
            st.channel = val
        elif "radio" in key:
            st.radio = val
        elif key.startswith("receive") or key.startswith("szybkosc odb"):
            st.rx_rate = val
        elif key.startswith("transmit"):
            st.tx_rate = val
        elif key.startswith("authentication") or key.startswith("uwierz"):
            st.auth = val
    return st


def wifi_connected():
    rc, out = run(["netsh", "wlan", "show", "interfaces"], timeout=10)
    low = out.lower()
    if "disconnected" in low or "rozlaczony" in low:
        return False
    return "connected" in low or "polaczony" in low


def wifi_neighbours(ssid=None):
    """Nearby networks with their channels, for co-channel interference checks."""
    rc, out = run(["netsh", "wlan", "show", "networks", "mode=bssid"], timeout=25)
    nets = []
    cur = None
    for line in out.splitlines():
        s = line.strip()
        if s.lower().startswith("ssid ") and ":" in s:
            cur = {"ssid": s.split(":", 1)[1].strip(), "channels": [], "signals": []}
            nets.append(cur)
        elif cur is not None and (s.lower().startswith("channel") or s.lower().startswith("kana")):
            m = re.search(r"(\d+)", s)
            if m:
                cur["channels"].append(int(m.group(1)))
        elif cur is not None and (s.lower().startswith("signal") or s.lower().startswith("sygna")):
            m = re.search(r"(\d+)", s)
            if m:
                cur["signals"].append(int(m.group(1)))
    return nets


def traceroute(host="1.1.1.1", max_hops=15):
    """Returns a list of (hop_no, address, times_text)."""
    rc, out = run(["tracert", "-d", "-h", str(max_hops), "-w", "800", host], timeout=120)
    hops = []
    for line in out.splitlines():
        m = re.match(r"\s*(\d+)\s+(.*)", line)
        if not m:
            continue
        rest = m.group(2).strip()
        a = re.search(r"(\d{1,3}(?:\.\d{1,3}){3})", rest)
        times = " ".join(re.findall(r"<?\s?\d+\s*ms|\*", rest))
        hops.append((int(m.group(1)), a.group(1) if a else "*", times))
    return hops


# --------------------------------------------------------------------------
# registry helpers (needed by the tweaks)
# --------------------------------------------------------------------------

NET_CLASS_KEY = (
    r"HKLM\SYSTEM\CurrentControlSet\Control\Class"
    r"\{4d36e972-e325-11ce-bfc1-08002be10318}"
)


def adapter_guid(name):
    """InterfaceGuid of a named adapter, e.g. '{A1B2...}'."""
    if not name:
        return None
    rc, out = powershell(
        "(Get-NetAdapter -Name '%s' -ErrorAction SilentlyContinue).InterfaceGuid" % name
    )
    m = re.search(r"\{[0-9A-Fa-f-]{36}\}", out)
    return m.group(0) if m else None


def adapter_class_key(guid):
    r"""Driver class subkey (…\Class\{4d36e972-…}\0012) for an adapter GUID."""
    if not guid:
        return None
    rc, out = run(["reg", "query", NET_CLASS_KEY, "/s", "/v", "NetCfgInstanceId"], timeout=40)
    cur = None
    for line in out.splitlines():
        s = line.strip()
        if s.upper().startswith("HKEY_"):
            cur = s
        elif guid.lower() in s.lower() and cur:
            return cur
    return None


def tcpip_interface_key(guid):
    """Per-interface TCP/IP parameters key for an adapter GUID."""
    if not guid:
        return None
    return (r"HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters"
            r"\Interfaces\%s" % guid)
