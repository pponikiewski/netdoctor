"""Tweaks: each one can be inspected, applied and reverted.

Nothing here runs by itself. Every tweak records the previous value to
tweak_snapshots.json before touching anything, so "Cofnij" restores the exact
state the machine was in — including after a reboot.
"""

import json
import re

from . import config, probes

RISK_LOW = "niskie"
RISK_MEDIUM = "średnie"
RISK_HIGH = "wysokie"


class Tweak:
    """Base class. Subclasses implement read_current/do_apply/do_revert."""

    id = "base"
    title = ""
    what = ""            # what it changes
    why = ""             # why it helps with ping / dropouts
    risk = RISK_LOW
    needs_admin = True
    reversible = True

    def __init__(self, net=None):
        self.net = net or probes.NetState()

    # -- to override ------------------------------------------------------
    def read_current(self):
        """Return (human_text, is_already_optimal, raw_value_for_snapshot)."""
        raise NotImplementedError

    def do_apply(self):
        raise NotImplementedError

    def do_revert(self, snapshot):
        raise NotImplementedError

    # -- shared -----------------------------------------------------------
    def status(self):
        try:
            return self.read_current()
        except Exception as e:
            return ("nie udało się odczytać: %s" % e, None, None)


# ---------------------------------------------------------------------------
# snapshot storage
# ---------------------------------------------------------------------------

def _load_snapshots():
    try:
        with open(config.SNAPSHOT_PATH, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def _save_snapshots(data):
    config.SNAPSHOT_PATH.parent.mkdir(parents=True, exist_ok=True)
    with open(config.SNAPSHOT_PATH, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)


def saved_snapshot(tweak_id):
    return _load_snapshots().get(tweak_id)


def apply_tweak(tweak, store=None):
    """Snapshot -> apply -> log. Returns (ok, message)."""
    if tweak.needs_admin and not probes.is_admin():
        return False, "Ta zmiana wymaga uprawnień administratora."
    text, optimal, raw = tweak.status()
    snaps = _load_snapshots()
    snaps[tweak.id] = {"value": raw, "text": text}
    _save_snapshots(snaps)
    try:
        ok, msg = tweak.do_apply()
    except Exception as e:
        ok, msg = False, "błąd: %s" % e
    if store:
        store.log_tweak(tweak.id, "apply", raw, ("OK: " if ok else "BŁĄD: ") + msg)
    return ok, msg


def revert_tweak(tweak, store=None):
    if tweak.needs_admin and not probes.is_admin():
        return False, "Cofnięcie wymaga uprawnień administratora."
    snap = saved_snapshot(tweak.id)
    if snap is None:
        return False, "Brak zapisanego stanu sprzed zmiany — nie ma czego cofać."
    try:
        ok, msg = tweak.do_revert(snap.get("value"))
    except Exception as e:
        ok, msg = False, "błąd: %s" % e
    if store:
        store.log_tweak(tweak.id, "revert", snap.get("value"), ("OK: " if ok else "BŁĄD: ") + msg)
    if ok:
        snaps = _load_snapshots()
        snaps.pop(tweak.id, None)
        _save_snapshots(snaps)
    return ok, msg


# ---------------------------------------------------------------------------
# individual tweaks
# ---------------------------------------------------------------------------

class WifiPowerSaving(Tweak):
    id = "wifi_power"
    title = "Wyłącz oszczędzanie energii karty sieciowej"
    what = "Odznacza „Zezwalaj komputerowi na wyłączanie tego urządzenia w celu oszczędzania energii”."
    why = ("Najczęstsza przyczyna nagłych zerwań na Wi-Fi w laptopach. Windows usypia "
           "kartę w bezczynności, a wybudzenie trwa na tyle długo, że połączenia padają.")
    risk = RISK_LOW

    # Bit 24 of PnPCapabilities tells Windows the device may not be powered
    # down; bit 8+16 hide the checkboxes entirely. 24 (0x18) is the value the
    # "device manager" checkbox writes.
    PNP_DISABLE = 24

    def _name(self):
        return self.net.adapter or ""

    def _class_key(self):
        return probes.adapter_class_key(probes.adapter_guid(self._name()))

    def _read_pnp(self, key):
        rc, out = probes.run(["reg", "query", key, "/v", "PnPCapabilities"], timeout=15)
        m = re.search(r"PnPCapabilities\s+REG_DWORD\s+0x([0-9a-f]+)", out, re.I)
        return int(m.group(1), 16) if m else None

    def read_current(self):
        name = self._name()
        if not name:
            return ("nie wykryto aktywnej karty", None, None)

        # Preferred path: the driver exposes the setting through WMI.
        rc, out = probes.powershell(
            "$p = Get-NetAdapterPowerManagement -Name '%s' -ErrorAction SilentlyContinue;"
            " if ($p) { 'ALLOW=' + $p.AllowComputerToTurnOffDevice }" % name
        )
        m = re.search(r"ALLOW=(\w+)", out)
        if m and m.group(1) in ("Enabled", "Disabled"):
            val = m.group(1)
            if val == "Enabled":
                return ("włączone — Windows może usypiać kartę", False, {"mode": "wmi", "val": val})
            return ("wyłączone", True, {"mode": "wmi", "val": val})

        # MediaTek and some Realtek drivers do not implement that class, so
        # fall back to the registry value the checkbox actually writes.
        key = self._class_key()
        if not key:
            return ("nie znaleziono karty w rejestrze", None, None)
        pnp = self._read_pnp(key)
        applied = pnp is not None and (pnp & self.PNP_DISABLE) == self.PNP_DISABLE
        txt = ("wyłączone (PnPCapabilities=%s)" % pnp) if applied else (
            "włączone — Windows może usypiać kartę (PnPCapabilities=%s)"
            % ("brak" if pnp is None else pnp))
        return (txt, applied, {"mode": "reg", "key": key, "val": pnp})

    def do_apply(self):
        name = self._name()
        rc, out = probes.powershell(
            "$p = Get-NetAdapterPowerManagement -Name '%s' -ErrorAction SilentlyContinue;"
            " if ($p) { $p.AllowComputerToTurnOffDevice = 'Disabled';"
            " Set-NetAdapterPowerManagement -InputObject $p -ErrorAction Stop; 'DONE' }" % name
        )
        if "DONE" in out:
            return True, "Oszczędzanie energii karty wyłączone."

        key = self._class_key()
        if not key:
            return False, "Nie znaleziono klucza karty w rejestrze."
        rc, out = probes.run(["reg", "add", key, "/v", "PnPCapabilities",
                              "/t", "REG_DWORD", "/d", str(self.PNP_DISABLE), "/f"], timeout=15)
        if rc != 0:
            return False, out.strip()[:300] or "zapis do rejestru nie powiódł się"
        return True, ("Oszczędzanie energii karty wyłączone w rejestrze. "
                      "Zadziała po restarcie komputera lub wyłączeniu i włączeniu karty.")

    def do_revert(self, snapshot):
        snapshot = snapshot or {}
        if snapshot.get("mode") == "wmi":
            val = snapshot.get("val") or "Enabled"
            rc, out = probes.powershell(
                "$p = Get-NetAdapterPowerManagement -Name '%s';"
                " $p.AllowComputerToTurnOffDevice = '%s';"
                " Set-NetAdapterPowerManagement -InputObject $p -ErrorAction Stop; 'DONE'"
                % (self._name(), val)
            )
            return ("DONE" in out), ("Przywrócono '%s'." % val if "DONE" in out
                                     else out.strip()[:300])
        key = snapshot.get("key") or self._class_key()
        if not key:
            return False, "Nie znaleziono klucza karty w rejestrze."
        old = snapshot.get("val")
        if old is None:
            probes.run(["reg", "delete", key, "/v", "PnPCapabilities", "/f"], timeout=15)
        else:
            probes.run(["reg", "add", key, "/v", "PnPCapabilities", "/t", "REG_DWORD",
                        "/d", str(old), "/f"], timeout=15)
        return True, "Przywrócono poprzednią wartość. Wymaga restartu komputera."


class WlanPowerPlan(Tweak):
    id = "wlan_power_plan"
    title = "Tryb maksymalnej wydajności Wi-Fi w planie zasilania"
    what = "Ustawia „Wireless Adapter Settings -> Power Saving Mode” na Maximum Performance (bateria i sieć)."
    why = ("Drugi, niezależny mechanizm usypiania radia. Nawet z wyłączonym "
           "oszczędzaniem w sterowniku plan zasilania potrafi ciąć moc nadajnika.")
    risk = RISK_LOW

    SUB = "19cbb8fa-5279-450e-9fac-8a3d5fedd0c1"      # Wireless Adapter Settings
    SETTING = "12bbebe6-58d6-4636-95bb-3217ef867c1a"  # Power Saving Mode

    def read_current(self):
        rc, out = probes.run(["powercfg", "/query", "SCHEME_CURRENT", self.SUB, self.SETTING], timeout=20)
        ac = re.search(r"Current AC Power Setting Index:\s*0x([0-9a-f]+)", out, re.I)
        dc = re.search(r"Current DC Power Setting Index:\s*0x([0-9a-f]+)", out, re.I)
        if not ac:
            return ("brak tego ustawienia w planie zasilania", None, None)
        vals = (int(ac.group(1), 16), int(dc.group(1), 16) if dc else 0)
        names = {0: "maks. wydajność", 1: "niskie oszczędzanie", 2: "średnie", 3: "maks. oszczędzanie"}
        txt = "zasilanie: %s / bateria: %s" % (names.get(vals[0], vals[0]), names.get(vals[1], vals[1]))
        return (txt, vals == (0, 0), list(vals))

    def do_apply(self):
        probes.run(["powercfg", "/setacvalueindex", "SCHEME_CURRENT", self.SUB, self.SETTING, "0"])
        probes.run(["powercfg", "/setdcvalueindex", "SCHEME_CURRENT", self.SUB, self.SETTING, "0"])
        rc, out = probes.run(["powercfg", "/setactive", "SCHEME_CURRENT"])
        return rc == 0, "Plan zasilania: radio Wi-Fi na maksymalnej wydajności."

    def do_revert(self, snapshot):
        ac, dc = (snapshot or [0, 3])[0], (snapshot or [0, 3])[1]
        probes.run(["powercfg", "/setacvalueindex", "SCHEME_CURRENT", self.SUB, self.SETTING, str(ac)])
        probes.run(["powercfg", "/setdcvalueindex", "SCHEME_CURRENT", self.SUB, self.SETTING, str(dc)])
        rc, out = probes.run(["powercfg", "/setactive", "SCHEME_CURRENT"])
        return rc == 0, "Przywrócono poprzednie ustawienia planu zasilania."


class FastDns(Tweak):
    id = "fast_dns"
    title = "Szybkie, niezależne serwery DNS"
    what = "Ustawia 1.1.1.1 i 8.8.8.8 na aktywnej karcie zamiast serwerów z DHCP."
    why = ("Jeśli jedynym DNS-em jest router, jego zawieszka wygląda dokładnie jak "
           "„padł internet”: ping po IP chodzi, a nic się nie otwiera.")
    risk = RISK_LOW

    def read_current(self):
        cur = self.net.dns_servers or []
        good = {"1.1.1.1", "1.0.0.1", "8.8.8.8", "8.8.4.4", "9.9.9.9"}
        txt = ", ".join(cur) if cur else "brak / z DHCP"
        only_router = bool(cur) and all(d == self.net.gateway for d in cur)
        if only_router:
            txt += "  (tylko router — pojedynczy punkt awarii)"
        return (txt, bool(cur) and any(d in good for d in cur), cur)

    def do_apply(self):
        name = self.net.adapter
        rc, out = probes.powershell(
            "Set-DnsClientServerAddress -InterfaceAlias '%s'"
            " -ServerAddresses ('1.1.1.1','8.8.8.8') -ErrorAction Stop;"
            " Clear-DnsClientCache; 'DONE'" % name
        )
        return ("DONE" in out), ("DNS ustawione na 1.1.1.1 i 8.8.8.8."
                                 if "DONE" in out else out.strip()[:300])

    def do_revert(self, snapshot):
        name = self.net.adapter
        # An empty snapshot means the interface was on DHCP.
        if not snapshot:
            rc, out = probes.powershell(
                "Set-DnsClientServerAddress -InterfaceAlias '%s' -ResetServerAddresses"
                " -ErrorAction Stop; Clear-DnsClientCache; 'DONE'" % name
            )
            return ("DONE" in out), "Przywrócono DNS z DHCP."
        addrs = ",".join("'%s'" % d for d in snapshot)
        rc, out = probes.powershell(
            "Set-DnsClientServerAddress -InterfaceAlias '%s' -ServerAddresses (%s)"
            " -ErrorAction Stop; Clear-DnsClientCache; 'DONE'" % (name, addrs)
        )
        return ("DONE" in out), "Przywrócono poprzednie serwery DNS."


class TcpAutotuning(Tweak):
    id = "tcp_autotuning"
    title = "TCP Receive Window Auto-Tuning = normal"
    what = "netsh int tcp set global autotuninglevel=normal"
    why = ("Poradniki „na ping” każą to wyłączać, co psuje przepustowość. "
           "Wartość normal jest poprawna — ten tweak naprawia wcześniejsze majstrowanie.")
    risk = RISK_LOW

    def read_current(self):
        rc, out = probes.run(["netsh", "int", "tcp", "show", "global"], timeout=20)
        m = re.search(r"Auto-Tuning Level\s*:\s*(\w+)", out, re.I)
        val = m.group(1).lower() if m else None
        if val is None:
            return ("nie udało się odczytać", None, None)
        return (val, val == "normal", val)

    def do_apply(self):
        rc, out = probes.run(["netsh", "int", "tcp", "set", "global", "autotuninglevel=normal"])
        return "Ok" in out or rc == 0, "Auto-tuning ustawiony na normal."

    def do_revert(self, snapshot):
        val = snapshot or "normal"
        rc, out = probes.run(["netsh", "int", "tcp", "set", "global", "autotuninglevel=%s" % val])
        return rc == 0, "Przywrócono auto-tuning: %s." % val


class NagleOff(Tweak):
    id = "nagle_off"
    title = "Wyłącz algorytm Nagle'a (dla gier)"
    what = "Dopisuje TcpAckFrequency=1 i TCPNoDelay=1 do interfejsu w rejestrze."
    why = ("Windows domyślnie buforuje małe pakiety i zwleka z ACK. W grach FPS/MOBA "
           "to kilka–kilkanaście ms dodatkowego opóźnienia. Dla pobierania bez znaczenia.")
    risk = RISK_MEDIUM

    def _iface_key(self):
        """Per-interface TCP/IP key, located by the adapter's GUID."""
        return probes.tcpip_interface_key(probes.adapter_guid(self.net.adapter))

    def read_current(self):
        key = self._iface_key()
        if not key:
            return ("nie znaleziono wpisu karty w rejestrze", None, None)
        vals = {}
        for name in ("TcpAckFrequency", "TCPNoDelay"):
            rc, out = probes.run(["reg", "query", key, "/v", name], timeout=15)
            m = re.search(name + r"\s+REG_DWORD\s+0x([0-9a-f]+)", out, re.I)
            vals[name] = int(m.group(1), 16) if m else None
        applied = vals["TcpAckFrequency"] == 1 and vals["TCPNoDelay"] == 1
        txt = "TcpAckFrequency=%s, TCPNoDelay=%s" % (
            vals["TcpAckFrequency"] if vals["TcpAckFrequency"] is not None else "brak",
            vals["TCPNoDelay"] if vals["TCPNoDelay"] is not None else "brak",
        )
        return (txt, applied, {"key": key, "vals": vals})

    def do_apply(self):
        key = self._iface_key()
        if not key:
            return False, "Nie znaleziono klucza karty w rejestrze."
        for name in ("TcpAckFrequency", "TCPNoDelay"):
            probes.run(["reg", "add", key, "/v", name, "/t", "REG_DWORD", "/d", "1", "/f"], timeout=15)
        return True, "Nagle wyłączony. Wymaga restartu komputera."

    def do_revert(self, snapshot):
        snapshot = snapshot or {}
        key = snapshot.get("key") or self._iface_key()
        if not key:
            return False, "Nie znaleziono klucza karty w rejestrze."
        vals = snapshot.get("vals", {})
        for name in ("TcpAckFrequency", "TCPNoDelay"):
            old = vals.get(name)
            if old is None:
                probes.run(["reg", "delete", key, "/v", name, "/f"], timeout=15)
            else:
                probes.run(["reg", "add", key, "/v", name, "/t", "REG_DWORD",
                            "/d", str(old), "/f"], timeout=15)
        return True, "Przywrócono poprzedni stan. Wymaga restartu komputera."


class NetworkThrottling(Tweak):
    id = "net_throttling"
    title = "Zdejmij limit pakietów multimediów"
    what = "NetworkThrottlingIndex = 0xffffffff, SystemResponsiveness = 10."
    why = ("Windows ogranicza ruch sieciowy do ~10 tys. pakietów/s, gdy działa "
           "odtwarzanie multimediów. Przy grze i streamie jednocześnie to widać jako lag.")
    risk = RISK_MEDIUM

    KEY = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile"

    def _read(self, name):
        rc, out = probes.run(["reg", "query", self.KEY, "/v", name], timeout=15)
        m = re.search(name + r"\s+REG_DWORD\s+0x([0-9a-f]+)", out, re.I)
        return int(m.group(1), 16) if m else None

    def read_current(self):
        nti = self._read("NetworkThrottlingIndex")
        sr = self._read("SystemResponsiveness")
        applied = nti == 0xFFFFFFFF
        txt = "NetworkThrottlingIndex=%s, SystemResponsiveness=%s" % (
            hex(nti) if nti is not None else "domyślne (10)",
            sr if sr is not None else "domyślne (20)",
        )
        return (txt, applied, {"nti": nti, "sr": sr})

    def do_apply(self):
        probes.run(["reg", "add", self.KEY, "/v", "NetworkThrottlingIndex",
                    "/t", "REG_DWORD", "/d", "4294967295", "/f"], timeout=15)
        probes.run(["reg", "add", self.KEY, "/v", "SystemResponsiveness",
                    "/t", "REG_DWORD", "/d", "10", "/f"], timeout=15)
        return True, "Limit zdjęty. Wymaga restartu komputera."

    def do_revert(self, snapshot):
        snapshot = snapshot or {}
        for name, key in (("NetworkThrottlingIndex", "nti"), ("SystemResponsiveness", "sr")):
            old = snapshot.get(key)
            if old is None:
                probes.run(["reg", "delete", self.KEY, "/v", name, "/f"], timeout=15)
            else:
                probes.run(["reg", "add", self.KEY, "/v", name, "/t", "REG_DWORD",
                            "/d", str(old), "/f"], timeout=15)
        return True, "Przywrócono domyślne. Wymaga restartu komputera."


class MtuFix(Tweak):
    id = "mtu"
    title = "Popraw MTU karty"
    what = "Ustawia MTU na wartość wyznaczoną testem fragmentacji (zwykle 1500 lub 1492 na PPPoE)."
    why = ("Za duże MTU powoduje odrzucanie pakietów gdzieś po drodze — objawia się to "
           "jako „strona się nie ładuje”, choć ping chodzi.")
    risk = RISK_MEDIUM

    def probe_best_mtu(self, host="1.1.1.1"):
        """Binary search on the largest unfragmented payload."""
        lo, hi, best = 1200, 1472, None
        while lo <= hi:
            mid = (lo + hi) // 2
            rc, out = probes.run(
                ["ping", host, "-f", "-l", str(mid), "-n", "1", "-w", "1500"], timeout=8
            )
            low = out.lower()
            if "ttl=" in low and "fragment" not in low:
                best = mid
                lo = mid + 1
            else:
                hi = mid - 1
        return (best + 28) if best else None

    def read_current(self):
        name = self.net.adapter
        rc, out = probes.run(["netsh", "interface", "ipv4", "show", "subinterfaces"], timeout=20)
        cur = None
        for line in out.splitlines():
            if name and name.lower() in line.lower():
                m = re.match(r"\s*(\d+)", line)
                if m:
                    cur = int(m.group(1))
        return ("MTU = %s" % (cur if cur else "nieznane"), None, cur)

    def do_apply(self):
        best = self.probe_best_mtu()
        if not best:
            return False, "Test MTU nie dał wyniku (ping z flagą -f zablokowany?)."
        rc, out = probes.run([
            "netsh", "interface", "ipv4", "set", "subinterface",
            self.net.adapter, "mtu=%d" % best, "store=persistent"
        ], timeout=20)
        ok = rc == 0 or "Ok" in out
        return ok, ("MTU ustawione na %d." % best) if ok else out.strip()[:300]

    def do_revert(self, snapshot):
        val = snapshot or 1500
        rc, out = probes.run([
            "netsh", "interface", "ipv4", "set", "subinterface",
            self.net.adapter, "mtu=%d" % val, "store=persistent"
        ], timeout=20)
        return (rc == 0 or "Ok" in out), "Przywrócono MTU %d." % val


class StackReset(Tweak):
    id = "stack_reset"
    title = "Reset stosu sieciowego (akcja naprawcza)"
    what = "ipconfig /flushdns, /release, /renew, netsh winsock reset, netsh int ip reset."
    why = ("Do użycia gdy net już padł i nie wraca. Czyści zepsuty stan Winsock/IP, "
           "który potrafi trzymać się do restartu.")
    risk = RISK_MEDIUM
    reversible = False

    def read_current(self):
        return ("akcja jednorazowa — nic nie jest trwale zmieniane", None, None)

    def do_apply(self):
        steps = [
            ["ipconfig", "/flushdns"],
            ["ipconfig", "/release"],
            ["ipconfig", "/renew"],
            ["netsh", "winsock", "reset"],
            ["netsh", "int", "ip", "reset"],
        ]
        done = []
        for cmd in steps:
            rc, out = probes.run(cmd, timeout=60)
            done.append("%s (%s)" % (cmd[-1], "ok" if rc == 0 else "błąd"))
        return True, "Wykonano: " + ", ".join(done) + ". Zalecany restart."

    def do_revert(self, snapshot):
        return False, "Tej akcji nie da się cofnąć."


def all_tweaks(net):
    return [
        WifiPowerSaving(net),
        WlanPowerPlan(net),
        FastDns(net),
        TcpAutotuning(net),
        NagleOff(net),
        NetworkThrottling(net),
        MtuFix(net),
        StackReset(net),
    ]
