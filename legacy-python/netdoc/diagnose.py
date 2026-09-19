"""One-shot diagnostic scan.

Each check returns a Finding. Severity drives the ordering in the GUI, and a
finding may point at a tweak that fixes it.
"""

import re
import time
from dataclasses import dataclass, field

from . import probes
from .settings import S

CRIT = 3
WARN = 2
INFO = 1
GOOD = 0

SEVERITY_LABEL = {CRIT: "KRYTYCZNE", WARN: "OSTRZEŻENIE", INFO: "INFO", GOOD: "OK"}


@dataclass
class Finding:
    key: str
    title: str
    severity: int
    detail: str = ""
    advice: str = ""
    tweak_id: str = ""
    data: dict = field(default_factory=dict)


def _wifi_band(channel):
    try:
        ch = int(str(channel).strip())
    except (TypeError, ValueError):
        return ""
    if ch <= 14:
        return "2.4 GHz"
    if ch >= 36 and ch < 200:
        return "5 GHz"
    return "6 GHz"


def scan(net=None, store=None, progress=None, quick=False):
    """Run every check. progress(text, fraction) is called between steps."""
    net = net or probes.read_net_state()
    findings = []
    steps = [
        ("Sprawdzam kartę i medium", _check_medium),
        ("Sprawdzam jakość Wi-Fi", _check_wifi_quality),
        ("Sprawdzam zatłoczenie kanału", _check_channel),
        ("Sprawdzam oszczędzanie energii karty", _check_power),
        ("Sprawdzam konfigurację DNS", _check_dns),
        ("Mierzę link do routera", _check_local_link),
        ("Mierzę ping do internetu", _check_internet),
        ("Sprawdzam MTU", _check_mtu),
        ("Sprawdzam ustawienia TCP", _check_tcp),
        ("Sprawdzam sterownik karty", _check_driver),
        ("Przeglądam historię zerwań", _check_history),
    ]
    if quick:
        steps = [s for s in steps if s[1] not in (_check_channel, _check_mtu, _check_driver)]

    total = len(steps)
    for i, (label, fn) in enumerate(steps):
        if progress:
            progress(label, i / total)
        try:
            out = fn(net, store)
        except Exception as e:
            out = [Finding(fn.__name__, label, INFO, "Test nie powiódł się: %s" % e)]
        findings.extend(out or [])
    if progress:
        progress("Gotowe", 1.0)

    findings.sort(key=lambda f: -f.severity)
    return findings


# ---------------------------------------------------------------------------

def _check_medium(net, store):
    if not net.adapter:
        return [Finding("medium", "Brak aktywnego połączenia", CRIT,
                        "Nie znaleziono karty z trasą domyślną.",
                        "Sprawdź kabel albo włącz Wi-Fi.")]
    if net.adapter_type == "Ethernet":
        return [Finding("medium", "Połączenie kablowe", GOOD,
                        "%s, %s" % (net.adapter, net.link_speed),
                        "Najlepszy możliwy wariant pod ping.")]
    return [Finding(
        "medium", "Połączenie przez Wi-Fi", INFO,
        "%s — %s, %s" % (net.adapter, net.ssid or "?", net.link_speed),
        "Wi-Fi zawsze ma wyższy jitter niż kabel i jest podatne na zakłócenia. "
        "Jeśli zerwania zdarzają się głównie w grach, kabel usuwa większość przyczyn naraz.",
    )]


def _check_wifi_quality(net, store):
    if net.adapter_type != "Wi-Fi":
        return []
    out = []
    sig = net.signal_pct
    band = _wifi_band(net.channel)
    if sig is None:
        return []
    if sig < 45:
        out.append(Finding(
            "signal", "Słaby sygnał Wi-Fi (%d%%)" % sig, CRIT,
            "Pasmo %s, kanał %s, %s, odbiór %s Mb/s." % (band, net.channel, net.radio, net.rx_rate),
            "Przy tym poziomie karta gubi ramki i okresowo się rozłącza. "
            "Przybliż się do routera, przenieś router wyżej/centralnie albo przejdź na 2.4 GHz "
            "(większy zasięg kosztem prędkości).",
        ))
    elif sig < 65:
        out.append(Finding(
            "signal", "Przeciętny sygnał Wi-Fi (%d%%)" % sig, WARN,
            "Pasmo %s, kanał %s, %s." % (band, net.channel, net.radio),
            "Wystarczy do przeglądania, ale przy szczycie obciążenia będzie skakać ping.",
        ))
    else:
        out.append(Finding(
            "signal", "Sygnał Wi-Fi dobry (%d%%)" % sig, GOOD,
            "Pasmo %s, kanał %s, %s, odbiór %s Mb/s." % (band, net.channel, net.radio, net.rx_rate),
        ))

    if band == "2.4 GHz":
        out.append(Finding(
            "band", "Sieć działa w paśmie 2.4 GHz", WARN,
            "Kanał %s." % net.channel,
            "2.4 GHz dzielisz z mikrofalówką, Bluetooth i sąsiadami. "
            "Jeśli router ma 5 GHz, połącz się z tamtym SSID.",
        ))
    return out


def _check_channel(net, store):
    if net.adapter_type != "Wi-Fi" or not net.channel:
        return []
    try:
        mine = int(str(net.channel).strip())
    except ValueError:
        return []
    nets = probes.wifi_neighbours()
    same = 0
    for n in nets:
        if n["ssid"] == net.ssid:
            continue
        if mine in n["channels"]:
            same += 1
    if same >= 3:
        return [Finding(
            "channel", "Kanał %d jest zatłoczony" % mine, WARN,
            "Na tym samym kanale nadaje %d innych sieci." % same,
            "Wejdź w panel routera i ustaw kanał ręcznie — w 2.4 GHz tylko 1, 6 lub 11; "
            "w 5 GHz wybierz kanał wolny od sąsiadów.",
            data={"same": same},
        )]
    return [Finding("channel", "Kanał %d bez większej konkurencji" % mine, GOOD,
                    "Sieci na tym samym kanale: %d." % same)]


def _check_power(net, store):
    from . import optimize
    t = optimize.WifiPowerSaving(net)
    txt, optimal, _ = t.status()
    if optimal is False:
        return [Finding(
            "power", "Windows może usypiać kartę sieciową", CRIT,
            txt,
            "To najczęstsza przyczyna zerwań „z niczego” na laptopie: karta zasypia "
            "w bezczynności, a powrót trwa kilka sekund. Wyłącz to w zakładce Optymalizacja.",
            tweak_id="wifi_power",
        )]
    if optimal is True:
        return [Finding("power", "Oszczędzanie energii karty wyłączone", GOOD, txt)]
    return [Finding("power", "Nie udało się sprawdzić zarządzania energią karty", INFO, txt)]


def _check_dns(net, store):
    out = []
    ms, err = probes.dns_lookup_ms()
    if err:
        out.append(Finding(
            "dns_resolve", "Rozwiązywanie nazw nie działa", CRIT, err,
            "Ping po adresie IP może chodzić, a strony i tak się nie otworzą. "
            "Zmień DNS na 1.1.1.1 w zakładce Optymalizacja.",
            tweak_id="fast_dns",
        ))
    elif ms is not None and ms > 150:
        out.append(Finding(
            "dns_slow", "Wolne DNS (%.0f ms)" % ms, WARN,
            "Serwery: %s" % (", ".join(net.dns_servers) or "z DHCP"),
            "Każde nowe połączenie czeka na tę odpowiedź — strony „mielą” zanim się zaczną ładować.",
            tweak_id="fast_dns",
        ))
    else:
        out.append(Finding("dns_ok", "DNS odpowiada szybko (%.0f ms)" % (ms or 0), GOOD,
                           "Serwery: %s" % (", ".join(net.dns_servers) or "z DHCP")))

    if net.dns_servers and all(d == net.gateway for d in net.dns_servers):
        out.append(Finding(
            "dns_router", "Jedynym serwerem DNS jest router", WARN,
            "DNS: %s" % net.gateway,
            "Gdy router się zatnie albo przeładuje, wygląda to identycznie jak awaria internetu. "
            "Dodanie 1.1.1.1 jako drugiego DNS usuwa ten pojedynczy punkt awarii.",
            tweak_id="fast_dns",
        ))
    return out


def _check_local_link(net, store):
    if not net.gateway:
        return [Finding("gateway", "Brak bramy domyślnej", CRIT, "",
                        "Komputer nie ma trasy do sieci — sprawdź DHCP na routerze.")]
    rtts = probes.ping_series(net.gateway, count=10)
    got = [r for r in rtts if r is not None]
    loss = (len(rtts) - len(got)) / len(rtts) * 100
    if not got:
        return [Finding("gateway", "Router nie odpowiada", CRIT,
                        "10/10 pakietów zgubionych do %s." % net.gateway,
                        "Problem jest między komputerem a routerem, nie u operatora.")]
    avg = sum(got) / len(got)
    jitter = (sum(abs(got[i] - got[i - 1]) for i in range(1, len(got))) / (len(got) - 1)
              if len(got) > 1 else 0.0)
    detail = "avg %.1f ms, min %.0f, max %.0f, jitter %.1f ms, strata %.0f%%" % (
        avg, min(got), max(got), jitter, loss)
    if loss > 0 or avg > 15 or jitter > 10:
        return [Finding(
            "gateway", "Link do routera jest niestabilny", WARN, detail,
            "Ping do własnego routera powinien być poniżej 5 ms i bez strat. "
            "Taki wynik oznacza problem na odcinku komputer–router (Wi-Fi, kabel, przeciążony router), "
            "a nie u operatora.",
            data={"avg": avg, "jitter": jitter, "loss": loss},
        )]
    return [Finding("gateway", "Link do routera w porządku", GOOD, detail,
                    data={"avg": avg, "jitter": jitter, "loss": loss})]


def _check_internet(net, store):
    rtts = probes.ping_series("1.1.1.1", count=15)
    got = [r for r in rtts if r is not None]
    loss = (len(rtts) - len(got)) / len(rtts) * 100
    if not got:
        return [Finding("internet", "Brak odpowiedzi z internetu", CRIT,
                        "15/15 pakietów zgubionych do 1.1.1.1.",
                        "Jeśli router odpowiada, problem jest po stronie WAN/operatora.")]
    avg = sum(got) / len(got)
    jitter = (sum(abs(got[i] - got[i - 1]) for i in range(1, len(got))) / (len(got) - 1)
              if len(got) > 1 else 0.0)
    spread = max(got) - min(got)
    detail = "avg %.1f ms, min %.0f, max %.0f, jitter %.1f ms, strata %.0f%%" % (
        avg, min(got), max(got), jitter, loss)
    data = {"avg": avg, "jitter": jitter, "loss": loss}

    out = []
    if loss > S.loss_ok:
        out.append(Finding("loss", "Utrata pakietów %.0f%%" % loss, CRIT, detail,
                           "Powyżej 2% strat gry zaczynają się rwać, a połączenia TCP zwalniać. "
                           "Najpierw sprawdź link do routera — jeśli tam jest czysto, problem jest dalej.",
                           data=data))
    if jitter > S.jitter_ok:
        out.append(Finding("jitter", "Wysoki jitter (%.0f ms)" % jitter, WARN, detail,
                           "Ping skacze o %.0f ms między pakietami. To właśnie odczuwasz jako "
                           "„laguje mimo dobrego pingu”. Typowe przy Wi-Fi i przy przeciążonym łączu."
                           % spread, data=data))
    if avg > S.ping_bad:
        out.append(Finding("ping", "Wysoki ping (%.0f ms)" % avg, WARN, detail,
                           "Sprawdź trasą, gdzie rośnie opóźnienie (przycisk Traceroute).", data=data))
    if not out:
        out.append(Finding("internet", "Ping do internetu w normie (%.0f ms)" % avg, GOOD,
                           detail, data=data))
    return out


def _check_mtu(net, store):
    from . import optimize
    t = optimize.MtuFix(net)
    best = t.probe_best_mtu()
    txt, _, cur = t.status()
    if best is None:
        return [Finding("mtu", "Nie udało się zmierzyć MTU", INFO,
                        "Test wymaga ICMP z flagą „don't fragment”, część sieci go blokuje.")]
    if cur and best < cur:
        return [Finding(
            "mtu", "MTU jest za duże", WARN,
            "Ustawione %d, a bez fragmentacji przechodzi %d." % (cur, best),
            "Pakiety powyżej %d są po drodze odrzucane. Objaw: ping działa, "
            "ale część stron się nie ładuje." % best,
            tweak_id="mtu", data={"best": best, "current": cur},
        )]
    return [Finding("mtu", "MTU poprawne (%s)" % (cur or best), GOOD,
                    "Największy pakiet bez fragmentacji odpowiada MTU %d." % best)]


def _check_tcp(net, store):
    from . import optimize
    out = []
    for cls, good_msg in ((optimize.TcpAutotuning, "TCP auto-tuning ustawiony poprawnie"),
                          (optimize.NetworkThrottling, "Limit pakietów multimediów zdjęty")):
        t = cls(net)
        txt, optimal, _ = t.status()
        if optimal is True:
            out.append(Finding(t.id, good_msg, GOOD, txt))
        elif optimal is False:
            out.append(Finding(t.id, t.title, INFO, txt, t.why, tweak_id=t.id))
    return out


def _check_driver(net, store):
    if not net.adapter:
        return []
    rc, out = probes.powershell(
        "$a = Get-NetAdapter -Name '%s' -ErrorAction SilentlyContinue;"
        " if ($a) { 'DRV=' + $a.DriverVersion; 'DATE=' + $a.DriverDate;"
        " 'PROV=' + $a.DriverProvider }" % net.adapter
    )
    ver = re.search(r"DRV=(.+)", out)
    date = re.search(r"DATE=(.+)", out)
    detail = "%s, wersja %s, data %s" % (
        net.adapter_desc or net.adapter,
        ver.group(1).strip() if ver else "?",
        date.group(1).strip()[:10] if date else "?",
    )
    age_years = None
    if date:
        m = re.search(r"(\d{1,2})[/.](\d{1,2})[/.](\d{4})", date.group(1))
        if m:
            year = int(m.group(3))
            age_years = time.localtime().tm_year - year
    if age_years is not None and age_years >= 3:
        return [Finding(
            "driver", "Stary sterownik karty sieciowej (%d lata+)" % age_years, WARN, detail,
            "Sterowniki Wi-Fi bywają główną przyczyną losowych rozłączeń, zwłaszcza MediaTek "
            "i Intel AX. Pobierz najnowszy ze strony producenta laptopa lub chipsetu — "
            "Windows Update zwykle podsuwa starszą wersję.",
        )]
    return [Finding("driver", "Sterownik karty", INFO, detail)]


def _check_history(net, store):
    if store is None:
        return []
    events = store.events_since(24 * 3600)
    if not events:
        return [Finding("history", "Brak zarejestrowanych zerwań w ostatniej dobie", GOOD,
                        "Monitor zapisuje każdą przerwę — zostaw go włączonego, żeby złapać kolejną.")]
    by_scope = {}
    for _id, ts_start, ts_end, kind, scope, detail in events:
        by_scope.setdefault(scope, []).append((ts_start, ts_end, kind, detail))

    out = []
    scope_advice = {
        "lan": ("Zerwania na odcinku komputer–router",
                "Winowajcą jest Wi-Fi, karta albo sam router. Zacznij od wyłączenia oszczędzania "
                "energii karty i aktualizacji sterownika."),
        "adapter": ("Karta sieciowa traciła połączenie z siecią",
                    "Karta rozłączała się od SSID. To sterownik, oszczędzanie energii "
                    "albo zbyt słaby sygnał."),
        "isp": ("Zerwania po stronie WAN/operatora",
                "Router odpowiadał, ale internet nie. Tu nie pomoże żadne ustawienie w Windows — "
                "to materiał na zgłoszenie do operatora. Pokaż mu godziny z historii."),
        "dns": ("Awarie DNS", "Łącze działało, nazwy się nie rozwiązywały. Zmiana DNS to załatwia."),
        "internet": ("Okresy niestabilności", "Połączenie działało, ale z lagami i stratami."),
    }
    for scope, items in sorted(by_scope.items(), key=lambda kv: -len(kv[1])):
        title, advice = scope_advice.get(scope, ("Zdarzenia: %s" % scope, ""))
        durations = [(e - s) for s, e, _, _ in items if e]
        avg_d = sum(durations) / len(durations) if durations else 0
        times = ", ".join(time.strftime("%H:%M", time.localtime(s)) for s, _, _, _ in items[-6:])
        out.append(Finding(
            "hist_" + scope, "%s — %d× w ostatniej dobie" % (title, len(items)),
            CRIT if len(items) >= 3 else WARN,
            "Średni czas trwania %.0f s. Ostatnie wystąpienia: %s" % (avg_d, times),
            advice, data={"count": len(items), "scope": scope},
        ))
    return out


def summarize(findings):
    """One sentence verdict for the top of the report."""
    crit = [f for f in findings if f.severity == CRIT]
    warn = [f for f in findings if f.severity == WARN]
    if crit:
        return "Znaleziono %d poważnych problemów. Najważniejszy: %s" % (len(crit), crit[0].title)
    if warn:
        return "Bez awarii, ale %d rzeczy da się poprawić. Najważniejsza: %s" % (
            len(warn), warn[0].title)
    return "Sieć wygląda zdrowo — nie znaleziono problemów wymagających reakcji."
