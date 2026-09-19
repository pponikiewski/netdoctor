"""Tests for the output parsers.

These are the parts most likely to break silently: Windows tools change their
wording between locales and versions, and a parser that quietly returns nothing
turns into a monitor that reports a healthy connection during an outage.

Run with:  python -m unittest discover tests
"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from netdoc import diagnose, monitor, probes                      # noqa: E402
from netdoc.settings import DEFAULTS, Settings                    # noqa: E402

PING_OK_EN = """
Pinging 1.1.1.1 with 32 bytes of data:
Reply from 1.1.1.1: bytes=32 time=12ms TTL=57

Ping statistics for 1.1.1.1:
    Packets: Sent = 1, Received = 1, Lost = 0 (0% loss),
"""

PING_OK_PL = """
Badanie 1.1.1.1 z 32 bajtami danych:
Odpowiedz z 1.1.1.1: bajtow=32 czas=8ms TTL=57
"""

PING_SUB_MS = "Reply from 192.168.50.1: bytes=32 time<1ms TTL=64"

PING_UNREACHABLE = """
Pinging 10.0.0.1 with 32 bytes of data:
Reply from 192.168.50.1: Destination host unreachable.
"""

PING_TIMEOUT = """
Pinging 1.1.1.1 with 32 bytes of data:
Request timed out.
"""

PING_NO_ROUTE = "PING: transmit failed. General failure."

WLAN_EN = """
There is 1 interface on the system:

    Name                   : WiFi
    Description            : MediaTek Wi-Fi 6 MT7921 Wireless LAN Card
    State                  : connected
    SSID                   : wi-fi dom_2G
    AP BSSID               : c8:7f:54:b0:15:44
    Radio type             : 802.11ax
    Authentication         : WPA2-Personal
    Channel                : 108
    Receive rate (Mbps)    : 1201
    Transmit rate (Mbps)   : 1201
    Signal                 : 77%
"""

WLAN_DISCONNECTED = """
    Name                   : WiFi
    State                  : disconnected
"""


class FakeRun:
    """Stands in for probes.run, returning canned output."""

    def __init__(self, output, rc=0):
        self.output, self.rc = output, rc
        self.calls = []

    def __call__(self, cmd, timeout=15):
        self.calls.append(cmd)
        return self.rc, self.output


class PingParsing(unittest.TestCase):
    def _ping(self, output):
        original = probes.run
        probes.run = FakeRun(output)
        try:
            return probes.ping("1.1.1.1")
        finally:
            probes.run = original

    def test_english_reply(self):
        r = self._ping(PING_OK_EN)
        self.assertTrue(r.ok)
        self.assertEqual(r.rtt_ms, 12.0)

    def test_polish_reply(self):
        r = self._ping(PING_OK_PL)
        self.assertTrue(r.ok)
        self.assertEqual(r.rtt_ms, 8.0)

    def test_sub_millisecond_reply_is_a_success(self):
        # "time<1ms" carries no exact figure; 1 ms is the honest upper bound.
        r = self._ping(PING_SUB_MS)
        self.assertTrue(r.ok)
        self.assertEqual(r.rtt_ms, 1.0)

    def test_unreachable_is_a_failure_despite_the_word_reply(self):
        # Windows exits 0 here, which is why the TTL is what we key on.
        r = self._ping(PING_UNREACHABLE)
        self.assertFalse(r.ok)
        self.assertIn("nieosiagalny", r.error)

    def test_timeout(self):
        self.assertFalse(self._ping(PING_TIMEOUT).ok)

    def test_no_route(self):
        r = self._ping(PING_NO_ROUTE)
        self.assertFalse(r.ok)
        self.assertIn("offline", r.error)

    def test_empty_host(self):
        self.assertFalse(probes.ping("").ok)


class WifiParsing(unittest.TestCase):
    def _state(self, output):
        original = probes.run
        probes.run = FakeRun(output)
        try:
            st = probes.NetState()
            return probes.fill_wifi(st)
        finally:
            probes.run = original

    def test_fields(self):
        st = self._state(WLAN_EN)
        self.assertEqual(st.ssid, "wi-fi dom_2G")
        self.assertEqual(st.bssid, "c8:7f:54:b0:15:44")   # netsh says "AP BSSID"
        self.assertEqual(st.signal_pct, 77)
        self.assertEqual(st.channel, "108")
        self.assertEqual(st.radio, "802.11ax")
        self.assertEqual(st.rx_rate, "1201")

    def test_connected_flag(self):
        original = probes.run
        probes.run = FakeRun(WLAN_EN)
        try:
            self.assertTrue(probes.wifi_connected())
        finally:
            probes.run = original

    def test_disconnected_is_not_read_as_connected(self):
        original = probes.run
        probes.run = FakeRun(WLAN_DISCONNECTED)
        try:
            self.assertFalse(probes.wifi_connected())
        finally:
            probes.run = original


class GatewayParsing(unittest.TestCase):
    def test_default_gateway_and_ipv4(self):
        text = """
   IPv4 Address. . . . . . . . . . . : 192.168.50.23(Preferred)
   Default Gateway . . . . . . . . . : 192.168.50.1
"""
        self.assertEqual(probes._GW_RE.search(text).group(1), "192.168.50.1")
        self.assertEqual(probes._IP_RE.search(text).group(1), "192.168.50.23")

    def test_empty_gateway_is_ignored(self):
        text = "   Default Gateway . . . . . . . . . :\n"
        self.assertIsNone(probes._GW_RE.search(text))


class Classification(unittest.TestCase):
    """The core logic: who is to blame for a dropout."""

    def setUp(self):
        class FakeStore:
            def stats(self, *a):
                return (10, 0.0, 12.0, 9.0, 15.0, 2.0)

        self.m = monitor.Monitor(FakeStore())
        self.m.net = probes.NetState(gateway="192.168.50.1", adapter_type="Wi-Fi",
                                     adapter="WiFi", dns_servers=["192.168.50.1"])

    def test_all_up(self):
        status, _ = self.m._classify({"gateway": (True, 2.0), "cloudflare": (True, 12.0),
                                      "google": (True, 14.0)})
        self.assertEqual(status, monitor.OK)

    def test_router_answers_internet_does_not_blames_the_isp(self):
        status, note = self.m._classify({"gateway": (True, 2.0), "cloudflare": (False, None),
                                         "google": (False, None)})
        self.assertEqual(status, monitor.ISP_DOWN)
        self.assertIn("Router odpowiada", note)

    def test_nothing_answers_blames_the_local_link(self):
        original = probes.wifi_connected
        probes.wifi_connected = lambda: True
        try:
            status, _ = self.m._classify({"gateway": (False, None), "cloudflare": (False, None),
                                          "google": (False, None)})
        finally:
            probes.wifi_connected = original
        self.assertEqual(status, monitor.LAN_DOWN)

    def test_disconnected_card_is_reported_as_such(self):
        original = probes.wifi_connected
        probes.wifi_connected = lambda: False
        try:
            status, _ = self.m._classify({"gateway": (False, None), "cloudflare": (False, None)})
        finally:
            probes.wifi_connected = original
        self.assertEqual(status, monitor.ADAPTER_DOWN)

    def test_working_ping_with_broken_names_is_a_dns_fault(self):
        self.m._dns_error = "getaddrinfo failed"
        status, _ = self.m._classify({"gateway": (True, 2.0), "cloudflare": (True, 12.0)})
        self.assertEqual(status, monitor.DNS_FAIL)

    def test_isp_dns_probe_is_skipped_when_it_is_the_router(self):
        keys = [k for k, _, _ in self.m._targets()]
        self.assertNotIn("dns_isp", keys)
        self.assertIn("gateway", keys)


class SettingsBehaviour(unittest.TestCase):
    def test_unknown_keys_from_an_older_file_are_ignored(self):
        import json
        import tempfile

        path = Path(tempfile.mkdtemp()) / "settings.json"
        path.write_text(json.dumps({"ping_ok": 45, "obsolete_key": 1}), encoding="utf-8")
        s = Settings(path)
        self.assertEqual(s.ping_ok, 45)
        self.assertEqual(s.ping_bad, DEFAULTS["ping_bad"])
        self.assertFalse(hasattr(s._data, "obsolete_key"))

    def test_extra_targets_become_probe_targets(self):
        import tempfile

        s = Settings(Path(tempfile.mkdtemp()) / "settings.json")
        s.extra_targets = ["8.8.4.4", "  ", "game.example.com"]
        hosts = [t["host"] for t in s.targets()]
        self.assertIn("8.8.4.4", hosts)
        self.assertIn("game.example.com", hosts)
        self.assertNotIn("  ", hosts)


class Bands(unittest.TestCase):
    def test_channel_to_band(self):
        self.assertEqual(diagnose._wifi_band(6), "2.4 GHz")
        self.assertEqual(diagnose._wifi_band("108"), "5 GHz")
        self.assertEqual(diagnose._wifi_band(""), "")


if __name__ == "__main__":
    unittest.main()
