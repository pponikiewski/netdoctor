"""Bufferbloat test: latency while the link is saturated.

An idle ping of 12 ms means nothing if it jumps to 300 ms the moment somebody
starts a download. That jump is bufferbloat — oversized buffers in the router or
at the ISP holding packets instead of dropping them — and it is the usual reason
a game lags "even though the ping is fine".

The test needs real traffic, so it downloads a few dozen MB from Cloudflare's
speed-test endpoint. Nothing is written to disk.
"""

import threading
import time
import urllib.request
from dataclasses import dataclass, field

from . import probes

# Cloudflare's speed test endpoint serves arbitrary-length zero payloads.
LOAD_URL = "https://speed.cloudflare.com/__down?bytes=%d"
CHUNK_BYTES = 25_000_000
STREAMS = 4

GRADES = [
    (25, "A", "Znakomicie — łącze nie puchnie pod obciążeniem."),
    (60, "B", "Dobrze — lekki wzrost, w grach niezauważalny."),
    (150, "C", "Średnio — przy pobieraniu ping wyraźnie rośnie."),
    (400, "D", "Słabo — podczas pobierania gry będą się rwać."),
    (float("inf"), "F", "Bardzo źle — klasyczny bufferbloat."),
]


@dataclass
class BloatResult:
    idle_avg: float | None = None
    idle_max: float | None = None
    loaded_avg: float | None = None
    loaded_max: float | None = None
    loaded_loss: float = 0.0
    bump_ms: float | None = None
    mbps: float | None = None
    grade: str = "?"
    verdict: str = ""
    error: str = ""
    samples_idle: list = field(default_factory=list)
    samples_loaded: list = field(default_factory=list)


def _ping_window(host, seconds, out):
    """Ping continuously for `seconds`, appending rtt (or None) to out."""
    end = time.time() + seconds
    while time.time() < end:
        r = probes.ping(host, timeout_ms=1000)
        out.append(r.rtt_ms if r.ok else None)


def _download(stop_flag, counter, lock):
    """Pull bytes until told to stop, counting what arrived."""
    try:
        req = urllib.request.Request(
            LOAD_URL % CHUNK_BYTES,
            headers={"User-Agent": "NetDoctor/1.0", "Cache-Control": "no-cache"},
        )
        while not stop_flag.is_set():
            with urllib.request.urlopen(req, timeout=15) as resp:
                while not stop_flag.is_set():
                    chunk = resp.read(65536)
                    if not chunk:
                        break
                    with lock:
                        counter[0] += len(chunk)
    except Exception:
        # A single stream failing is fine as long as the others carry load.
        pass


def run(host="1.1.1.1", idle_s=6, load_s=12, progress=None, stop_event=None):
    """Measure latency before and during saturation. Returns a BloatResult."""
    res = BloatResult()

    def say(text, frac):
        if progress:
            progress(text, frac)

    say("Mierzę ping na spoczynku…", 0.05)
    idle = []
    _ping_window(host, idle_s, idle)
    res.samples_idle = idle
    got = [v for v in idle if v is not None]
    if not got:
        res.error = "Brak odpowiedzi z %s — test przerwany." % host
        return res
    res.idle_avg = sum(got) / len(got)
    res.idle_max = max(got)

    if stop_event and stop_event.is_set():
        res.error = "Przerwano."
        return res

    say("Wysycam łącze i mierzę ping pod obciążeniem…", 0.4)
    stop_flag = threading.Event()
    counter = [0]
    lock = threading.Lock()
    threads = [threading.Thread(target=_download, args=(stop_flag, counter, lock), daemon=True)
               for _ in range(STREAMS)]
    for t in threads:
        t.start()

    # Give the streams a moment to ramp up before the buffers matter.
    time.sleep(1.5)
    t0 = time.time()
    with lock:
        start_bytes = counter[0]

    loaded = []
    _ping_window(host, load_s, loaded)

    elapsed = time.time() - t0
    with lock:
        moved = counter[0] - start_bytes
    stop_flag.set()
    for t in threads:
        t.join(timeout=2)

    res.samples_loaded = loaded
    got_l = [v for v in loaded if v is not None]
    res.loaded_loss = (len(loaded) - len(got_l)) / len(loaded) * 100 if loaded else 0.0
    if elapsed > 0 and moved > 0:
        res.mbps = moved * 8 / elapsed / 1_000_000

    if not got_l:
        res.error = "Pod obciążeniem host przestał odpowiadać — to już samo w sobie wynik."
        res.grade = "F"
        res.verdict = ("Łącze pod obciążeniem gubi wszystkie pakiety ICMP. "
                       "Przy pobieraniu wszystko inne przestaje działać.")
        return res

    res.loaded_avg = sum(got_l) / len(got_l)
    res.loaded_max = max(got_l)
    res.bump_ms = res.loaded_avg - res.idle_avg

    for limit, grade, verdict in GRADES:
        if res.bump_ms < limit:
            res.grade, res.verdict = grade, verdict
            break

    say("Gotowe", 1.0)
    return res


def advice(res):
    """What to actually do about the result."""
    if res.error and res.grade == "?":
        return res.error
    if res.grade in ("A", "B"):
        return ("Nic nie trzeba robić — łącze radzi sobie pod obciążeniem. "
                "Jeśli mimo to zdarzają się lagi, przyczyna leży gdzie indziej "
                "(Wi-Fi, sterownik, trasa do serwera).")
    lines = [
        "Ping rośnie o %.0f ms, gdy łącze jest zajęte. To znaczy, że pakiety "
        "czekają w kolejce — u Ciebie w routerze albo u operatora." % (res.bump_ms or 0),
        "",
        "Co pomaga, w kolejności skuteczności:",
        "1. Włącz SQM / Smart Queue / QoS w routerze (szukaj „cake” lub „fq_codel”) "
        "i ustaw limit na ~90% realnej prędkości łącza.",
        "2. Jeśli router tego nie ma — to najlepszy powód, żeby go wymienić. "
        "Żadne ustawienie w Windows tego nie naprawi.",
        "3. Doraźnie: ogranicz prędkość pobierania w programach, które zapychają łącze "
        "(Steam, torrenty, aktualizacje), do ~80% przepustowości.",
    ]
    if res.mbps:
        lines.append("")
        lines.append("Zmierzona przepustowość w teście: %.0f Mb/s — użyj jej do ustawienia limitu."
                     % res.mbps)
    return "\n".join(lines)
