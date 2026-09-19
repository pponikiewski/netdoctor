//! Bilingual string table, resolved at compile time.
//!
//! Both languages are baked into the binary, so a lookup is a branch on an
//! atomic and every string stays `&'static str`. That keeps the `Tweak` trait
//! and the `Severity::label` style of API unchanged, and it costs nothing at
//! runtime beyond the load.
//!
//! Technical vocabulary (bufferbloat, jitter, MTU, TCP autotuning, Nagle)
//! stays in English inside Polish sentences: that is how the terms are used in
//! practice, and it keeps them searchable.

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    En,
    Pl,
}

impl Lang {
    /// Name of the language in that language, for the picker.
    pub fn native_name(&self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Pl => "Polski",
        }
    }

    pub const ALL: [Lang; 2] = [Lang::En, Lang::Pl];
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

#[inline]
pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Pl,
        _ => Lang::En,
    }
}

/// Declares one accessor per string. Missing a translation is a compile error,
/// which is the whole reason the table lives in code rather than in a file.
macro_rules! strings {
    ($($(#[$meta:meta])* $key:ident => $en:expr, $pl:expr;)*) => {
        $(
            $(#[$meta])*
            #[inline]
            pub fn $key() -> &'static str {
                match current() {
                    Lang::En => $en,
                    Lang::Pl => $pl,
                }
            }
        )*
    };
}

strings! {
    // -----------------------------------------------------------------------
    // shell: tabs, common buttons, shared vocabulary
    // -----------------------------------------------------------------------
    tab_live => "Live", "Na żywo";
    tab_diagnose => "Diagnose", "Diagnostyka";
    tab_bloat => "Load test", "Test obciążeniowy";
    tab_optimise => "Optimise", "Optymalizacja";
    tab_history => "Outage history", "Historia awarii";
    tab_settings => "Settings", "Ustawienia";

    btn_dismiss => "Dismiss", "Zamknij";
    btn_apply => "Apply", "Zastosuj";
    btn_revert => "Revert", "Cofnij";
    btn_refresh => "Refresh", "Odśwież";

    word_none => "none", "brak";
    word_router => "Router", "Router";
    word_isp_resolver => "ISP resolver", "Resolver ISP";

    hdr_administrator => "administrator", "administrator";
    hdr_restart_elevated => "Restart as administrator", "Uruchom ponownie jako administrator";
    hdr_standard_mode =>
        "standard mode — changes need elevation",
        "tryb zwykły — zmiany wymagają uprawnień administratora";
    hdr_no_adapter => "No active adapter.", "Brak aktywnej karty sieciowej.";

    // -----------------------------------------------------------------------
    // probes
    // -----------------------------------------------------------------------
    medium_unknown => "unknown", "nieznane";
    ping_unreachable => "host unreachable", "host nieosiągalny";
    ping_timeout => "no reply", "brak odpowiedzi";
    ping_no_route => "no route — adapter offline?", "brak trasy — karta offline?";
    dns_no_addresses => "resolver returned no addresses", "resolver nie zwrócił żadnych adresów";

    // -----------------------------------------------------------------------
    // shell chrome and the command line
    // -----------------------------------------------------------------------
    app_tagline =>
        "network diagnostics and latency optimiser",
        "diagnostyka sieci i optymalizacja opóźnień";
    err_open_db =>
        "Cannot open the history database:",
        "Nie można otworzyć bazy historii:";
    cli_help =>
"USAGE:
    netdoctor [OPTIONS]

OPTIONS:
    --scan         run the diagnostic scan on the console and exit
    --minimised    start with the window minimised (used by autostart)
    --version      print the version and exit
    --help         print this help

Diagnostics work without elevation. Applying changes needs administrator
rights; the app offers to relaunch itself when you ask it to apply one.",
"UŻYCIE:
    netdoctor [OPCJE]

OPCJE:
    --scan         uruchom skan diagnostyczny w konsoli i zakończ
    --minimised    uruchom z oknem zminimalizowanym (używane przy autostarcie)
    --version      wypisz wersję i zakończ
    --help         wypisz tę pomoc

Diagnostyka działa bez podniesionych uprawnień. Zastosowanie zmian wymaga praw
administratora; aplikacja sama zaproponuje ponowne uruchomienie, gdy o to
poprosisz.";

    // -----------------------------------------------------------------------
    // severity
    // -----------------------------------------------------------------------
    sev_critical => "CRITICAL", "KRYTYCZNE";
    sev_warning => "WARNING", "OSTRZEŻENIE";
    sev_info => "INFO", "INFO";
    sev_ok => "OK", "OK";

    // -----------------------------------------------------------------------
    // settings tab
    // -----------------------------------------------------------------------
    set_language => "Language", "Język";
    set_language_hint =>
        "Takes effect immediately. Detected from Windows on first run.",
        "Działa od razu. Przy pierwszym uruchomieniu wykrywany z Windowsa.";

    set_sec_probing => "Probing", "Sondowanie";
    set_sec_targets => "Extra ping targets", "Dodatkowe cele pingowania";
    set_sec_thresholds => "Thresholds", "Progi";
    set_sec_behaviour => "Behaviour", "Zachowanie";

    set_interval => "Interval between sweeps (ms)", "Odstęp między seriami (ms)";
    set_interval_hint =>
        "Lower is more detailed but adds traffic. The floor is 300 ms.",
        "Mniej znaczy dokładniej, ale więcej ruchu. Dolna granica to 300 ms.";
    set_ping_timeout => "Ping timeout (ms)", "Limit czasu pingu (ms)";
    set_fails_before_alarm => "Failed sweeps before an alarm", "Nieudane serie przed alarmem";
    set_fails_hint =>
        "Guards against logging a single dropped packet as an outage.",
        "Chroni przed zapisaniem pojedynczego zgubionego pakietu jako awarii.";
    set_chart_width => "Chart width (samples)", "Szerokość wykresu (próbki)";
    set_keep_days => "Days of history to keep", "Ile dni historii przechowywać";

    set_targets_hint =>
        "One host per line — for example the game server you play on. Names are resolved when \
         settings are saved.",
        "Jeden host w linii — na przykład serwer gry, na którym grasz. Nazwy są rozwiązywane \
         przy zapisie ustawień.";

    set_lat_good => "Latency still good up to (ms)", "Opóźnienie jeszcze dobre do (ms)";
    set_lat_bad => "Latency bad above (ms)", "Opóźnienie złe powyżej (ms)";
    set_jitter_good => "Jitter good up to (ms)", "Jitter dobry do (ms)";
    set_jitter_ok => "Jitter acceptable up to (ms)", "Jitter akceptowalny do (ms)";
    set_loss_ok => "Acceptable packet loss (%)", "Akceptowalna utrata pakietów (%)";

    set_notify => "Announce outages in the app", "Zgłaszaj awarie w aplikacji";
    set_start_min => "Start minimised", "Uruchamiaj zminimalizowany";
    set_autostart => "Start with Windows", "Uruchamiaj razem z Windowsem";
    set_autostart_hint =>
        "Autostart only matters together with background monitoring: without it the app cannot \
         record an outage that happens while it is closed.",
        "Autostart ma sens tylko razem z monitorowaniem w tle: bez niego aplikacja nie zapisze \
         awarii, która zdarzy się przy zamkniętym programie.";
    set_autostart_stale =>
        "The autostart entry points at an executable that no longer exists. Toggle it off and on \
         again to repoint it here.",
        "Wpis autostartu wskazuje na plik, którego już nie ma. Wyłącz i włącz go ponownie, żeby \
         wskazywał na ten plik.";

    set_btn_save => "Save settings", "Zapisz ustawienia";
    set_btn_defaults => "Restore defaults", "Przywróć domyślne";

    set_err_interval =>
        "An interval below 300 ms loads the network more than it measures.",
        "Odstęp poniżej 300 ms bardziej obciąża sieć, niż ją mierzy.";
    set_err_thresholds =>
        "\"Good\" latency must be lower than \"bad\" latency.",
        "Opóźnienie „dobre” musi być mniejsze niż „złe”.";
    set_saved => "Settings saved.", "Ustawienia zapisane.";

    // -----------------------------------------------------------------------
    // live tab
    // -----------------------------------------------------------------------
    live_roamed =>
        "The card just roamed to a different access point.",
        "Karta właśnie przełączyła się na inny access point.";
    live_x_now => "now", "teraz";
    live_red_line => "red line = lost packet", "czerwona linia = zgubiony pakiet";
    live_no_data => "no data", "brak danych";

    live_card_latency => "Latency (1.1.1.1)", "Opóźnienie (1.1.1.1)";
    live_card_latency_sub_none => "no data yet", "jeszcze brak danych";
    live_card_jitter => "Jitter", "Jitter";
    live_card_jitter_sub => "swing between packets", "wahania między pakietami";
    live_card_loss => "Packet loss", "Utrata pakietów";
    live_card_loss_sub => "last 5 minutes", "ostatnie 5 minut";
    live_card_router => "Router latency", "Opóźnienie do routera";
    live_card_router_none => "not answering", "nie odpowiada";
    live_card_dns => "DNS", "DNS";
    live_card_dns_err => "error", "błąd";
    live_card_dns_sub => "name resolution", "rozwiązywanie nazw";
    live_card_uninterrupted => "Uninterrupted", "Bez przerw";
    live_card_uninterrupted_val => "24 h+", "24 h+";
    live_card_uninterrupted_sub => "no outages logged", "brak zapisanych awarii";
    live_card_since_outage => "Since last outage", "Od ostatniej awarii";

    live_btn_pause => "Pause monitor", "Wstrzymaj monitor";
    live_btn_resume => "Resume monitor", "Wznów monitor";
    live_btn_trace => "Traceroute to 1.1.1.1", "Traceroute do 1.1.1.1";
    live_btn_report => "Save report…", "Zapisz raport…";
    live_tracing => "Tracing route…", "Śledzenie trasy…";
    live_trace_note_1 =>
        "Hop 1 is your router. If latency only climbs further out,",
        "Skok 1 to Twój router. Jeśli opóźnienie rośnie dopiero dalej,";
    live_trace_note_2 =>
        "the problem is on the provider's side, not yours.",
        "problem jest po stronie dostawcy, nie u Ciebie.";

    // -----------------------------------------------------------------------
    // diagnose tab
    // -----------------------------------------------------------------------
    diag_scan_hint => "A full scan takes about 30 seconds.", "Pełny skan trwa około 30 sekund.";
    diag_scan_done => "Scan complete.", "Skan zakończony.";
    diag_scan_starting => "Starting…", "Uruchamianie…";
    diag_btn_scan => "Run full scan", "Uruchom pełny skan";
    diag_no_scan_yet =>
        "No scan yet. The scan measures the link to your router, latency and loss to the \
         internet, DNS behaviour, Wi-Fi quality, MTU, TCP settings and the recorded outage \
         history.",
        "Jeszcze nie było skanu. Skan mierzy łącze do routera, opóźnienie i straty do \
         internetu, zachowanie DNS, jakość Wi-Fi, MTU, ustawienia TCP oraz zapisaną historię \
         awarii.";
    diag_select_finding =>
        "Select a finding to see what it means.",
        "Wybierz wynik, żeby zobaczyć, co oznacza.";
    diag_btn_fix => "Fix this", "Napraw to";

    // -----------------------------------------------------------------------
    // history tab
    // -----------------------------------------------------------------------
    hist_none_24h => "No outages in the last 24 hours.", "Brak awarii w ciągu ostatnich 24 godzin.";
    hist_where_lan => "between this PC and the router", "między tym komputerem a routerem";
    hist_where_adapter => "on the network adapter", "na karcie sieciowej";
    hist_where_isp => "on the provider side", "po stronie dostawcy";
    hist_where_dns => "in DNS", "w DNS";
    hist_where_other => "as degraded quality", "jako pogorszenie jakości";
    hist_blurb =>
        "Each entry records the connection state at that moment — signal, channel, access point \
         and whether the router was still answering. That last detail is what decides whether it \
         was your laptop or your provider.",
        "Każdy wpis zapisuje stan połączenia z danej chwili — sygnał, kanał, access point oraz \
         to, czy router jeszcze odpowiadał. Ten ostatni szczegół rozstrzyga, czy zawinił Twój \
         komputer, czy dostawca.";
    hist_nothing_logged => "Nothing logged yet.", "Nic jeszcze nie zapisano.";
    hist_col_started => "Started", "Początek";
    hist_col_duration => "Duration", "Czas trwania";
    hist_col_kind => "Kind", "Rodzaj";
    hist_col_detail => "Detail", "Szczegóły";
    hist_ongoing => "ongoing", "trwa";

    // -----------------------------------------------------------------------
    // history tab: the cause panel for a selected outage
    // -----------------------------------------------------------------------
    hist_select_hint =>
        "Select an outage to see what led up to it.",
        "Wybierz awarię, żeby zobaczyć, co ją poprzedziło.";
    hist_cause_heading => "Probable cause", "Prawdopodobna przyczyna";
    hist_leadup_heading => "Before, during and after", "Przed, w trakcie i po";
    hist_state_heading => "Connection at the moment it broke", "Połączenie w chwili zerwania";
    hist_recovery_heading => "Connection when it came back", "Połączenie po powrocie";
    hist_no_recovery => "It has not come back yet.", "Jeszcze nie wróciło.";
    hist_btn_fix => "Open the fix", "Otwórz poprawkę";
    hist_btn_close => "Close", "Zamknij";
    hist_no_leadup =>
        "No lead-up was recorded for this outage — it was logged by an earlier version.",
        "Dla tej awarii nie zapisano przebiegu — wpis pochodzi z wcześniejszej wersji.";
    hist_no_state =>
        "No connection state was stored for this entry.",
        "Dla tego wpisu nie zapisano stanu połączenia.";
    hist_leadup_rssi => "Signal (dBm)", "Sygnał (dBm)";
    hist_leadup_rtt => "Router (ms)", "Router (ms)";
    hist_tweaks_heading => "Changes applied shortly before", "Zmiany zastosowane krótko przed";
    hist_leadup_axis =>
        "seconds relative to the start of the outage",
        "sekundy względem początku awarii";

    // Confidence in a reading, not severity of the outage.
    conf_certain => "clear", "jednoznaczne";
    conf_likely => "likely", "prawdopodobne";
    conf_possible => "possible", "możliwe";

    // -----------------------------------------------------------------------
    // diagnostic scan: progress steps
    // -----------------------------------------------------------------------
    step_medium => "Checking adapter and medium", "Sprawdzanie karty i medium";
    step_wifi => "Checking Wi-Fi quality", "Sprawdzanie jakości Wi-Fi";
    step_power => "Checking adapter power management", "Sprawdzanie zarządzania energią karty";
    step_dns => "Checking DNS configuration", "Sprawdzanie konfiguracji DNS";
    step_link => "Measuring the link to the router", "Pomiar łącza do routera";
    step_internet => "Measuring internet latency", "Pomiar opóźnienia do internetu";
    step_mtu => "Checking MTU", "Sprawdzanie MTU";
    step_tcp => "Checking TCP settings", "Sprawdzanie ustawień TCP";
    step_history => "Reviewing outage history", "Przegląd historii awarii";
    step_done => "Done", "Gotowe";

    scan_all_healthy =>
        "The network looks healthy — nothing needs attention.",
        "Sieć wygląda zdrowo — nic nie wymaga uwagi.";

    // -----------------------------------------------------------------------
    // findings: adapter and medium
    // -----------------------------------------------------------------------
    f_no_connection => "No active connection", "Brak aktywnego połączenia";
    f_no_connection_detail =>
        "No adapter holds a default route.",
        "Żadna karta nie ma trasy domyślnej.";
    f_no_connection_advice => "Check the cable, or turn Wi-Fi on.", "Sprawdź kabel albo włącz Wi-Fi.";
    f_wired => "Wired connection", "Połączenie przewodowe";
    f_wired_advice =>
        "The best possible starting point for latency.",
        "Najlepszy możliwy punkt wyjścia dla opóźnień.";
    f_wifi => "Connected over Wi-Fi", "Połączenie przez Wi-Fi";
    f_wifi_advice =>
        "Wi-Fi always has higher jitter than a cable and is vulnerable to interference. If the \
         drops happen mostly while gaming, a cable removes several causes at once.",
        "Wi-Fi zawsze ma wyższy jitter niż kabel i jest podatne na zakłócenia. Jeśli zrywy \
         zdarzają się głównie w trakcie grania, kabel usuwa kilka przyczyn naraz.";

    // -----------------------------------------------------------------------
    // findings: Wi-Fi quality
    // -----------------------------------------------------------------------
    f_signal_weak_advice =>
        "At this level the card drops frames and will periodically disconnect. Move closer, \
         reposition the router, or switch to 2.4 GHz for range at the cost of speed.",
        "Przy takim poziomie karta gubi ramki i będzie się okresowo rozłączać. Podejdź bliżej, \
         przestaw router albo przejdź na 2.4 GHz — zyskasz zasięg kosztem prędkości.";
    f_signal_mid_advice =>
        "Fine for browsing, but latency will spike under load.",
        "Do przeglądania wystarczy, ale pod obciążeniem opóźnienie będzie skakać.";
    f_band_24 => "Running on the 2.4 GHz band", "Praca w paśmie 2.4 GHz";
    f_band_24_advice =>
        "2.4 GHz is shared with microwaves, Bluetooth and every neighbour. If the router offers 5 \
         GHz, connect to that SSID instead.",
        "Pasmo 2.4 GHz dzielisz z mikrofalówkami, Bluetoothem i każdym sąsiadem. Jeśli router \
         udostępnia 5 GHz, połącz się z tamtym SSID.";

    // -----------------------------------------------------------------------
    // findings: adapter power management
    // -----------------------------------------------------------------------
    f_power_bad =>
        "Windows is allowed to power down the network adapter",
        "Windows może wyłączać zasilanie karty sieciowej";
    f_power_bad_advice =>
        "This is the most common cause of drops \"out of nowhere\" on a laptop: the card suspends \
         while idle and takes seconds to come back. Fix it on the Optimise tab.",
        "To najczęstsza przyczyna zrywów „znikąd” na laptopie: karta usypia w bezczynności i \
         potrzebuje kilku sekund, żeby wrócić. Napraw to na zakładce Optymalizacja.";
    f_power_good => "Adapter power saving is disabled", "Oszczędzanie energii karty jest wyłączone";
    f_power_unknown =>
        "Could not read adapter power management",
        "Nie udało się odczytać zarządzania energią karty";

    // -----------------------------------------------------------------------
    // findings: DNS
    // -----------------------------------------------------------------------
    f_dns_from_dhcp => "from DHCP", "z DHCP";
    f_dns_failing => "Name resolution is failing", "Rozwiązywanie nazw zawodzi";
    f_dns_failing_advice =>
        "Pings by IP may work while nothing loads in a browser. Switch to 1.1.1.1 on the Optimise \
         tab.",
        "Ping po IP może działać, a mimo to nic się nie ładuje w przeglądarce. Przełącz się na \
         1.1.1.1 na zakładce Optymalizacja.";
    f_dns_slow_advice =>
        "Every new connection waits on this, which is why pages seem to stall before loading.",
        "Każde nowe połączenie na to czeka — dlatego strony wyglądają, jakby się zawieszały przed \
         załadowaniem.";
    f_dns_unknown => "DNS timing unavailable", "Brak pomiaru czasu DNS";
    f_dns_router_only => "The router is the only DNS server", "Router jest jedynym serwerem DNS";
    f_dns_router_only_advice =>
        "When the router stalls or reboots this looks exactly like an internet outage. Adding \
         1.1.1.1 as a second resolver removes that single point of failure.",
        "Gdy router się zatnie albo zrestartuje, wygląda to dokładnie jak awaria internetu. \
         Dodanie 1.1.1.1 jako drugiego resolvera usuwa ten pojedynczy punkt awarii.";

    // -----------------------------------------------------------------------
    // findings: link to the router
    // -----------------------------------------------------------------------
    f_no_gateway => "No default gateway", "Brak bramy domyślnej";
    f_no_gateway_detail =>
        "This machine has no route to the network.",
        "Ten komputer nie ma trasy do sieci.";
    f_no_gateway_advice => "Check DHCP on the router.", "Sprawdź DHCP na routerze.";
    f_router_silent => "The router is not answering", "Router nie odpowiada";
    f_router_silent_advice =>
        "The problem is between this PC and the router, not at the ISP.",
        "Problem jest między tym komputerem a routerem, nie u dostawcy.";
    f_link_unstable => "The link to the router is unstable", "Łącze do routera jest niestabilne";
    f_link_unstable_advice =>
        "A ping to your own router should be under 5 ms with no loss. This points at the \
         PC-to-router hop — Wi-Fi, cabling, or an overloaded router — not at the ISP.",
        "Ping do własnego routera powinien być poniżej 5 ms i bez strat. To wskazuje na odcinek \
         komputer–router: Wi-Fi, okablowanie albo przeciążony router, a nie dostawcę.";
    f_link_healthy => "The link to the router is healthy", "Łącze do routera jest zdrowe";

    // -----------------------------------------------------------------------
    // findings: internet
    // -----------------------------------------------------------------------
    f_net_silent => "No response from the internet", "Brak odpowiedzi z internetu";
    f_net_silent_detail => "15/15 packets lost to 1.1.1.1.", "15/15 pakietów zgubionych do 1.1.1.1.";
    f_net_silent_advice =>
        "If the router still answers, the fault is on the WAN/ISP side.",
        "Jeśli router nadal odpowiada, wina leży po stronie WAN/ISP.";
    f_loss_advice =>
        "Above 2% games start to stutter and TCP throughput collapses. Check the router link \
         first — if that is clean, the problem is further upstream.",
        "Powyżej 2% gry zaczynają się ciąć, a przepustowość TCP się załamuje. Najpierw sprawdź \
         łącze do routera — jeśli jest czyste, problem jest dalej w górę.";
    f_ping_high_advice =>
        "Run a traceroute to see which hop the delay appears at.",
        "Uruchom traceroute, żeby zobaczyć, na którym skoku pojawia się opóźnienie.";

    // -----------------------------------------------------------------------
    // findings: MTU
    // -----------------------------------------------------------------------
    f_mtu_unmeasured => "Could not measure MTU", "Nie udało się zmierzyć MTU";
    f_mtu_unmeasured_detail =>
        "The test needs ICMP with the don't-fragment flag, which some networks block.",
        "Test wymaga ICMP z flagą don't-fragment, którą część sieci blokuje.";
    f_mtu_too_large =>
        "MTU is larger than the path supports",
        "MTU jest większe, niż obsługuje trasa";

    // -----------------------------------------------------------------------
    // findings: TCP
    // -----------------------------------------------------------------------
    f_tcp_autotuning_ok => "TCP auto-tuning is set correctly", "TCP auto-tuning jest ustawione poprawnie";
    f_throttle_ok => "Multimedia packet throttle is lifted", "Ograniczenie pakietów multimedialnych zdjęte";

    // -----------------------------------------------------------------------
    // findings: outage history
    // -----------------------------------------------------------------------
    f_hist_none => "No outages recorded in the last 24 hours", "Brak awarii w ciągu ostatnich 24 godzin";
    f_hist_none_detail =>
        "The monitor logs every interruption — leave it running to catch the next one.",
        "Monitor zapisuje każdą przerwę — zostaw go włączonego, żeby złapał następną.";
    f_hist_lan => "Drops between this PC and the router", "Zrywy między tym komputerem a routerem";
    f_hist_lan_advice =>
        "The culprit is Wi-Fi, the adapter, or the router itself. Start with adapter power \
         management and a driver update.",
        "Winne jest Wi-Fi, karta sieciowa albo sam router. Zacznij od zarządzania energią karty i \
         aktualizacji sterownika.";
    f_hist_adapter => "The adapter lost its network association", "Karta straciła powiązanie z siecią";
    f_hist_adapter_advice =>
        "The card disconnected from the SSID. That is the driver, power saving, or too weak a \
         signal.",
        "Karta rozłączyła się z SSID. To sterownik, oszczędzanie energii albo zbyt słaby sygnał.";
    f_hist_isp => "Drops on the WAN/ISP side", "Zrywy po stronie WAN/ISP";
    f_hist_isp_advice =>
        "The router answered but the internet did not. No Windows setting fixes this — it is \
         evidence for a support ticket. Show them these timestamps.",
        "Router odpowiadał, a internet nie. Żadne ustawienie Windowsa tego nie naprawi — to \
         materiał do zgłoszenia u dostawcy. Pokaż im te znaczniki czasu.";
    f_hist_dns => "DNS failures", "Awarie DNS";
    f_hist_dns_advice =>
        "The link was up but names would not resolve. Changing DNS fixes this.",
        "Łącze działało, ale nazwy się nie rozwiązywały. Zmiana DNS to naprawia.";
    f_hist_other => "Periods of degraded quality", "Okresy pogorszonej jakości";
    f_hist_other_advice =>
        "The connection worked, but with lag and loss.",
        "Połączenie działało, ale z lagami i stratami.";

    // -----------------------------------------------------------------------
    // monitor verdicts
    // -----------------------------------------------------------------------
    mon_ok => "Connection healthy", "Połączenie sprawne";
    mon_degraded => "Connection unstable", "Połączenie niestabilne";
    mon_dns_fail =>
        "Internet reachable, but name resolution is failing",
        "Internet osiągalny, ale rozwiązywanie nazw zawodzi";
    mon_isp_down =>
        "Router answers, internet does not — WAN/ISP problem",
        "Router odpowiada, internet nie — problem WAN/ISP";
    mon_lan_down =>
        "Router not answering — problem between PC and router",
        "Router nie odpowiada — problem między komputerem a routerem";
    mon_adapter_down => "Network adapter disconnected", "Karta sieciowa rozłączona";

    mon_wifi_deassociated =>
        "Wi-Fi card reports it is no longer associated with the network.",
        "Karta Wi-Fi zgłasza, że nie jest już powiązana z siecią.";
    mon_nothing_responded =>
        "Neither the router nor the internet responded.",
        "Nie odpowiedział ani router, ani internet.";
    mon_no_gateway =>
        "No default gateway — this machine has no route to the network.",
        "Brak bramy domyślnej — ten komputer nie ma trasy do sieci.";

    // -----------------------------------------------------------------------
    // load test tab
    // -----------------------------------------------------------------------
    bloat_test_done => "Test complete.", "Test zakończony.";
    bloat_starting => "Starting…", "Uruchamianie…";
    bloat_title => "Latency under load", "Opóźnienie pod obciążeniem";
    bloat_blurb =>
        "Idle latency says little. What matters is what happens to it when somebody in the house \
         starts a download. This test saturates the link and measures the rise.\n\n\
         It pulls a few dozen MB from Cloudflare and takes about 25 seconds. Skip it on a \
         metered connection.",
        "Opóźnienie na bezczynnym łączu mówi niewiele. Liczy się to, co się z nim dzieje, gdy \
         ktoś w domu zaczyna pobierać plik. Ten test wysyca łącze i mierzy wzrost.\n\n\
         Pobiera kilkadziesiąt MB z Cloudflare i trwa około 25 sekund. Pomiń go na połączeniu \
         z limitem transferu.";
    bloat_btn_run => "Run test", "Uruchom test";
    bloat_card_idle => "Idle latency", "Opóźnienie bezczynne";
    bloat_card_loaded => "Under load", "Pod obciążeniem";
    bloat_card_increase => "Increase", "Wzrost";
    bloat_card_throughput => "Throughput", "Przepustowość";
    bloat_card_throughput_sub => "during the test", "w trakcie testu";
    bloat_card_grade => "Grade", "Ocena";

    bloat_prog_idle => "Measuring idle latency…", "Pomiar opóźnienia bezczynnego…";
    bloat_prog_load =>
        "Saturating the link and measuring latency under load…",
        "Wysycanie łącza i pomiar opóźnienia pod obciążeniem…";
    bloat_prog_done => "Done", "Gotowe";

    grade_a =>
        "Excellent — the link does not bloat under load.",
        "Doskonale — łącze nie puchnie pod obciążeniem.";
    grade_b => "Good — a mild rise, unnoticeable in games.", "Dobrze — lekki wzrost, w grach niezauważalny.";
    grade_c =>
        "Fair — latency climbs noticeably while downloading.",
        "Średnio — opóźnienie wyraźnie rośnie podczas pobierania.";
    grade_d =>
        "Poor — games will stutter during any download.",
        "Słabo — gry będą się ciąć przy każdym pobieraniu.";
    grade_f => "Very poor — textbook bufferbloat.", "Bardzo słabo — podręcznikowy bufferbloat.";
    grade_unknown => "Not measured.", "Nie zmierzono.";

    bloat_silent_under_load =>
        "Under load the host stopped replying entirely — that is itself the result.",
        "Pod obciążeniem host przestał odpowiadać całkowicie — to już jest wynik.";
    bloat_advice_run => "Run the test to get a result.", "Uruchom test, żeby zobaczyć wynik.";
    bloat_advice_ok =>
        "Nothing to do — the link copes with load. If you still get lag, the cause is elsewhere \
         (Wi-Fi, driver, or the route to that particular server).",
        "Nie ma co poprawiać — łącze radzi sobie z obciążeniem. Jeśli mimo to masz lagi, \
         przyczyna leży gdzie indziej (Wi-Fi, sterownik albo trasa do konkretnego serwera).";
    bloat_advice_header => "What helps, most effective first:", "Co pomaga, od najskuteczniejszego:";
    bloat_advice_1 =>
        "1. Enable SQM / Smart Queue / QoS on the router (look for \"cake\" or \"fq_codel\") and \
         cap it at about 90% of the real line speed.",
        "1. Włącz SQM / Smart Queue / QoS na routerze (szukaj „cake” albo „fq_codel”) i ustaw \
         limit na około 90% rzeczywistej prędkości łącza.";
    bloat_advice_2 =>
        "2. If the router has no such option, that is the best possible reason to replace it. No \
         Windows setting can fix this.",
        "2. Jeśli router nie ma takiej opcji, to najlepszy możliwy powód, żeby go wymienić. Żadne \
         ustawienie Windowsa tego nie naprawi.";
    bloat_advice_3 =>
        "3. As a stopgap, throttle whatever saturates the link (Steam, torrents, updates) to \
         about 80% of capacity.",
        "3. Doraźnie ogranicz to, co wysyca łącze (Steam, torrenty, aktualizacje) do około 80% \
         przepustowości.";

    // -----------------------------------------------------------------------
    // exported report
    //
    // Section headings and row labels follow the UI language. Row labels are
    // padded to a fixed width by the caller, so they stay aligned whatever
    // their length.
    // -----------------------------------------------------------------------
    rep_title => "NetDoctor report", "Raport NetDoctor";
    rep_sec_connection => "CONNECTION", "POŁĄCZENIE";
    rep_sec_measurements => "MEASUREMENTS (last hour)", "POMIARY (ostatnia godzina)";
    rep_sec_outages => "OUTAGES (last 24 hours)", "AWARIE (ostatnie 24 godziny)";
    rep_sec_bloat => "LATENCY UNDER LOAD (bufferbloat)", "OPÓŹNIENIE POD OBCIĄŻENIEM (bufferbloat)";
    rep_sec_diagnosis => "DIAGNOSIS", "DIAGNOZA";
    rep_sec_changes => "SETTINGS CHANGES", "ZMIANY USTAWIEŃ";

    rep_adapter => "adapter", "karta";
    rep_driver => "driver", "sterownik";
    rep_gateway => "gateway", "brama";
    rep_dns => "DNS", "DNS";
    rep_ssid => "SSID", "SSID";
    rep_signal => "signal", "sygnał";
    rep_rates => "rates", "prędkości";
    rep_link_speed => "link speed", "prędkość łącza";
    rep_idle => "idle", "bezczynne";
    rep_loaded => "loaded", "obciążone";
    rep_increase => "increase", "wzrost";
    rep_throughput => "throughput", "przepustowość";
    rep_grade => "grade", "ocena";
    rep_none => "none", "brak";
    rep_no_reply => "no reply", "brak odpowiedzi";

    // -----------------------------------------------------------------------
    // tweaks: shared vocabulary and errors
    // -----------------------------------------------------------------------
    risk_low => "low", "niskie";
    risk_medium => "medium", "średnie";

    tw_needs_admin =>
        "This change requires administrator rights.",
        "Ta zmiana wymaga uprawnień administratora.";
    tw_revert_needs_admin =>
        "Reverting requires administrator rights.",
        "Cofnięcie wymaga uprawnień administratora.";
    tw_no_snapshot =>
        "No saved state for this change — nothing to revert to.",
        "Brak zapisanego stanu dla tej zmiany — nie ma do czego wrócić.";
    tw_state_unreadable => "cannot read", "nie można odczytać";
    tw_not_set => "not set", "nie ustawione";

    // adapter power saving
    tw_power_title =>
        "Stop Windows powering down the network adapter",
        "Zablokuj wyłączanie zasilania karty sieciowej przez Windows";
    tw_power_what =>
        "Clears \"Allow the computer to turn off this device to save power\" for the adapter.",
        "Odznacza „Zezwalaj komputerowi na wyłączanie tego urządzenia w celu oszczędzania \
         energii” dla karty.";
    tw_power_why =>
        "The most common cause of connections dropping \"for no reason\" on laptops. Windows \
         suspends the card when idle and waking it takes long enough for sessions to die.",
        "Najczęstsza przyczyna zrywania połączeń „bez powodu” na laptopach. Windows usypia kartę \
         w bezczynności, a jej wybudzenie trwa na tyle długo, że sesje się rozpadają.";
    tw_power_no_adapter =>
        "adapter not found in the registry",
        "nie znaleziono karty w rejestrze";
    tw_power_off => "disabled", "wyłączone";
    tw_power_on_unset =>
        "enabled — Windows may suspend the card (value not set)",
        "włączone — Windows może uśpić kartę (wartość nieustawiona)";
    tw_power_applied =>
        "Power management disabled for the adapter. Takes effect after a restart (or disabling \
         and re-enabling the adapter).",
        "Zarządzanie energią karty wyłączone. Zadziała po ponownym uruchomieniu (albo po \
         wyłączeniu i włączeniu karty).";
    tw_power_reverted =>
        "Previous value restored. Takes effect after a restart.",
        "Przywrócono poprzednią wartość. Zadziała po ponownym uruchomieniu.";

    // Wi-Fi power plan
    tw_wlan_title =>
        "Wi-Fi radio at maximum performance in the power plan",
        "Radio Wi-Fi na maksymalnej wydajności w planie zasilania";
    tw_wlan_what =>
        "Sets Wireless Adapter Settings → Power Saving Mode to Maximum Performance, on both \
         mains and battery.",
        "Ustawia Ustawienia karty bezprzewodowej → Tryb oszczędzania energii na Maksymalna \
         wydajność, zarówno na zasilaniu sieciowym, jak i na baterii.";
    tw_wlan_why =>
        "A second, independent throttle. Even with driver power management off, the power plan \
         can still cut transmit power and cause drops.",
        "Drugi, niezależny dławik. Nawet przy wyłączonym zarządzaniu energią w sterowniku plan \
         zasilania nadal może obciąć moc nadawania i powodować zrywy.";
    tw_wlan_absent =>
        "setting not present in this power plan",
        "ustawienie nieobecne w tym planie zasilania";
    tw_wlan_max_perf => "max performance", "maks. wydajność";
    tw_wlan_low_save => "low saving", "niskie oszczędzanie";
    tw_wlan_med_save => "medium saving", "średnie oszczędzanie";
    tw_wlan_max_save => "max saving", "maks. oszczędzanie";
    tw_wlan_applied =>
        "Wi-Fi radio set to maximum performance on mains and battery.",
        "Radio Wi-Fi ustawione na maksymalną wydajność na zasilaniu sieciowym i na baterii.";
    tw_wlan_reverted =>
        "Previous power plan values restored.",
        "Przywrócono poprzednie wartości planu zasilania.";

    // DNS
    tw_dns_title => "Fast, independent DNS servers", "Szybkie, niezależne serwery DNS";
    tw_dns_what =>
        "Sets 1.1.1.1 and 8.8.8.8 on the active adapter instead of the DHCP-supplied servers.",
        "Ustawia 1.1.1.1 i 8.8.8.8 na aktywnej karcie zamiast serwerów z DHCP.";
    tw_dns_why =>
        "When the router is the only resolver, its hiccup looks exactly like \"the internet is \
         down\": pings by IP work, but nothing loads.",
        "Gdy router jest jedynym resolverem, jego zadyszka wygląda dokładnie jak „nie ma \
         internetu”: ping po IP działa, ale nic się nie ładuje.";
    tw_dns_none => "none / from DHCP", "brak / z DHCP";
    tw_dns_router_only_note =>
        "  (router only — single point of failure)",
        "  (tylko router — pojedynczy punkt awarii)";
    tw_dns_applied => "DNS set to 1.1.1.1 and 8.8.8.8.", "DNS ustawiony na 1.1.1.1 i 8.8.8.8.";
    tw_dns_reverted_dhcp => "DNS returned to DHCP.", "DNS wrócił do DHCP.";
    tw_dns_reverted => "Previous DNS servers restored.", "Przywrócono poprzednie serwery DNS.";

    // TCP autotuning
    tw_autotune_title =>
        "TCP receive window auto-tuning = normal",
        "TCP auto-tuning okna odbiorczego = normal";
    tw_autotune_why =>
        "\"Ping boost\" guides tell people to disable this, which cripples throughput on any \
         fast link. Normal is the correct value; this undoes that damage.",
        "Poradniki „na lepszy ping” każą to wyłączać, co rujnuje przepustowość na każdym szybkim \
         łączu. Normal to wartość poprawna; to cofa tamtą szkodę.";
    tw_autotune_applied => "Auto-tuning set to normal.", "Auto-tuning ustawiony na normal.";

    // Nagle
    tw_nagle_title => "Disable Nagle's algorithm (for games)", "Wyłącz algorytm Nagle'a (pod gry)";
    tw_nagle_what =>
        "Writes TcpAckFrequency=1 and TCPNoDelay=1 for the active interface.",
        "Zapisuje TcpAckFrequency=1 i TCPNoDelay=1 dla aktywnego interfejsu.";
    tw_nagle_why =>
        "Windows buffers small packets and delays acknowledgements. In twitch games that is a \
         few to a dozen extra milliseconds. It makes no difference to downloads.",
        "Windows buforuje małe pakiety i opóźnia potwierdzenia. W dynamicznych grach to kilka do \
         kilkunastu dodatkowych milisekund. Na pobieranie nie ma wpływu.";
    tw_nagle_no_guid => "adapter GUID unknown", "nieznany GUID karty";
    tw_no_adapter_key => "adapter key not found", "nie znaleziono klucza karty";
    tw_snapshot_no_key => "snapshot has no key", "zrzut nie zawiera klucza";
    tw_nagle_applied => "Nagle disabled. Requires a restart.", "Nagle wyłączony. Wymaga ponownego uruchomienia.";
    tw_nagle_reverted =>
        "Previous state restored. Requires a restart.",
        "Przywrócono poprzedni stan. Wymaga ponownego uruchomienia.";

    // multimedia throttle
    tw_throttle_title => "Lift the multimedia packet throttle", "Zdejmij ograniczenie pakietów multimedialnych";
    tw_throttle_why =>
        "Windows caps network traffic at roughly 10k packets/s while any multimedia playback is \
         running. Gaming with a stream or music on shows this up as lag.",
        "Windows ogranicza ruch sieciowy do około 10 tys. pakietów/s, gdy cokolwiek odtwarza \
         multimedia. Granie przy włączonym streamie albo muzyce objawia się wtedy lagami.";
    tw_throttle_default_10 => "default (10)", "domyślne (10)";
    tw_throttle_default_20 => "default (20)", "domyślne (20)";
    tw_throttle_applied => "Throttle lifted. Requires a restart.", "Ograniczenie zdjęte. Wymaga ponownego uruchomienia.";
    tw_throttle_reverted =>
        "Defaults restored. Requires a restart.",
        "Przywrócono wartości domyślne. Wymaga ponownego uruchomienia.";

    // MTU
    tw_mtu_title => "Correct the adapter MTU", "Popraw MTU karty";
    tw_mtu_what =>
        "Sets MTU to the largest size that survives a fragmentation test (usually 1500, or 1492 \
         on PPPoE).",
        "Ustawia MTU na największy rozmiar, który przechodzi test fragmentacji (zwykle 1500, a \
         na PPPoE 1492).";
    tw_mtu_why =>
        "An MTU that is too large means packets get dropped somewhere along the path. The \
         symptom is pages that never finish loading while ping works fine.",
        "Zbyt duże MTU oznacza, że pakiety są gdzieś po drodze odrzucane. Objaw: strony, które \
         nigdy się nie doładowują, mimo że ping działa bez zarzutu.";
    tw_mtu_unknown => "MTU unknown", "MTU nieznane";
    tw_mtu_probe_failed =>
        "MTU probe produced no result (is DF-flagged ICMP blocked?)",
        "Sonda MTU nic nie zwróciła (czy ICMP z flagą DF jest blokowane?)";

    // stack reset
    tw_reset_title => "Reset the network stack (repair action)", "Zresetuj stos sieciowy (akcja naprawcza)";
    tw_reset_why =>
        "For when the connection has already died and will not come back. Clears broken \
         Winsock/IP state that otherwise persists until a reboot.",
        "Na wypadek, gdy połączenie już padło i nie wraca. Czyści zepsuty stan Winsock/IP, który \
         inaczej utrzymuje się aż do restartu.";
    tw_reset_state =>
        "one-off action — nothing is permanently changed",
        "akcja jednorazowa — nic nie zmienia się na stałe";
    tw_reset_step_ok => "ok", "ok";
    tw_reset_step_failed => "failed", "niepowodzenie";
    tw_reset_irreversible => "This action cannot be undone.", "Tej akcji nie da się cofnąć.";

    // -----------------------------------------------------------------------
    // autostart
    // -----------------------------------------------------------------------
    auto_enabled =>
        "NetDoctor will start minimised when you sign in.",
        "NetDoctor uruchomi się zminimalizowany przy logowaniu.";
    auto_disabled => "Autostart disabled.", "Autostart wyłączony.";
    auto_elevation_declined =>
        "Elevation was declined or failed.",
        "Podniesienie uprawnień zostało odrzucone lub się nie powiodło.";

    // -----------------------------------------------------------------------
    // optimise tab
    // -----------------------------------------------------------------------
    opt_blurb =>
        "Every change records the previous value first, so each one can be reverted individually \
         — including after a reboot.",
        "Każda zmiana najpierw zapisuje poprzednią wartość, więc da się ją cofnąć pojedynczo — \
         również po ponownym uruchomieniu komputera.";
    opt_btn_apply_all => "Apply all safe changes", "Zastosuj wszystkie bezpieczne zmiany";
    opt_needs_admin => "Requires administrator rights", "Wymaga uprawnień administratora";
    opt_read_only =>
        "read-only — restart as administrator to apply",
        "tylko do odczytu — uruchom ponownie jako administrator, żeby zastosować";
    opt_col_change => "Change", "Zmiana";
    opt_col_state => "Current state", "Stan obecny";
    opt_col_risk => "Risk", "Ryzyko";
    opt_select_tweak =>
        "Select a change to see what it does.",
        "Wybierz zmianę, żeby zobaczyć, co robi.";
    opt_nothing_to_revert => "Nothing saved to revert to", "Nie zapisano nic, do czego można wrócić";
    opt_needs_reboot => "takes effect after a restart", "działa po ponownym uruchomieniu";
    opt_irreversible => "cannot be undone", "nie da się cofnąć";
    opt_all_ok =>
        "Everything safe is already set correctly.",
        "Wszystko, co bezpieczne, jest już ustawione poprawnie.";
}

// ---------------------------------------------------------------------------
// Strings that take arguments. Same table, written out by hand because the
// placeholders differ between the two languages.
// ---------------------------------------------------------------------------

/// Toast shown when the connection comes back after an outage.
pub fn toast_restored(secs: f64) -> String {
    match current() {
        Lang::En => format!("Connection restored after {secs:.0} s. See the History tab."),
        Lang::Pl => {
            format!("Połączenie przywrócone po {secs:.0} s. Zobacz zakładkę Historia awarii.")
        }
    }
}

pub fn ping_api_failed(err: &str) -> String {
    match current() {
        Lang::En => format!("probe failed: {err}"),
        Lang::Pl => format!("sonda zawiodła: {err}"),
    }
}

// --- exported report -------------------------------------------------------

/// Column widths in the report's channel and measurement lines.
pub fn rep_channel_line(channel: &str, band: &str, phy: &str) -> String {
    match current() {
        Lang::En => format!("channel {channel} ({band}), {phy}"),
        Lang::Pl => format!("kanał {channel} ({band}), {phy}"),
    }
}

pub fn rep_rates_line(rx: u32, tx: u32) -> String {
    match current() {
        Lang::En => format!("{rx} Mbps receive / {tx} Mbps transmit"),
        Lang::Pl => format!("odbiór {rx} Mbps / nadawanie {tx} Mbps"),
    }
}

pub fn rep_stats_line(
    label: &str,
    count: usize,
    loss: f64,
    avg: f64,
    min: f64,
    max: f64,
    jitter: f64,
) -> String {
    match current() {
        Lang::En => format!(
            "  {label:<14} samples {count:>5}, loss {loss:>5.1}%, avg {avg:>7.2} ms, \
             min {min:>6.2}, max {max:>7.2}, jitter {jitter:>6.2}"
        ),
        Lang::Pl => format!(
            "  {label:<14} próbek {count:>5}, straty {loss:>5.1}%, śr. {avg:>7.2} ms, \
             min {min:>6.2}, maks {max:>7.2}, jitter {jitter:>6.2}"
        ),
    }
}

pub fn rep_loaded_line(avg: f64, max: f64, loss: f64) -> String {
    match current() {
        Lang::En => format!("{avg:.1} ms (max {max:.0}, loss {loss:.0}%)"),
        Lang::Pl => format!("{avg:.1} ms (maks {max:.0}, straty {loss:.0}%)"),
    }
}

// --- tweaks ----------------------------------------------------------------

pub fn tw_cannot_read(err: &str) -> String {
    match current() {
        Lang::En => format!("cannot read: {err}"),
        Lang::Pl => format!("nie można odczytać: {err}"),
    }
}

pub fn tw_power_off_detail(value: u32) -> String {
    format!("{} (PnPCapabilities={value})", tw_power_off())
}

pub fn tw_power_on_detail(value: u32) -> String {
    match current() {
        Lang::En => format!("enabled — Windows may suspend the card (PnPCapabilities={value})"),
        Lang::Pl => format!("włączone — Windows może uśpić kartę (PnPCapabilities={value})"),
    }
}

pub fn tw_wlan_state(ac: &str, dc: &str) -> String {
    match current() {
        Lang::En => format!("mains: {ac} / battery: {dc}"),
        Lang::Pl => format!("sieć: {ac} / bateria: {dc}"),
    }
}

pub fn tw_autotune_reverted(level: &str) -> String {
    match current() {
        Lang::En => format!("Auto-tuning restored to {level}."),
        Lang::Pl => format!("Auto-tuning przywrócony do {level}."),
    }
}

pub fn tw_mtu_state(mtu: u32) -> String {
    format!("MTU = {mtu}")
}

pub fn tw_mtu_applied(mtu: u32) -> String {
    match current() {
        Lang::En => format!("MTU set to {mtu}."),
        Lang::Pl => format!("MTU ustawione na {mtu}."),
    }
}

pub fn tw_mtu_reverted(mtu: u64) -> String {
    match current() {
        Lang::En => format!("MTU restored to {mtu}."),
        Lang::Pl => format!("MTU przywrócone do {mtu}."),
    }
}

pub fn tw_reset_done(steps: &str) -> String {
    match current() {
        Lang::En => format!("Ran: {steps}. A restart is recommended."),
        Lang::Pl => format!("Wykonano: {steps}. Zalecany restart."),
    }
}

// --- diagnostic scan -------------------------------------------------------

pub fn scan_critical(count: usize, first: &str) -> String {
    match current() {
        Lang::En => format!("{count} serious problem(s) found. Most important: {first}"),
        Lang::Pl => format!("Znaleziono poważnych problemów: {count}. Najważniejszy: {first}"),
    }
}

pub fn scan_warnings(count: usize, first: &str) -> String {
    match current() {
        Lang::En => {
            format!("No failures, but {count} thing(s) could be improved. Most important: {first}")
        }
        Lang::Pl => format!(
            "Nic nie jest zepsute, ale da się poprawić rzeczy: {count}. Najważniejsza: {first}"
        ),
    }
}

pub fn f_wired_detail(adapter: &str, mbps: u64) -> String {
    format!("{adapter}, {mbps} Mbps")
}

pub fn f_wifi_detail(adapter: &str, ssid: &str, mbps: u64) -> String {
    match current() {
        Lang::En => format!("{adapter} — {ssid}, {mbps} Mbps link rate"),
        Lang::Pl => format!("{adapter} — {ssid}, prędkość łącza {mbps} Mbps"),
    }
}

pub fn f_wifi_quality_detail(band: &str, channel: &str, phy: &str, rssi: &str, rx: u32) -> String {
    match current() {
        Lang::En => format!("Band {band}, channel {channel}, {phy}{rssi}, {rx} Mbps receive"),
        Lang::Pl => format!("Pasmo {band}, kanał {channel}, {phy}{rssi}, odbiór {rx} Mbps"),
    }
}

pub fn f_signal_weak(pct: u32) -> String {
    match current() {
        Lang::En => format!("Weak Wi-Fi signal ({pct}%)"),
        Lang::Pl => format!("Słaby sygnał Wi-Fi ({pct}%)"),
    }
}

pub fn f_signal_mid(pct: u32) -> String {
    match current() {
        Lang::En => format!("Mediocre Wi-Fi signal ({pct}%)"),
        Lang::Pl => format!("Przeciętny sygnał Wi-Fi ({pct}%)"),
    }
}

pub fn f_signal_good(pct: u32) -> String {
    match current() {
        Lang::En => format!("Wi-Fi signal is good ({pct}%)"),
        Lang::Pl => format!("Sygnał Wi-Fi jest dobry ({pct}%)"),
    }
}

pub fn f_dns_slow(ms: f64) -> String {
    match current() {
        Lang::En => format!("Slow DNS ({ms:.0} ms)"),
        Lang::Pl => format!("Wolny DNS ({ms:.0} ms)"),
    }
}

pub fn f_dns_ok(ms: f64) -> String {
    match current() {
        Lang::En => format!("DNS responds quickly ({ms:.0} ms)"),
        Lang::Pl => format!("DNS odpowiada szybko ({ms:.0} ms)"),
    }
}

pub fn f_dns_servers(servers: &str) -> String {
    match current() {
        Lang::En => format!("Servers: {servers}"),
        Lang::Pl => format!("Serwery: {servers}"),
    }
}

pub fn f_router_silent_detail(gw: &str) -> String {
    match current() {
        Lang::En => format!("10/10 packets lost to {gw}."),
        Lang::Pl => format!("10/10 pakietów zgubionych do {gw}."),
    }
}

/// The shared "avg / min / max / jitter / loss" line under a latency finding.
pub fn f_stats_line(avg: f64, min: f64, max: f64, jitter: f64, loss: f64) -> String {
    match current() {
        Lang::En => format!(
            "avg {avg:.1} ms, min {min:.1}, max {max:.1}, jitter {jitter:.1} ms, loss {loss:.0}%"
        ),
        Lang::Pl => format!(
            "śr. {avg:.1} ms, min {min:.1}, maks {max:.1}, jitter {jitter:.1} ms, straty {loss:.0}%"
        ),
    }
}

pub fn f_loss(pct: f64) -> String {
    match current() {
        Lang::En => format!("Packet loss {pct:.0}%"),
        Lang::Pl => format!("Utrata pakietów {pct:.0}%"),
    }
}

pub fn f_jitter_high(jitter: f64) -> String {
    match current() {
        Lang::En => format!("High jitter ({jitter:.0} ms)"),
        Lang::Pl => format!("Wysoki jitter ({jitter:.0} ms)"),
    }
}

pub fn f_jitter_high_advice(spread: f64) -> String {
    match current() {
        Lang::En => format!(
            "Latency swings by {spread:.0} ms between packets. This is what \"lagging despite a \
             good ping\" actually is — typical of Wi-Fi and of a saturated link."
        ),
        Lang::Pl => format!(
            "Opóźnienie waha się o {spread:.0} ms między pakietami. To właśnie jest „lagowanie \
             mimo dobrego pingu” — typowe dla Wi-Fi i wysyconego łącza."
        ),
    }
}

pub fn f_ping_high(avg: f64) -> String {
    match current() {
        Lang::En => format!("High latency ({avg:.0} ms)"),
        Lang::Pl => format!("Wysokie opóźnienie ({avg:.0} ms)"),
    }
}

pub fn f_net_ok(avg: f64) -> String {
    match current() {
        Lang::En => format!("Internet latency is normal ({avg:.0} ms)"),
        Lang::Pl => format!("Opóźnienie do internetu jest normalne ({avg:.0} ms)"),
    }
}

pub fn f_mtu_too_large_detail(cur: u32, best: u32) -> String {
    match current() {
        Lang::En => format!("Configured {cur}, but only {best} passes without fragmentation."),
        Lang::Pl => format!("Ustawione {cur}, ale bez fragmentacji przechodzi tylko {best}."),
    }
}

pub fn f_mtu_too_large_advice(best: u32) -> String {
    match current() {
        Lang::En => format!(
            "Packets above {best} get dropped along the way. The symptom is that ping works but \
             some pages never finish loading. Note that some paths rate-limit the probe, so it is \
             worth re-running this before changing anything."
        ),
        Lang::Pl => format!(
            "Pakiety powyżej {best} są po drodze odrzucane. Objaw: ping działa, ale część stron \
             nigdy się nie doładowuje. Uwaga: niektóre trasy ograniczają tempo sondy, więc warto \
             powtórzyć pomiar przed jakąkolwiek zmianą."
        ),
    }
}

pub fn f_mtu_ok(mtu: u32) -> String {
    match current() {
        Lang::En => format!("MTU looks correct ({mtu})"),
        Lang::Pl => format!("MTU wygląda poprawnie ({mtu})"),
    }
}

pub fn f_mtu_ok_detail(best: u32) -> String {
    match current() {
        Lang::En => format!("Largest unfragmented packet corresponds to MTU {best}."),
        Lang::Pl => format!("Największy niefragmentowany pakiet odpowiada MTU {best}."),
    }
}

pub fn f_hist_title(title: &str, count: usize) -> String {
    match current() {
        Lang::En => format!("{title} — {count}× in the last 24 h"),
        Lang::Pl => format!("{title} — {count}× w ciągu ostatnich 24 h"),
    }
}

pub fn f_hist_detail(avg: f64, times: &str) -> String {
    match current() {
        Lang::En => format!("Average duration {avg:.0} s. Most recent: {times}"),
        Lang::Pl => format!("Średni czas trwania {avg:.0} s. Ostatnie: {times}"),
    }
}

/// The test could not reach the host at all.
pub fn bloat_no_reply(host: &str) -> String {
    match current() {
        Lang::En => format!("No reply from {host} — test aborted."),
        Lang::Pl => format!("Brak odpowiedzi od {host} — test przerwany."),
    }
}

/// Opening line of the advice when the link does bloat.
pub fn bloat_advice_intro(bump: f64) -> String {
    match current() {
        Lang::En => format!(
            "Latency rises by {bump:.0} ms when the link is busy, which means packets are \
             queueing — either in your router or at the ISP."
        ),
        Lang::Pl => format!(
            "Opóźnienie rośnie o {bump:.0} ms, gdy łącze jest zajęte, czyli pakiety ustawiają się \
             w kolejce — w Twoim routerze albo u dostawcy."
        ),
    }
}

/// Closing line naming the measured speed to cap against.
pub fn bloat_advice_throughput(mbps: f64) -> String {
    match current() {
        Lang::En => {
            format!("Throughput measured during the test: {mbps:.0} Mbps — use that to set the cap.")
        }
        Lang::Pl => format!(
            "Przepustowość zmierzona w teście: {mbps:.0} Mbps — na tej podstawie ustaw limit."
        ),
    }
}

/// Human label for a stored event `kind`, which is a stable code in the
/// database and so can be translated after the fact.
pub fn event_kind(kind: &str) -> String {
    let (en, pl) = match kind {
        "degraded" => ("degraded quality", "pogorszona jakość"),
        "dns_fail" => ("DNS failure", "awaria DNS"),
        "isp_down" => ("provider down", "awaria u dostawcy"),
        "lan_down" => ("router unreachable", "router nieosiągalny"),
        "adapter_down" => ("adapter down", "karta sieciowa wyłączona"),
        other => return other.to_string(),
    };
    match current() {
        Lang::En => en.to_string(),
        Lang::Pl => pl.to_string(),
    }
}

/// Short name for a tweak, by the id stored in the `tweaks` table. Kept here
/// rather than on the `Tweak` trait so a change logged by a version that has
/// since dropped that tweak still reads as something.
pub fn tweak_name(id: &str) -> String {
    let (en, pl) = match id {
        "adapter_power" => ("adapter power saving", "oszczędzanie energii karty"),
        "fast_dns" => ("public DNS", "publiczny DNS"),
        "mtu" => ("MTU", "MTU"),
        "nagle_off" => ("Nagle off", "wyłączony Nagle"),
        "net_throttling" => ("network throttling", "network throttling"),
        "stack_reset" => ("network stack reset", "reset stosu sieciowego"),
        "tcp_autotuning" => ("TCP autotuning", "TCP autotuning"),
        "wlan_power_plan" => ("Wi-Fi power plan", "plan zasilania Wi-Fi"),
        other => return other.to_string(),
    };
    pick(en, pl)
}

#[inline]
fn pick(en: &str, pl: &str) -> String {
    match current() {
        Lang::En => en.to_string(),
        Lang::Pl => pl.to_string(),
    }
}

/// Headline for a cause code produced by `crate::cause`.
pub fn cause_title(code: &str) -> String {
    let (en, pl) = match code {
        "after_tweak" => (
            "A change applied just before this",
            "Zmiana zastosowana tuż przed awarią",
        ),
        "adapter_powered_down" => (
            "Windows put the Wi-Fi card to sleep",
            "Windows uśpił kartę Wi-Fi",
        ),
        "adapter_power_plan" => (
            "The power plan may be parking the radio",
            "Plan zasilania może wyłączać radio",
        ),
        "out_of_range" => ("Out of range of the access point", "Poza zasięgiem access pointa"),
        "adapter_or_driver" => (
            "The adapter disappeared — driver or hardware",
            "Karta zniknęła — sterownik albo sprzęt",
        ),
        "roaming" => (
            "Handover to another access point",
            "Przełączenie na inny access point",
        ),
        "signal_fade" => ("The signal faded away", "Sygnał stopniowo zanikał"),
        "airtime_24ghz" => (
            "The 2.4 GHz channel is crowded",
            "Kanał 2.4 GHz jest zatłoczony",
        ),
        "router_side" => (
            "The radio was fine — the router side was not",
            "Radio było w porządku — problem po stronie routera",
        ),
        "weak_signal" => ("Weak signal at the moment of the drop", "Słaby sygnał w chwili zerwania"),
        "marginal_link" => ("The link was marginal", "Łącze było na granicy"),
        "cable_or_router" => ("Cable or router, not Wi-Fi", "Kabel albo router, nie Wi-Fi"),
        "isp_sustained" => ("A sustained outage at the provider", "Dłuższa awaria u dostawcy"),
        "isp_brief" => ("A brief drop on the WAN side", "Krótki zryw po stronie WAN"),
        "isp_pattern" => ("The provider drops repeatedly", "Dostawca zrywa regularnie"),
        "dns_router_only" => (
            "The router is the only resolver",
            "Router jest jedynym resolverem",
        ),
        "dns_resolver" => ("The resolver did not answer", "Resolver nie odpowiedział"),
        "local_saturation" => (
            "The link to the router was saturated",
            "Łącze do routera było wysycone",
        ),
        "rate_collapse" => ("The Wi-Fi rate collapsed", "Prędkość Wi-Fi załamała się"),
        "time_pattern" => ("It happens at the same hour", "Zdarza się o tej samej godzinie"),
        "no_evidence" => ("No evidence was recorded", "Nie zapisano dowodów"),
        "unclear" => ("No single cause stands out", "Żadna przyczyna się nie wyróżnia"),
        other => return other.to_string(),
    };
    pick(en, pl)
}

/// What to do about a cause. Concrete enough to act on without a second tab.
pub fn cause_advice(code: &str) -> String {
    let (en, pl) = match code {
        "after_tweak" => (
            "Revert that change and watch whether the outages stop. If they do, the change is the \
             cause; if they carry on, put it back and look further down this list.",
            "Cofnij tę zmianę i sprawdź, czy awarie ustaną. Jeśli tak — to ona jest przyczyną; \
             jeśli nie — przywróć ją i szukaj niżej na tej liście.",
        ),
        "adapter_powered_down" => (
            "Turn off power saving on the adapter. This is the most common cause of a connection \
             that drops while the computer is idle and comes back the moment you touch it.",
            "Wyłącz oszczędzanie energii na karcie. To najczęstsza przyczyna zrywania połączenia, \
             gdy komputer stoi bezczynnie, a wraca ono, gdy tylko go dotkniesz.",
        ),
        "adapter_power_plan" => (
            "Set the wireless adapter to maximum performance in the active power plan. It is a \
             separate setting from the adapter's own power saving and both have to be off.",
            "Ustaw kartę bezprzewodową na maksymalną wydajność w aktywnym planie zasilania. To \
             osobne ustawienie od oszczędzania energii samej karty — oba muszą być wyłączone.",
        ),
        "out_of_range" | "weak_signal" => (
            "Move closer to the access point or add one. Below about -75 dBm a link stops being \
             usable no matter how fast the router is.",
            "Zbliż się do access pointa albo dodaj kolejny. Poniżej mniej więcej -75 dBm łącze \
             przestaje być używalne, niezależnie od tego, jak szybki jest router.",
        ),
        "adapter_or_driver" => (
            "Reinstall or roll back the adapter driver. If the adapter also vanishes from Device \
             Manager, suspect the hardware or its power supply.",
            "Przeinstaluj albo cofnij sterownik karty. Jeśli karta znika też z Menedżera \
             urządzeń, podejrzewaj sprzęt albo jego zasilanie.",
        ),
        "roaming" => (
            "The computer changed access point and the handover cost it the connection. If you \
             have several APs, giving each a distinct channel and matching their power usually \
             cures it; a single AP means the router itself switched band.",
            "Komputer zmienił access point i przełączenie kosztowało go połączenie. Jeśli masz \
             kilka AP, zwykle pomaga nadanie każdemu osobnego kanału i wyrównanie mocy; przy \
             jednym AP oznacza to, że router sam przełączył pasmo.",
        ),
        "signal_fade" => (
            "The signal was dropping steadily before the connection broke, so the computer or the \
             access point moved, or something came between them. This is not a router fault.",
            "Sygnał spadał miarowo, zanim połączenie padło — więc komputer albo access point \
             zmienił położenie, albo coś stanęło między nimi. To nie jest wina routera.",
        ),
        "airtime_24ghz" => (
            "2.4 GHz is shared with every neighbour, microwave and Bluetooth device around. Move \
             to 5 GHz if the adapter supports it, or pick channel 1, 6 or 11 — whichever is least \
             used nearby.",
            "2.4 GHz dzielisz z każdym sąsiadem, mikrofalówką i urządzeniem Bluetooth w okolicy. \
             Przejdź na 5 GHz, jeśli karta to obsługuje, albo wybierz kanał 1, 6 lub 11 — ten \
             najmniej obciążony w pobliżu.",
        ),
        "router_side" => (
            "The signal was strong and steady right up to the drop, so the radio link was not the \
             problem. Look at the router: its uptime, its temperature, its firmware, and whether \
             it drops other devices at the same moment.",
            "Sygnał był mocny i stabilny aż do zerwania, więc łącze radiowe nie było problemem. \
             Sprawdź router: czas pracy, temperaturę, firmware oraz to, czy zrywa w tym samym \
             momencie także innym urządzeniom.",
        ),
        "marginal_link" => (
            "It failed on a weak signal and recovered on a much stronger one, so the link sits \
             right at the edge of usable. Anything that nudges it — a door, a body, a microwave — \
             will keep breaking it.",
            "Zerwało się przy słabym sygnale, a wróciło przy znacznie mocniejszym, więc łącze \
             działa na samej granicy używalności. Cokolwiek je poruszy — drzwi, człowiek, \
             mikrofalówka — będzie je zrywać dalej.",
        ),
        "cable_or_router" => (
            "This is a wired link, so start with the cable and the port: reseat both ends, try \
             another port, try another cable. A failing cable looks exactly like a failing router.",
            "To łącze przewodowe, więc zacznij od kabla i portu: przepnij oba końce, spróbuj \
             innego portu, innego kabla. Psujący się kabel wygląda dokładnie jak psujący się \
             router.",
        ),
        "isp_sustained" | "isp_brief" => (
            "The router was answering the whole time, so the break was beyond it. Nothing on this \
             computer will fix that — but this log is the evidence to put in front of the \
             provider.",
            "Router odpowiadał przez cały czas, więc zerwanie było za nim. Nic na tym komputerze \
             tego nie naprawi — ale ten dziennik jest dowodem, który możesz przedstawić dostawcy.",
        ),
        "isp_pattern" => (
            "Repeated WAN drops are a service fault, not bad luck. Export the report and quote the \
             timestamps; a provider will engage with a list of dated outages and will not engage \
             with \"my internet is bad\".",
            "Powtarzające się zrywy WAN to usterka usługi, a nie pech. Wyeksportuj raport i podaj \
             znaczniki czasu — dostawca podejmie rozmowę o liście awarii z datami, a nie o \
             stwierdzeniu „internet mi nie działa”.",
        ),
        "dns_router_only" => (
            "Every name lookup goes through the router, so when its resolver stalls the internet \
             looks dead while it is in fact reachable. Add a public resolver alongside it.",
            "Każde zapytanie o nazwę idzie przez router, więc gdy jego resolver się zatnie, \
             internet wygląda na martwy, choć jest osiągalny. Dodaj obok publiczny resolver.",
        ),
        "dns_resolver" => (
            "Names stopped resolving while the network itself was up. Switching to a public \
             resolver is the quickest way to tell a resolver fault from a connection fault.",
            "Nazwy przestały się rozwiązywać, choć sieć działała. Przełączenie na publiczny \
             resolver to najszybszy sposób, żeby odróżnić awarię resolvera od awarii połączenia.",
        ),
        "local_saturation" => (
            "Latency to the router climbed before the quality dropped, so something on this side \
             filled the link — an upload, a backup, an update. Run the load test to confirm it and \
             to find the rate the link actually holds.",
            "Opóźnienie do routera rosło, zanim jakość spadła, więc coś po tej stronie zapchało \
             łącze — wysyłka, backup, aktualizacja. Uruchom test obciążeniowy, żeby to \
             potwierdzić i znaleźć przepustowość, którą łącze naprawdę utrzymuje.",
        ),
        "rate_collapse" => (
            "The negotiated Wi-Fi rate fell away before the quality did. That is interference or \
             distance, not the router's capacity — a faster router will not change it.",
            "Wynegocjowana prędkość Wi-Fi spadła, zanim spadła jakość. To zakłócenia albo \
             odległość, a nie wydajność routera — szybszy router tego nie zmieni.",
        ),
        "time_pattern" => (
            "Outages clustered at one hour of the day have a schedule behind them: a neighbour's \
             appliance, the router's nightly resync, a backup job, a provider maintenance window.",
            "Awarie skupione o jednej godzinie mają za sobą harmonogram: urządzenie sąsiada, \
             nocny resync routera, zadanie backupu, okno serwisowe dostawcy.",
        ),
        "no_evidence" => (
            "This outage was logged before the app kept the state that led up to it. Newer entries \
             carry it, so the next occurrence will be explainable.",
            "Ta awaria została zapisana, zanim aplikacja zachowywała stan ją poprzedzający. Nowsze \
             wpisy już go mają, więc następne wystąpienie da się wyjaśnić.",
        ),
        "unclear" => (
            "The recorded state does not single out one cause. If it repeats, the lead-up below is \
             the place to look for what changes each time.",
            "Zapisany stan nie wskazuje jednej przyczyny. Jeśli się powtórzy, przebieg poniżej \
             jest miejscem, w którym warto szukać tego, co za każdym razem się zmienia.",
        ),
        _ => ("", ""),
    };
    pick(en, pl)
}

// ---------------------------------------------------------------------------
// evidence lines: the numbers a verdict was read off
// ---------------------------------------------------------------------------

pub fn ev_after_tweak(tweak: &str, mins: i64) -> String {
    match current() {
        Lang::En => format!("\"{tweak}\" was applied {mins} min before this outage started"),
        Lang::Pl => format!("„{tweak}” zastosowano {mins} min przed początkiem tej awarii"),
    }
}

pub fn ev_adapter_powered_down(rssi: Option<i32>) -> String {
    let signal = match rssi {
        Some(r) => format!("{r} dBm"),
        None => pick("unknown", "nieznany"),
    };
    match current() {
        Lang::En => format!("the adapter went down while the signal was still {signal}"),
        Lang::Pl => format!("karta wyłączyła się, gdy sygnał wynosił jeszcze {signal}"),
    }
}

pub fn ev_adapter_power_plan() -> String {
    pick(
        "the active power plan is a second, independent way Windows parks the radio",
        "aktywny plan zasilania to drugi, niezależny sposób, w jaki Windows wyłącza radio",
    )
}

pub fn ev_rssi_low(rssi: i32) -> String {
    match current() {
        Lang::En => format!("signal was {rssi} dBm — below the usable threshold of about -75 dBm"),
        Lang::Pl => format!("sygnał wynosił {rssi} dBm — poniżej progu używalności około -75 dBm"),
    }
}

pub fn ev_adapter_absent() -> String {
    pick(
        "the adapter reported no link and no radio state at all",
        "karta nie zgłaszała ani łącza, ani żadnego stanu radia",
    )
}

pub fn ev_roam(from: &str, to: &str) -> String {
    match current() {
        Lang::En => format!("the access point changed from {from} to {to} just before the drop"),
        Lang::Pl => format!("access point zmienił się z {from} na {to} tuż przed zerwaniem"),
    }
}

pub fn ev_roam_flag() -> String {
    pick(
        "a handover to another access point was recorded within the preceding minute",
        "w ciągu poprzedzającej minuty odnotowano przełączenie na inny access point",
    )
}

pub fn ev_rssi_fade(from: i32, to: i32) -> String {
    let drop = from - to;
    match current() {
        Lang::En => format!("signal slid from {from} to {to} dBm — {drop} dB lost before the drop"),
        Lang::Pl => {
            format!("sygnał osunął się z {from} do {to} dBm — {drop} dB straty przed zerwaniem")
        }
    }
}

pub fn ev_crowded_24(channel: Option<u32>) -> String {
    let ch = channel.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
    match current() {
        Lang::En => {
            format!("the signal was strong and steady on 2.4 GHz channel {ch} right up to the drop")
        }
        Lang::Pl => {
            format!("sygnał był mocny i stabilny na kanale {ch} w paśmie 2.4 GHz aż do zerwania")
        }
    }
}

pub fn ev_signal_was_fine(rssi: i32) -> String {
    match current() {
        Lang::En => format!("signal held at {rssi} dBm through the whole lead-up"),
        Lang::Pl => format!("sygnał utrzymywał się na {rssi} dBm przez cały przebieg przed awarią"),
    }
}

pub fn ev_recovered_stronger(start: i32, end: i32) -> String {
    match current() {
        Lang::En => format!("it broke at {start} dBm and recovered at {end} dBm"),
        Lang::Pl => format!("zerwało się przy {start} dBm, a wróciło przy {end} dBm"),
    }
}

pub fn ev_wired() -> String {
    pick(
        "no radio state was recorded, so this link is wired",
        "nie zapisano żadnego stanu radia, więc to łącze przewodowe",
    )
}

pub fn ev_isp(duration: Option<f64>) -> String {
    let d = match duration {
        Some(d) if d >= 60.0 => format!("{:.0} min", d / 60.0),
        Some(d) => format!("{d:.0} s"),
        None => return pick("the router kept answering; it is still down", "router odpowiadał; awaria trwa"),
    };
    match current() {
        Lang::En => format!("the router kept answering for the whole {d}; only the WAN was gone"),
        Lang::Pl => format!("router odpowiadał przez całe {d}; zniknął tylko WAN"),
    }
}

pub fn ev_isp_pattern(count: usize) -> String {
    match current() {
        Lang::En => format!("{count} WAN outages are recorded in this history"),
        Lang::Pl => format!("w tej historii zapisano {count} awarii WAN"),
    }
}

pub fn ev_dns_router_only() -> String {
    pick(
        "the router is the only configured resolver, so its stall takes every lookup with it",
        "router jest jedynym skonfigurowanym resolverem, więc jego zacięcie zabiera wszystkie \
         zapytania",
    )
}

pub fn ev_dns_error(err: &str) -> String {
    if err.is_empty() {
        return pick("the test lookup did not complete", "testowe zapytanie nie zakończyło się");
    }
    match current() {
        Lang::En => format!("the test lookup failed: {err}"),
        Lang::Pl => format!("testowe zapytanie nie powiodło się: {err}"),
    }
}

pub fn ev_latency_climb(from: f64, to: f64) -> String {
    match current() {
        Lang::En => format!("round-trip to the router climbed from {from:.0} to {to:.0} ms"),
        Lang::Pl => format!("czas do routera wzrósł z {from:.0} do {to:.0} ms"),
    }
}

pub fn ev_rate_drop(from: u32, to: u32) -> String {
    match current() {
        Lang::En => format!("the Wi-Fi receive rate fell from {from} to {to} Mbps"),
        Lang::Pl => format!("prędkość odbioru Wi-Fi spadła z {from} do {to} Mbps"),
    }
}

pub fn ev_time_pattern(count: usize, hour: i64) -> String {
    match current() {
        Lang::En => format!("{count} of these outages started between {hour:02}:00 and {:02}:00", hour + 1),
        Lang::Pl => {
            format!("{count} z tych awarii zaczęło się między {hour:02}:00 a {:02}:00", hour + 1)
        }
    }
}

pub fn ev_none() -> String {
    pick(
        "this entry predates the app storing the state around an outage",
        "ten wpis powstał, zanim aplikacja zapisywała stan wokół awarii",
    )
}

pub fn ev_unclear() -> String {
    pick(
        "the recorded state matches no single pattern",
        "zapisany stan nie pasuje do żadnego pojedynczego wzorca",
    )
}

/// Heading over the cause panel, naming the outage being explained.
pub fn hist_cause_for(when: &str) -> String {
    match current() {
        Lang::En => format!("Outage of {when}"),
        Lang::Pl => format!("Awaria z {when}"),
    }
}

/// Ping by address works but names do not resolve.
pub fn mon_dns_detail(err: &str) -> String {
    match current() {
        Lang::En => format!("Ping by IP works, name resolution does not: {err}"),
        Lang::Pl => format!("Ping po IP działa, rozwiązywanie nazw nie: {err}"),
    }
}

/// The router is up but nothing beyond it is.
pub fn mon_isp_detail(gateway: &str) -> String {
    match current() {
        Lang::En => format!("Router at {gateway} answers, no internet host does."),
        Lang::Pl => format!("Router pod {gateway} odpowiada, żaden host w internecie nie."),
    }
}

pub fn mon_loss_detail(pct: f64) -> String {
    match current() {
        Lang::En => format!("{pct:.1}% packet loss over the last minute."),
        Lang::Pl => format!("{pct:.1}% utraconych pakietów w ciągu ostatniej minuty."),
    }
}

pub fn mon_jitter_detail(jitter: f64) -> String {
    match current() {
        Lang::En => {
            format!("Jitter {jitter:.0} ms — latency is swinging, which shows up as lag in games.")
        }
        Lang::Pl => format!(
            "Jitter {jitter:.0} ms — opóźnienie skacze, co w grach objawia się jako lagi."
        ),
    }
}

pub fn mon_ping_detail(worst: f64) -> String {
    match current() {
        Lang::En => format!("Ping {worst:.0} ms — above the playable threshold."),
        Lang::Pl => format!("Ping {worst:.0} ms — powyżej progu grywalności."),
    }
}

/// The worst sample seen while the link was saturated.
pub fn bloat_worst(max: f64, loss_pct: f64) -> String {
    match current() {
        Lang::En => format!("Worst sample under load: {max:.0} ms, packet loss {loss_pct:.0}%."),
        Lang::Pl => format!(
            "Najgorsza próbka pod obciążeniem: {max:.0} ms, utrata pakietów {loss_pct:.0}%."
        ),
    }
}

/// Labels in the tweak detail panel.
pub fn opt_what_it_does(text: &str) -> String {
    match current() {
        Lang::En => format!("What it does: {text}"),
        Lang::Pl => format!("Co robi: {text}"),
    }
}

pub fn opt_why_it_helps(text: &str) -> String {
    match current() {
        Lang::En => format!("Why it helps: {text}"),
        Lang::Pl => format!("Dlaczego pomaga: {text}"),
    }
}

pub fn opt_risk_note(label: &str) -> String {
    match current() {
        Lang::En => format!("Risk: {label}"),
        Lang::Pl => format!("Ryzyko: {label}"),
    }
}

/// Outcome of "apply all safe changes".
pub fn opt_applied(applied: usize) -> String {
    match current() {
        Lang::En => format!("Applied {applied} change(s). Some need a restart to take effect."),
        Lang::Pl => {
            format!("Zastosowano zmian: {applied}. Część zadziała po ponownym uruchomieniu.")
        }
    }
}

pub fn opt_applied_partial(applied: usize, failed: usize) -> String {
    match current() {
        Lang::En => {
            format!("Applied {applied}, failed {failed}. See the Optimise list for details.")
        }
        Lang::Pl => format!(
            "Zastosowano {applied}, nie udało się {failed}. Szczegóły na liście Optymalizacja."
        ),
    }
}

/// Headline over the outage table.
pub fn hist_summary(count: usize, where_text: &str) -> String {
    match current() {
        Lang::En => format!("{count} outage(s) in the last 24 hours, mostly {where_text}."),
        Lang::Pl => format!("{count} awarii w ciągu ostatnich 24 godzin, głównie {where_text}."),
    }
}

/// Sub-line under the latency card: the 5-minute range.
pub fn live_minmax(min: f64, max: f64) -> String {
    match current() {
        Lang::En => format!("min {min:.0} / max {max:.0} (5 min)"),
        Lang::Pl => format!("min {min:.0} / maks {max:.0} (5 min)"),
    }
}

/// Sub-line under the router card.
pub fn live_router_loss(pct: f64) -> String {
    match current() {
        Lang::En => format!("loss {pct:.1}%"),
        Lang::Pl => format!("strata {pct:.1}%"),
    }
}

/// Sub-line under the "since last outage" card.
pub fn live_outages_24h(count: usize) -> String {
    match current() {
        Lang::En => format!("{count} in 24 h"),
        Lang::Pl => format!("{count} w ciągu 24 h"),
    }
}

/// Where the report file landed.
pub fn live_report_saved(path: &str) -> String {
    match current() {
        Lang::En => format!("Report saved to {path}"),
        Lang::Pl => format!("Raport zapisany w {path}"),
    }
}

/// Settings saved, but some ping targets did not resolve.
pub fn set_saved_unresolved(list: &str) -> String {
    match current() {
        Lang::En => format!("Saved, but these could not be resolved: {list}"),
        Lang::Pl => format!("Zapisano, ale tych nie udało się rozwiązać: {list}"),
    }
}

/// Writing the settings file failed.
pub fn set_save_failed(err: &str) -> String {
    match current() {
        Lang::En => format!("Could not save: {err}"),
        Lang::Pl => format!("Nie udało się zapisać: {err}"),
    }
}

/// One-line summary of a Wi-Fi link, under the headline.
#[allow(clippy::too_many_arguments)]
pub fn conn_line_wifi(
    adapter: &str,
    ssid: &str,
    signal_pct: u32,
    rssi: &str,
    channel: &str,
    phy: &str,
    rx_mbps: u32,
    gateway: &str,
) -> String {
    match current() {
        Lang::En => format!(
            "{adapter} · {ssid} · signal {signal_pct}%{rssi} · channel {channel} ({phy}) · \
             {rx_mbps} Mbps · gateway {gateway}"
        ),
        Lang::Pl => format!(
            "{adapter} · {ssid} · sygnał {signal_pct}%{rssi} · kanał {channel} ({phy}) · \
             {rx_mbps} Mbps · brama {gateway}"
        ),
    }
}

/// One-line summary of a wired link, under the headline.
pub fn conn_line_wired(adapter: &str, mbps: u64, gateway: &str, dns: &str) -> String {
    match current() {
        Lang::En => format!("{adapter} · {mbps} Mbps · gateway {gateway} · DNS {dns}"),
        Lang::Pl => format!("{adapter} · {mbps} Mbps · brama {gateway} · DNS {dns}"),
    }
}

/// The language is global, and `cargo test` runs tests in parallel. Any test
/// that switches languages, or that asserts on translated text, holds this so
/// it cannot observe another test's language.
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Windows UI language, used only to pick a default on first run.
#[cfg(windows)]
pub fn detect() -> Lang {
    // Low 10 bits of the LANGID are the primary language; 0x15 is Polish.
    const LANG_POLISH: u16 = 0x15;
    let langid = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    if langid & 0x3ff == LANG_POLISH {
        Lang::Pl
    } else {
        Lang::En
    }
}

#[cfg(not(windows))]
pub fn detect() -> Lang {
    Lang::En
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores English afterwards: the table is global, and the other test
    /// modules assume the default.
    fn in_polish<T>(body: impl FnOnce() -> T) -> T {
        let _guard = test_lock();
        set(Lang::Pl);
        let out = body();
        set(Lang::En);
        out
    }

    #[test]
    fn switching_language_changes_the_accessors() {
        assert_eq!(tab_live(), "Live");
        assert_eq!(sev_critical(), "CRITICAL");

        in_polish(|| {
            assert_eq!(tab_live(), "Na żywo");
            assert_eq!(sev_critical(), "KRYTYCZNE");
        });

        assert_eq!(tab_live(), "Live");
    }

    #[test]
    fn parameterised_strings_follow_the_language_too() {
        assert!(scan_critical(2, "MTU").starts_with("2 serious"));
        in_polish(|| {
            let pl = scan_critical(2, "MTU");
            assert!(pl.starts_with("Znaleziono"), "{pl}");
            assert!(pl.contains("MTU"), "the argument must survive: {pl}");
        });
    }

    #[test]
    fn event_kinds_translate_and_unknown_codes_pass_through() {
        assert_eq!(event_kind("lan_down"), "router unreachable");
        in_polish(|| assert_eq!(event_kind("lan_down"), "router nieosiągalny"));
        // A code written by a future version must not become an empty cell.
        assert_eq!(event_kind("something_new"), "something_new");
    }

    #[test]
    fn polish_text_is_actually_polish() {
        in_polish(|| {
            // Guards against a key added with the English string pasted into
            // both slots, which compiles and silently ships untranslated.
            assert_ne!(mon_ok(), "Connection healthy");
            assert_ne!(f_wired(), "Wired connection");
            assert_ne!(tw_power_title(), "Stop Windows powering down the network adapter");
        });
    }
}
