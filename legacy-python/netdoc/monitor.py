"""Background monitor.

Pings the router and several internet endpoints once per second, in parallel,
and turns the pattern of failures into a verdict about *where* the connection
broke. That distinction is the whole point: "internet wywala" means something
different when the router still answers than when it does not.
"""

import threading
import time
from collections import deque
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field

from . import probes
from .settings import S

# Connection verdicts, worst last.
OK = "ok"
DEGRADED = "degraded"
DNS_FAIL = "dns_fail"
ISP_DOWN = "isp_down"
LAN_DOWN = "lan_down"
ADAPTER_DOWN = "adapter_down"

STATUS_LABEL = {
    OK: "Połączenie sprawne",
    DEGRADED: "Połączenie niestabilne",
    DNS_FAIL: "Internet działa, ale DNS nie odpowiada",
    ISP_DOWN: "Router odpowiada, internet nie — problem po stronie WAN/operatora",
    LAN_DOWN: "Router nie odpowiada — problem między komputerem a routerem",
    ADAPTER_DOWN: "Karta sieciowa rozłączona",
}

STATUS_SCOPE = {
    OK: "ok",
    DEGRADED: "internet",
    DNS_FAIL: "dns",
    ISP_DOWN: "isp",
    LAN_DOWN: "lan",
    ADAPTER_DOWN: "adapter",
}


@dataclass
class Snapshot:
    """One sweep, handed to the GUI."""
    ts: float = 0.0
    status: str = OK
    results: dict = field(default_factory=dict)      # key -> (ok, rtt_ms)
    net: object = None                                # probes.NetState
    dns_ms: float | None = None
    dns_error: str = ""
    roamed: bool = False
    note: str = ""


class Monitor:
    def __init__(self, store, on_sample=None):
        self.store = store
        self.on_sample = on_sample
        self._stop = threading.Event()
        self._thread = None
        self._pool = ThreadPoolExecutor(max_workers=6)

        self.net = probes.NetState()
        self.history = {t["key"]: deque(maxlen=S.history_points) for t in S.targets()}
        self.last = Snapshot()
        self.paused = False

        self._fail_streak = 0
        self._open_event = None
        self._open_status = None
        self._last_bssid = ""
        self._sweep = 0
        self._dns_ms = None
        self._dns_error = ""
        self._dns_busy = False

    def rebuild_targets(self):
        """Re-read the target list after the user edited the settings."""
        keep = {t["key"] for t in S.targets()}
        for key in list(self.history):
            if key not in keep:
                del self.history[key]
        for key in keep:
            old = self.history.get(key)
            new = deque(old or (), maxlen=S.history_points)
            self.history[key] = new

    # -- lifecycle --------------------------------------------------------
    def start(self):
        if self._thread and self._thread.is_alive():
            return
        self._stop.clear()
        self._thread = threading.Thread(target=self._loop, name="netdoc-monitor", daemon=True)
        self._thread.start()

    def stop(self):
        self._stop.set()
        if self._thread:
            self._thread.join(timeout=5)
        self._pool.shutdown(wait=False)
        if self._open_event is not None:
            self.store.close_event(self._open_event, "monitor zatrzymany")
            self._open_event = None

    # -- main loop --------------------------------------------------------
    def _loop(self):
        self.net = probes.read_net_state()
        self._last_bssid = self.net.bssid
        next_state_refresh = 0.0

        while not self._stop.is_set():
            t0 = time.time()
            if self.paused:
                self._stop.wait(0.5)
                continue

            if t0 >= next_state_refresh:
                # Adapter/Wi-Fi state changes slowly; refreshing it every sweep
                # would cost more than the pings themselves.
                self._pool.submit(self._refresh_state)
                next_state_refresh = t0 + 5.0

            if self._sweep % 10 == 0 and not self._dns_busy:
                self._pool.submit(self._refresh_dns)

            snap = self._sweep_once(t0)
            self._record(snap)
            if self.on_sample:
                try:
                    self.on_sample(snap)
                except Exception:
                    pass

            self._sweep += 1
            if self._sweep % 600 == 0:
                self._pool.submit(self.store.prune, S.keep_days)

            elapsed = time.time() - t0
            self._stop.wait(max(0.05, S.probe_interval_s - elapsed))

    def _refresh_state(self):
        try:
            st = probes.read_net_state()
            if st.gateway or st.adapter:
                self.net = st
        except Exception:
            pass

    def _refresh_dns(self):
        self._dns_busy = True
        try:
            ms, err = probes.dns_lookup_ms()
            self._dns_ms, self._dns_error = ms, err
        except Exception as e:
            self._dns_ms, self._dns_error = None, str(e)
        finally:
            self._dns_busy = False

    def _targets(self):
        """Resolve the dynamic targets (router, ISP DNS) against current state."""
        out = []
        for t in S.targets():
            host = t["host"]
            if t["key"] == "gateway":
                host = self.net.gateway
            elif t["key"] == "dns_isp":
                host = self.net.dns_servers[0] if self.net.dns_servers else None
                # If DNS is the router itself, that probe adds nothing.
                if host and host == self.net.gateway:
                    host = self.net.dns_servers[1] if len(self.net.dns_servers) > 1 else None
            if host:
                out.append((t["key"], host, t["scope"]))
        return out

    def _sweep_once(self, ts):
        targets = self._targets()
        futures = {k: self._pool.submit(probes.ping, host) for k, host, _ in targets}
        results = {}
        for key, fut in futures.items():
            try:
                r = fut.result(timeout=S.ping_timeout_ms / 1000.0 + 5)
            except Exception:
                r = probes.PingResult("?", False, error="timeout sondy")
            results[key] = (r.ok, r.rtt_ms)
            if key not in self.history:
                self.history[key] = deque(maxlen=S.history_points)
            self.history[key].append((ts, r.rtt_ms if r.ok else None))

        snap = Snapshot(
            ts=ts,
            results=results,
            net=self.net,
            dns_ms=self._dns_ms,
            dns_error=self._dns_error,
        )
        snap.roamed = bool(self.net.bssid and self._last_bssid and self.net.bssid != self._last_bssid)
        if snap.roamed:
            self._last_bssid = self.net.bssid
        elif self.net.bssid:
            self._last_bssid = self.net.bssid

        snap.status, snap.note = self._classify(results)
        self.last = snap
        return snap

    def _classify(self, results):
        internet_keys = [k for k, _, scope in self._targets() if scope in ("internet", "isp")]
        internet_ok = any(results.get(k, (False, None))[0] for k in internet_keys)
        gw = results.get("gateway")
        gw_ok = gw[0] if gw else None

        if internet_ok:
            if self._dns_error:
                return DNS_FAIL, "Ping po IP przechodzi, rozwiązywanie nazw nie: %s" % self._dns_error
            return self._quality_verdict(results)

        # No internet. Who is still answering?
        if gw_ok:
            return ISP_DOWN, "Router odpowiada (%s), żaden host w internecie nie." % (self.net.gateway or "?")
        if self.net.adapter_type == "Wi-Fi" and not probes.wifi_connected():
            return ADAPTER_DOWN, "Karta Wi-Fi zgłasza brak połączenia z siecią."
        if gw_ok is None:
            return ADAPTER_DOWN, "Brak bramy domyślnej — komputer nie ma trasy do sieci."
        return LAN_DOWN, "Brak odpowiedzi od routera i od internetu."

    def _quality_verdict(self, results):
        """Connection is up — decide whether it is actually usable."""
        rtts = [v for k, (ok, v) in results.items()
                if ok and v is not None and k in ("cloudflare", "google")]
        if not rtts:
            return OK, ""
        worst = max(rtts)
        _, loss, _, _, _, jitter = self.store.stats("cloudflare", 60)
        if loss > S.loss_ok:
            return DEGRADED, "Utrata pakietów %.1f%% w ostatniej minucie." % loss
        if jitter is not None and jitter > S.jitter_ok * 2:
            return DEGRADED, "Jitter %.0f ms — ping skacze, w grach to lagi." % jitter
        if worst > S.ping_bad:
            return DEGRADED, "Ping %.0f ms — powyżej progu grywalności." % worst
        return OK, ""

    # -- event bookkeeping ------------------------------------------------
    def _record(self, snap):
        rows = [(snap.ts, k, v, 1 if ok else 0) for k, (ok, v) in snap.results.items()]
        try:
            self.store.add_samples(rows)
        except Exception:
            pass

        bad = snap.status != OK
        if bad:
            self._fail_streak += 1
        else:
            self._fail_streak = 0

        # Open an event only after a few consecutive bad sweeps, so a single
        # dropped packet does not fill the log with noise.
        if bad and self._fail_streak >= S.outage_after_fails and self._open_event is None:
            ctx = {
                "adapter": snap.net.adapter,
                "type": snap.net.adapter_type,
                "ssid": snap.net.ssid,
                "bssid": snap.net.bssid,
                "signal_pct": snap.net.signal_pct,
                "channel": snap.net.channel,
                "radio": snap.net.radio,
                "rx_rate": snap.net.rx_rate,
                "gateway": snap.net.gateway,
                "dns": snap.net.dns_servers,
                "results": snap.results,
            }
            self._open_event = self.store.open_event(
                snap.status, STATUS_SCOPE.get(snap.status, "?"), snap.note, ctx
            )
            self._open_status = snap.status
        elif not bad and self._open_event is not None:
            self.store.close_event(self._open_event)
            self._open_event = None
            self._open_status = None

    # -- helpers for the GUI ---------------------------------------------
    def series(self, key):
        return list(self.history.get(key, []))

    def summary(self, window_s=300):
        out = {}
        for t in S.targets():
            out[t["key"]] = self.store.stats(t["key"], window_s)
        return out
