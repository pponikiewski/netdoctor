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

        /// Every string in the table, for tests that have to check all of them.
        #[cfg(test)]
        const ALL_STRINGS: &[(&str, &str, &str)] = &[
            $((stringify!($key), $en, $pl),)*
        ];
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
    // Labels for the header's connection facts. Short on purpose: each one
    // sits directly in front of the value it names, so it only has to say
    // which value this is, not explain it.
    word_signal => "signal", "sygnał";
    word_channel => "channel", "kanał";
    word_gateway => "gateway", "brama";
    word_link => "link", "łącze";
    word_adapter => "adapter", "karta";
    word_wired => "Wired", "Kabel";

    hdr_administrator => "administrator", "administrator";
    hdr_restart_elevated => "Restart as administrator", "Uruchom ponownie jako administrator";
    hdr_standard_mode =>
        "standard mode, changes need elevation",
        "tryb zwykły, zmiany wymagają uprawnień administratora";
    hdr_no_adapter => "No active adapter.", "Brak aktywnej karty sieciowej.";

    // -----------------------------------------------------------------------
    // probes
    // -----------------------------------------------------------------------
    medium_unknown => "unknown", "nieznane";
    ping_unreachable => "host unreachable", "host nieosiągalny";
    ping_timeout => "no reply", "brak odpowiedzi";
    ping_no_route => "no route (adapter offline?)", "brak trasy (karta offline?)";
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
    --quick        with --scan, skip the load test (no line saturation)
    --minimised    start with the window minimised (used by autostart)
    --version      print the version and exit
    --help         print this help

Diagnostics work without elevation. Applying changes needs administrator
rights; the app offers to relaunch itself when you ask it to apply one.",
"UŻYCIE:
    netdoctor [OPCJE]

OPCJE:
    --scan         uruchom skan diagnostyczny w konsoli i zakończ
    --quick        razem z --scan: pomiń test obciążenia (bez saturacji łącza)
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

    // -----------------------------------------------------------------------
    // updates
    // -----------------------------------------------------------------------
    upd_section => "Updates", "Aktualizacje";
    upd_auto_check => "Check for updates on start", "Sprawdzaj aktualizacje przy starcie";
    upd_auto_check_hint =>
        "One request to GitHub when the app opens. Nothing is downloaded until you ask for it.",
        "Jedno zapytanie do GitHuba przy starcie aplikacji. Nic nie jest pobierane, dopóki sam \
         tego nie wybierzesz.";
    upd_btn_check => "Check now", "Sprawdź teraz";
    upd_btn_install => "Update and restart", "Zaktualizuj i uruchom ponownie";
    upd_btn_restart => "Restart now", "Uruchom ponownie teraz";
    upd_btn_later => "Later", "Później";
    upd_btn_page => "Release notes on GitHub", "Opis wydania na GitHubie";
    upd_checking => "Checking for updates…", "Sprawdzanie aktualizacji…";
    upd_downloading => "Downloading…", "Pobieranie…";
    upd_whats_new => "What's new", "Co nowego";
    upd_restart_hint =>
        "The new version is in place. It starts running when the app is restarted.",
        "Nowa wersja jest na miejscu. Zacznie działać po ponownym uruchomieniu aplikacji.";
    upd_err_not_exe =>
        "What was downloaded is not a Windows program. The release may be damaged.",
        "To, co zostało pobrane, nie jest programem Windows. Wydanie może być uszkodzone.";

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
    live_spike_explainer =>
        "All the targets are reached over the same Wi-Fi link, the same router and the same \
         uplink. A delay introduced anywhere on that shared stretch has to appear on all of them \
         in the same sweep, so a spike on one target alone cannot have come from there — it is \
         that one responder taking its time to answer a ping, which traffic passing through it \
         never waits for. Only the marked sweeps say anything about your connection.",
        "Wszystkie cele są osiągane przez to samo Wi-Fi, ten sam router i ten sam uplink. \
         Opóźnienie powstałe gdziekolwiek na tym wspólnym odcinku musi pojawić się na wszystkich \
         naraz, więc skok na jednym celu nie mógł stamtąd pochodzić — to ten jeden węzeł zwleka z \
         odpowiedzią na pinga, na co ruch przez niego przechodzący nigdy nie czeka. Tylko \
         oznaczone zamiatania mówią cokolwiek o Twoim łączu.";
    live_no_data => "no data", "brak danych";

    live_card_latency => "Latency", "Opóźnienie";
    live_card_latency_sub_none => "no data yet", "jeszcze brak danych";
    live_card_jitter => "Jitter", "Jitter";
    live_card_jitter_sub => "swing, last 5 min", "wahania, ostatnie 5 min";
    live_card_loss => "Packet loss", "Utrata pakietów";
    live_card_loss_sub => "last 5 minutes", "ostatnie 5 minut";
    live_card_router => "Router", "Router";
    live_card_router_none => "not answering", "nie odpowiada";
    live_card_dns => "DNS", "DNS";
    live_card_dns_err => "error", "błąd";
    live_card_dns_sub => "name lookup", "wyszukanie nazwy";
    live_card_uninterrupted => "Uninterrupted", "Bez przerw";
    live_card_uninterrupted_val => "24 h+", "24 h+";
    live_card_uninterrupted_sub => "no outages logged", "brak zapisanych awarii";
    live_card_since_outage => "Since last outage", "Od ostatniej awarii";

    // What each headline number actually is. The cards were six figures with
    // six technical names over them, which is a readout for someone who
    // already knows what they are looking at and a wall for everyone else.
    live_tip_latency =>
        "How long a packet takes to reach 1.1.1.1 and come back, averaged over the last five \
         minutes.

This is the one number that decides whether a call stutters or a game feels \
         late, and it has nothing to do with how fast things download.",
        "Ile czasu zajmuje pakietowi dotarcie do 1.1.1.1 i powrót, uśrednione z ostatnich pięciu \
         minut.

To ta jedna liczba decyduje o tym, czy rozmowa się tnie, a gra reaguje z \
         opóźnieniem, i nie ma związku z tym, jak szybko się pobiera.";
    live_tip_jitter =>
        "How much the latency jumps about from packet to packet.

A steady 80 ms is easier on a \
         call than an average of 40 ms that swings between 10 and 90, because the far end waits \
         for the slowest packet either way. This is the number that explains a call breaking up \
         while the latency looks fine.",
        "O ile opóźnienie skacze z pakietu na pakiet.

Równe 80 ms jest dla rozmowy łatwiejsze niż \
         średnia 40 ms skacząca między 10 a 90, bo druga strona i tak czeka na najwolniejszy \
         pakiet. To ta liczba tłumaczy rwaną rozmowę przy ładnie wyglądającym opóźnieniu.";
    live_tip_loss =>
        "The share of packets that never came back, over the last five minutes.

Whatever goes \
         missing has to be sent again, which is why a percent or two can hurt more than a high \
         latency. Steady loss points at the link; loss in short bursts usually means something \
         on the way was briefly overloaded.",
        "Udział pakietów, które nigdy nie wróciły, z ostatnich pięciu minut.

Co zginie, trzeba wysłać \
         ponownie — dlatego procent czy dwa potrafią zaszkodzić bardziej niż wysokie opóźnienie. \
         Stała strata wskazuje na łącze; strata w krótkich seriach zwykle oznacza chwilowe \
         przeciążenie czegoś po drodze.";
    live_tip_router =>
        "The trip to your own router and back — the first hop, and the only stretch of the path \
         that is entirely yours.

Over cable this sits well under a millisecond; over Wi-Fi a few \
         milliseconds is normal and tens of them mean interference or distance. If this number \
         is bad, every other number on this page is bad for the same reason.",
        "Droga do własnego routera i z powrotem — pierwszy skok i jedyny odcinek trasy w całości \
         Twój.

Po kablu jest to grubo poniżej milisekundy; po Wi-Fi kilka milisekund jest \
         normalne, a kilkadziesiąt oznacza zakłócenia albo odległość. Jeśli ta liczba jest zła, \
         wszystkie pozostałe na tej stronie są złe z tego samego powodu.";
    live_tip_dns =>
        "How long it took to turn a name like example.com into an address.

It is paid once when \
         you open a site you have not visited in a while, not on every packet, so it shows up as \
         a page hanging before it starts loading rather than as a slow connection. A slow \
         resolver is worth replacing, and the Optimise tab does it.",
        "Ile trwała zamiana nazwy w rodzaju example.com na adres.

Płaci się to raz, przy otwieraniu \
         strony, na której dawno nie byłeś, a nie przy każdym pakiecie — więc objawia się jako \
         strona, która chwilę wisi, zanim zacznie się ładować, a nie jako wolne łącze. Wolny \
         resolver warto wymienić, robi to zakładka Optymalizacja.";
    live_tip_uptime =>
        "An outage here means every target stopped answering at once, for long enough to count — \
         not one lost packet.

This is the figure to quote when reporting a fault, because it is \
         the one a provider cannot argue with.",
        "Awaria oznacza tutaj, że wszystkie cele przestały odpowiadać naraz, na tyle długo, by to \
         liczyć — a nie pojedynczy zgubiony pakiet.

Tę liczbę warto podać przy zgłaszaniu awarii, \
         bo z nią dostawca nie będzie dyskutował.";

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
        "No scan yet. The scan splits the chain — this PC, the router, the provider's first hop, \
         the open internet — and measures where latency and loss are actually introduced. It \
         then saturates the line to see whether latency survives a download, checks that real \
         TCP traffic gets through and not just ping, and compares everything against this \
         machine's own history rather than a generic threshold.",
        "Jeszcze nie było skanu. Skan rozcina łańcuch — ten komputer, router, pierwszy węzeł \
         dostawcy, otwarty internet — i mierzy, gdzie naprawdę powstaje opóźnienie i gdzie giną \
         pakiety. Potem obciąża łącze, żeby sprawdzić, czy opóźnienie przeżyje pobieranie, \
         weryfikuje, czy przechodzi realny ruch TCP, a nie tylko ping, i porównuje wszystko z \
         własną historią tego komputera zamiast ze sztywnym progiem.";
    diag_select_finding =>
        "Select a finding to see what it means.",
        "Wybierz wynik, żeby zobaczyć, co oznacza.";
    diag_btn_fix => "Fix this", "Napraw to";
    diag_deep => "Include the load test", "Dołącz test obciążeniowy";
    diag_deep_hint =>
        "Adds about 20 seconds and briefly saturates the line. Without it the scan cannot see \
         bufferbloat, which is the usual reason a fast connection feels slow.",
        "Wydłuża skan o około 20 sekund i na chwilę obciąża łącze do pełna. Bez tego skan nie \
         zobaczy bufferbloatu, a to zwykle on sprawia, że szybkie łącze wydaje się wolne.";

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
        "No lead-up was recorded for this outage. It was logged by an earlier version.",
        "Dla tej awarii nie zapisano przebiegu. Wpis pochodzi z wcześniejszej wersji.";
    hist_no_state =>
        "No connection state was stored for this entry.",
        "Dla tego wpisu nie zapisano stanu połączenia.";
    hist_leadup_rssi => "Signal (dBm)", "Sygnał (dBm)";
    hist_leadup_rtt => "Router (ms)", "Router (ms)";
    hist_tweaks_heading => "Changes applied shortly before", "Zmiany zastosowane krótko przed";
    // -----------------------------------------------------------------------
    // the per-hop path
    // -----------------------------------------------------------------------
    live_path_heading => "Where the loss starts", "Gdzie zaczyna się strata";
    live_path_waiting =>
        "Walking the path and measuring each hop. The first figures arrive in about a minute.",
        "Sprawdzam ścieżkę i mierzę każdy skok. Pierwsze liczby za mniej więcej minutę.";
    live_path_clean =>
        "Nothing on the path is losing packets or adding delay.",
        "Nic na ścieżce nie gubi pakietów ani nie dokłada opóźnienia.";
    live_path_note =>
        "A hop that shows loss while the hops behind it do not is rate-limiting its own replies, \
         not dropping your traffic. Only loss that continues to the end of the path is counted.",
        "Skok, który pokazuje stratę, podczas gdy skoki za nim nie, ogranicza tempo własnych \
         odpowiedzi, a nie gubi twojego ruchu. Liczy się tylko strata, która trwa do końca \
         ścieżki.";
    live_trace_heading => "Traceroute to 1.1.1.1", "Traceroute do 1.1.1.1";
    live_trace_empty =>
        "Not run yet. It walks the whole route once, including the hops the \
         continuous measurement on the left leaves out.",
        "Jeszcze nieuruchomiony. Przechodzi całą trasę raz, także te skoki, \
         które pomija ciągły pomiar po lewej.";
    live_scale_ok => "good", "dobre";
    live_range_label => "window", "okno";
    live_smooth => "trend", "trend";
    live_smooth_hint =>
        "Draw each slice's average instead of its range. Reads the shape of an hour; hides the \
         individual spikes, which the count below still reports.",
        "Rysuj średnią każdego wycinka zamiast jego rozpiętości. Pokazuje kształt godziny; ukrywa \
         pojedyncze skoki, o których i tak mówi licznik poniżej.";
    live_series_toggle => "click to hide this line", "kliknij, żeby ukryć tę linię";
    live_series_show => "click to show this line again", "kliknij, żeby pokazać tę linię z powrotem";

    // What a figure on a given line actually means. Four lines of numbers say
    // nothing until you know which stretch of the path each one covers: the
    // chart's whole point is that the segment where the number goes bad is
    // the segment the fault is in.
    live_meaning_lan =>
        "Your own router, one hop away over Wi-Fi or cable.\n\nA high figure here is inside your \
        home — Wi-Fi interference, distance from the router, or the router itself under load. \
        Nothing further out on the internet can cause it. Note that when this one climbs, every \
        other line climbs with it, because every packet passes through here first.",
        "Twój własny router, jeden skok przez Wi-Fi albo kabel.\n\nWysoka wartość tutaj oznacza \
        problem u Ciebie w domu — zakłócenia Wi-Fi, odległość od routera albo obciążony router. \
        Nic dalej w internecie nie może tego powodować. Gdy ta linia rośnie, rosną też wszystkie \
        pozostałe, bo każdy pakiet przechodzi najpierw tędy.";
    live_meaning_isp =>
        "The DNS server your provider handed you — the first machine outside your home.\n\nHigh \
        here while the router is low means the delay starts on the provider's link. High here \
        while 1.1.1.1 and 8.8.8.8 stay low means only that one resolver is slow: it delays every \
        new site you open, and switching DNS on the Optimise tab takes it out of the path.",
        "Serwer DNS podany przez Twojego dostawcę — pierwsza maszyna poza Twoim domem.\n\nWysoko \
        tutaj przy niskim routerze oznacza, że opóźnienie zaczyna się na łączu dostawcy. Wysoko \
        tutaj, gdy 1.1.1.1 i 8.8.8.8 są niskie, oznacza, że wolny jest sam ten resolver: opóźnia \
        każdą nowo otwieraną stronę, a zmiana DNS w zakładce Optymalizacja wyjmuje go ze \
        ścieżki.";
    live_meaning_internet =>
        "A public server far out on the internet (1.1.1.1 is Cloudflare, 8.8.8.8 is \
        Google).\n\nBoth are built to answer instantly, so whatever you see here is the path, \
        not the server. This is the full route: your Wi-Fi, your router, your provider, and the \
        backbone beyond them. High on these while the router stays low puts the fault outside \
        your home.\n\nHigh on one of them alone, with the other fine, is that single route \
        having a bad moment and is not something you can fix.",
        "Publiczny serwer daleko w internecie (1.1.1.1 to Cloudflare, 8.8.8.8 to Google).\n\nOba \
        są zbudowane tak, by odpowiadać natychmiast, więc to, co tu widzisz, to stan trasy, a \
        nie serwera. To cała droga: Twoje Wi-Fi, Twój router, Twój dostawca i sieć szkieletowa \
        za nimi. Wysoko tutaj przy niskim routerze oznacza, że wina leży poza Twoim \
        domem.\n\nWysoko na jednym z nich, gdy drugi jest w porządku, to gorszy moment tej \
        jednej trasy — tego nie naprawisz.";
    live_meaning_custom =>
        "A target you added yourself. It is measured the same way as the rest: the figure covers \
        the whole path to it, so compare it against 1.1.1.1 to tell the route apart from the \
        host at the end of it.",
        "Cel dodany przez Ciebie. Mierzony tak samo jak reszta: wartość obejmuje całą drogę do \
        niego, więc porównaj ją z 1.1.1.1, żeby odróżnić stan trasy od stanu samego hosta na jej \
        końcu.";

    // Said once, on the axis caption. The unit is the thing the whole tab is
    // made of and it was never defined anywhere.
    live_ms_explainer =>
        "A millisecond is a thousandth of a second. The figure is how long one small packet took \
        to travel there and back, so it is a round trip, not a one-way distance, and it is not \
        about download speed at all — a fast connection can have poor latency and the other way \
        round. Under 30 ms is quick, video calls and games start to suffer past roughly 100 ms.",
        "Milisekunda to tysięczna część sekundy. Wartość mówi, ile mały pakiet leciał tam i z \
        powrotem — to podróż w obie strony, nie odległość w jedną. Nie ma to nic wspólnego z \
        prędkością pobierania: szybkie łącze może mieć kiepskie opóźnienie i odwrotnie. Poniżej \
        30 ms jest szybko, rozmowy wideo i gry zaczynają cierpieć powyżej mniej więcej 100 ms.";

    live_scale_mid => "elevated", "podwyższone";

    // The key under the plot. Each entry sits next to a swatch in the colour
    // it describes, so it no longer has to name the colour in words: "red
    // line = lost packet" was saying in text what the swatch says by being
    // red, and the row was long enough already.
    // The row had no name, so it read as loose text that happened to sit
    // under a chart rather than as the chart's key.
    live_key_heading => "Chart key", "Legenda wykresu";
    live_key_lost => "lost packet", "zgubiony pakiet";
    live_key_spike => "spike on every target", "skok na wszystkich celach";
    live_key_bands => "thresholds", "progi";
    live_key_help => "what am I looking at?", "co tu widzę?";

    live_hover_lost => "no reply", "brak odpowiedzi";
    // The same instruction, in the form it takes inside the help rather than
    // squeezed onto the key row.
    live_hover_hint_long =>
        "Point anywhere on the chart to read every target's value at that exact moment, and the \
         three bands the colours stand for: good, elevated and poor, at the thresholds set on \
         the Settings tab.",
        "Najedź w dowolne miejsce wykresu, żeby odczytać wartość każdego celu dokładnie z tej \
         chwili. Kolory oznaczają trzy pasma: dobre, podwyższone i słabe, według progów \
         ustawionych w zakładce Ustawienia.";
    live_hover_spike =>
        "a spike hit several targets at once here",
        "w tym miejscu skok dotknął kilku celów naraz";
    live_scale_bad => "poor", "słabe";
    live_path_col_hop => "Hop", "Skok";
    live_path_col_addr => "Address", "Adres";
    live_path_col_owner => "Whose", "Czyje";
    live_path_col_loss => "Loss", "Strata";
    live_path_col_avg => "Average", "Średnia";
    // A hop that answered the walk but drops every probe addressed to it.
    // Common, harmless, and nothing like the same thing as losing packets.
    path_no_answer => "does not answer", "nie odpowiada";
    path_owner_gateway => "your router", "twój router";
    path_owner_local => "your network", "twoja sieć";
    path_owner_edge => "provider edge", "brzeg dostawcy";
    path_owner_internet => "beyond the provider", "za dostawcą";

    hist_log_heading => "What Windows wrote down", "Co zapisał Windows";
    hist_log_loading => "Reading the event log…", "Czytam dziennik zdarzeń…";
    hist_log_none =>
        "The Windows event log has nothing from this period. Either nothing on this machine \
         faulted, or the channels that would have said so are turned off.",
        "Dziennik zdarzeń Windows nie ma nic z tego okresu. Albo nic w tym komputerze nie \
         zawiodło, albo kanały, które by to zgłosiły, są wyłączone.";
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
    step_path => "Measuring every segment at once", "Pomiar wszystkich odcinków naraz";
    step_load => "Measuring behaviour under load", "Pomiar zachowania pod obciążeniem";
    step_mtu => "Checking MTU", "Sprawdzanie MTU";
    step_tcp => "Checking TCP settings", "Sprawdzanie ustawień TCP";
    step_history => "Reviewing outage history", "Przegląd historii awarii";
    step_done => "Done", "Gotowe";

    scan_all_healthy =>
        "The network looks healthy. Nothing needs attention.",
        "Sieć wygląda zdrowo. Nic nie wymaga uwagi.";

    // -----------------------------------------------------------------------
    // the verdict: which segment of the chain is at fault, and what it costs
    // -----------------------------------------------------------------------
    verdict_heading => "Where the problem is", "Gdzie leży problem";
    verdict_cost_heading => "What it costs you", "Ile Cię to kosztuje";
    verdict_actions_heading => "What to do, in order", "Co zrobić, po kolei";
    verdict_none =>
        "Too little was measured to point at a segment.",
        "Zmierzono za mało, żeby wskazać odcinek.";

    seg_lan => "Between this PC and the router", "Między tym komputerem a routerem";
    // Named apart from both neighbours on purpose: the queue physically sits in
    // the router, the congestion is felt on the provider's uplink, and the fix
    // belongs to neither party alone.
    seg_uplink => "In the outbound queue (router ↔ provider)", "W kolejce wyjściowej (router ↔ dostawca)";
    seg_isp => "On the provider's side", "Po stronie dostawcy";
    seg_internet => "Beyond the provider", "Poza dostawcą";
    seg_dns => "In DNS", "W DNS";
    seg_config => "In this machine's settings", "W ustawieniach tego komputera";
    seg_healthy => "Nowhere — the chain is sound", "Nigdzie — łańcuch jest sprawny";

    cost_none =>
        "Nothing measurable. Calls, games and streaming should all behave.",
        "Nic mierzalnego. Rozmowy, gry i streaming powinny działać bez zarzutu.";
    cost_down =>
        "Nothing gets through this segment right now, so everything that depends on it is \
         stopped rather than slow.",
        "Przez ten odcinek w tej chwili nic nie przechodzi, więc wszystko, co od niego zależy, \
         stoi, a nie działa wolno.";
    cost_intermittent =>
        "Everything measures clean right now, so the fault is not constant — but it was recorded \
         breaking within the last 24 hours. A scan is a thirty-second window; the outage history \
         is the better evidence here.",
        "W tej chwili wszystko mierzy się czysto, więc usterka nie jest stała — ale w ciągu \
         ostatnich 24 godzin zapisano jej wystąpienia. Skan to okno trzydziestu sekund; lepszym \
         dowodem jest tutaj historia awarii.";
    cost_config =>
        "The line itself measures clean. What is left are settings on this machine that work \
         against it — worth changing, but not the reason for a bad call.",
        "Samo łącze mierzy się czysto. Zostają ustawienia na tym komputerze, które mu szkodzą — \
         warto je zmienić, ale to nie one psują rozmowę.";

    // -----------------------------------------------------------------------
    // findings: segment isolation, real reachability, behaviour under load
    // -----------------------------------------------------------------------
    f_edge_unknown => "Provider's first hop is hidden", "Pierwszy węzeł dostawcy jest ukryty";
    f_edge_unknown_detail =>
        "The routers on the path do not answer expired packets, so latency cannot be split \
         between your router and the provider. Everything else in this scan still holds.",
        "Routery na trasie nie odpowiadają na wygasłe pakiety, więc nie da się rozdzielić \
         opóźnienia między Twój router a dostawcę. Reszta tego skanu pozostaje w mocy.";
    f_local_hop_advice =>
        "This hop is still your own equipment — a second router, a mesh node, or a modem left in \
         router mode. Its latency counts as your network, not the provider's, which is why the \
         split above charges it to the LAN.",
        "Ten węzeł to nadal Twój własny sprzęt — drugi router, węzeł mesh albo modem zostawiony w \
         trybie routera. Jego opóźnienie liczy się jako Twoja sieć, a nie dostawcy, i dlatego \
         podział powyżej przypisuje je do LAN-u.";
    f_local_hop_slow_advice =>
        "A second box of your own is adding this delay before the traffic even leaves the house. \
         Putting it into bridge mode, or removing it from the chain, recovers the whole amount.",
        "Drugie własne urządzenie dokłada to opóźnienie, zanim ruch w ogóle opuści dom. \
         Przełączenie go w tryb bridge albo wyjęcie z łańcucha odzyskuje całą tę wartość.";
    f_edge_lossy_advice =>
        "Packets are being dropped on the provider's first hop, not inside your home. No setting \
         on this machine changes that.",
        "Pakiety giną na pierwszym węźle dostawcy, a nie u Ciebie w domu. Żadne ustawienie na tym \
         komputerze tego nie zmieni.";
    f_edge_slow_advice =>
        "The hop into the provider's network is where the delay appears. Typical for a loaded \
         DSL or cable segment at peak hours.",
        "Opóźnienie pojawia się dopiero na wejściu do sieci dostawcy. Typowe dla obciążonego \
         segmentu DSL lub kablowego w godzinach szczytu.";

    f_tcp_blocked => "Ping works but real traffic does not", "Ping działa, ale ruch realny już nie";
    f_tcp_blocked_advice =>
        "ICMP gets through and TCP on port 443 does not. That is a captive portal waiting for a \
         login, a firewall, or a proxy — not a broken line. Open any page in a browser and see \
         what answers.",
        "ICMP przechodzi, a TCP na porcie 443 już nie. To captive portal czekający na \
         zalogowanie, firewall albo proxy — nie zepsute łącze. Otwórz dowolną stronę w \
         przeglądarce i zobacz, co odpowie.";
    f_tcp_slow_advice =>
        "The handshake takes far longer than the ping to the same place, which points at \
         filtering or an overloaded middlebox rather than at the line itself.",
        "Handshake trwa znacznie dłużej niż ping w to samo miejsce, co wskazuje na filtrowanie \
         albo przeciążony middlebox, a nie na samo łącze.";

    f_load_skipped => "Behaviour under load not measured", "Nie zmierzono zachowania pod obciążeniem";
    f_load_skipped_detail =>
        "The load test was left out of this scan. It is the single most informative check for \
         \"the internet feels slow\", because it is the only one that reproduces the condition.",
        "Test obciążeniowy został pominięty w tym skanie. To najbardziej wymowna pojedyncza \
         próba przy objawie „internet działa wolno\", bo jako jedyna odtwarza warunki awarii.";
    f_load_bad_advice =>
        "This is bufferbloat: while something downloads, everything else queues behind it. It \
         is fixed on the router with SQM or QoS, not on this machine.",
        "To bufferbloat: w trakcie pobierania wszystko inne czeka w kolejce za transferem. \
         Naprawia się to na routerze przez SQM lub QoS, a nie na tym komputerze.";
    f_load_ok_advice =>
        "The line holds its latency while saturated, so a download in the background will not \
         break a call or a game.",
        "Łącze trzyma opóźnienie przy pełnym obciążeniu, więc pobieranie w tle nie zepsuje \
         rozmowy ani gry.";

    f_baseline_advice =>
        "Compared against this machine's own median from the last seven days, not against a \
         generic threshold. A number that is normal for one line is a fault on another.",
        "Porównanie z własną medianą tego komputera z ostatnich siedmiu dni, a nie ze sztywnym \
         progiem. Wartość normalna dla jednego łącza jest awarią na innym.";

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
        "The monitor logs every interruption. Leave it running to catch the next one.",
        "Monitor zapisuje każdą przerwę. Zostaw go włączonego, żeby złapał następną.";
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
        "Router answers, internet does not: WAN/ISP problem",
        "Router odpowiada, internet nie: problem WAN/ISP";
    mon_lan_down =>
        "Router not answering: problem between PC and router",
        "Router nie odpowiada: problem między komputerem a routerem";
    mon_adapter_down => "Network adapter disconnected", "Karta sieciowa rozłączona";

    mon_wifi_deassociated =>
        "Wi-Fi card reports it is no longer associated with the network.",
        "Karta Wi-Fi zgłasza, że nie jest już powiązana z siecią.";
    mon_nothing_responded =>
        "Neither the router nor the internet responded.",
        "Nie odpowiedział ani router, ani internet.";
    mon_no_gateway =>
        "No default gateway. This machine has no route to the network.",
        "Brak bramy domyślnej. Ten komputer nie ma trasy do sieci.";

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
        "Excellent. The link does not bloat under load.",
        "Doskonale. Łącze nie puchnie pod obciążeniem.";
    grade_b => "Good. A mild rise, unnoticeable in games.", "Dobrze. Lekki wzrost, w grach niezauważalny.";
    grade_c =>
        "Fair. Latency climbs noticeably while downloading.",
        "Średnio. Opóźnienie wyraźnie rośnie podczas pobierania.";
    grade_d =>
        "Poor. Games will stutter during any download.",
        "Słabo. Gry będą się ciąć przy każdym pobieraniu.";
    grade_f => "Very poor. Textbook bufferbloat.", "Bardzo słabo. Podręcznikowy bufferbloat.";
    grade_unknown => "Not measured.", "Nie zmierzono.";

    bloat_silent_under_load =>
        "Under load the host stopped replying entirely. That is itself the result.",
        "Pod obciążeniem host przestał odpowiadać całkowicie. To już jest wynik.";
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
    rep_sec_path => "PATH, HOP BY HOP", "ŚCIEŻKA, SKOK PO SKOKU";
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
    risk_high => "high", "wysokie";

    tw_needs_admin =>
        "This change requires administrator rights.",
        "Ta zmiana wymaga uprawnień administratora.";
    tw_revert_needs_admin =>
        "Reverting requires administrator rights.",
        "Cofnięcie wymaga uprawnień administratora.";
    tw_no_snapshot =>
        "No saved state for this change. Nothing to revert to.",
        "Brak zapisanego stanu dla tej zmiany. Nie ma do czego wrócić.";
    tw_state_unreadable => "cannot read", "nie można odczytać";
    tw_snapshots_unreadable_hint =>
        "The saved-state file is damaged, so Revert is unavailable for every change. The file was not overwritten — it can still be repaired by hand.",
        "Plik zapisanych stanów jest uszkodzony, więc Cofnij jest niedostępne dla wszystkich zmian. Plik nie został nadpisany — nadal da się go naprawić ręcznie.";
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
        "enabled, Windows may suspend the card (value not set)",
        "włączone, Windows może uśpić kartę (wartość nieustawiona)";
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
         down\": pings by IP work, but nothing loads. In exchange every name you look up goes to \
         Cloudflare and Google instead of your ISP, and on a company network or a VPN the \
         internal names stop resolving. This only changes IPv4, so anything reaching a resolver \
         over IPv6 keeps using the old one.",
        "Gdy router jest jedynym resolverem, jego zadyszka wygląda dokładnie jak „nie ma \
         internetu”: ping po IP działa, ale nic się nie ładuje. W zamian każda rozwiązywana nazwa \
         trafia do Cloudflare i Google zamiast do dostawcy, a w sieci firmowej lub na VPN \
         przestaną działać nazwy wewnętrzne. Zmiana dotyczy wyłącznie IPv4, więc ruch idący do \
         resolvera po IPv6 zostaje przy starym.";
    tw_dns_none => "none / from DHCP", "brak / z DHCP";
    tw_dns_router_only_note =>
        "  (router only, single point of failure)",
        "  (tylko router, pojedynczy punkt awarii)";
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
        "one-off action, nothing is permanently changed",
        "akcja jednorazowa, nic nie zmienia się na stałe";
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
        "read-only (restart as administrator to apply)",
        "tylko do odczytu (uruchom ponownie jako administrator, żeby zastosować)";
    opt_select_tweak =>
        "Select a change to see what it does.",
        "Wybierz zmianę, żeby zobaczyć, co robi.";
    opt_nothing_to_revert => "Nothing saved to revert to", "Nie zapisano nic, do czego można wrócić";
    opt_needs_reboot => "takes effect after a restart", "działa po ponownym uruchomieniu";
    opt_irreversible => "cannot be undone", "nie da się cofnąć";
    opt_all_ok =>
        "Everything safe is already set correctly.",
        "Wszystko, co bezpieczne, jest już ustawione poprawnie.";

    // -----------------------------------------------------------------------
    // tweaks: the radio itself
    // -----------------------------------------------------------------------
    tw_adv_unsupported =>
        "this driver does not expose the setting",
        "ten sterownik nie udostępnia tego ustawienia";
    tw_adv_no_option =>
        "the driver offers no matching option",
        "sterownik nie oferuje pasującej opcji";

    tw_tx_title =>
        "Turn the Wi-Fi card's transmit power up to maximum",
        "Podnieś moc nadawania karty Wi-Fi do maksimum";
    tw_tx_what =>
        "Sets the adapter's Transmit Power property to the highest level its driver offers.",
        "Ustawia właściwość Transmit Power karty na najwyższy poziom oferowany przez \
         sterownik.";
    tw_tx_why =>
        "Laptops ship this below maximum to save battery, which shortens the range in the \
         direction that matters: the router hears the laptop less well than the laptop hears \
         the router, so uploads and acknowledgements fail first at the edge of the flat.",
        "Laptopy mają to fabrycznie poniżej maksimum, żeby oszczędzać baterię, co skraca \
         zasięg w tę stronę, która boli: router słyszy laptopa gorzej niż laptop routera, \
         więc na krańcu mieszkania pierwsze sypią się wysyłki i potwierdzenia.";

    tw_roam_title =>
        "Let the card switch to a stronger access point sooner",
        "Pozwól karcie szybciej przełączać się na mocniejszy access point";
    tw_roam_what =>
        "Sets roaming aggressiveness to the highest level the driver offers.",
        "Ustawia agresywność roamingu na najwyższy poziom oferowany przez sterownik.";
    tw_roam_why =>
        "With a mesh or a second access point, a cautious card clings to the one it joined \
         first until the signal is nearly gone, and everything is slow in the meantime. This \
         makes it hand over while there is still a good signal to hand over from. On a single \
         access point it does nothing, and in a noisy block of flats it can make the card hop \
         about — revert it if the connection starts stuttering.",
        "Przy mesh lub drugim access poincie ostrożna karta trzyma się tego, z którym się \
         połączyła, aż sygnał prawie zniknie, a w międzyczasie wszystko muli. To sprawia, że \
         przełącza się, póki jest jeszcze z czego. Przy jednym access poincie nic nie zmienia, \
         a w zagęszczonym bloku karta może zacząć skakać — wtedy cofnij.";

    tw_psm_title =>
        "Stop the Wi-Fi radio going to sleep between packets",
        "Nie pozwól radiu Wi-Fi zasypiać między pakietami";
    tw_psm_what =>
        "Sets the adapter's power save mode to maximum performance.",
        "Ustawia tryb oszczędzania energii karty na maksymalną wydajność.";
    tw_psm_why =>
        "Power save parks the radio between beacons, so the first packet after a quiet moment \
         waits for it to wake. That is the tens of milliseconds that show up as a stutter at \
         the start of every call and every click, and it is separate from the Windows device \
         setting — the driver has its own.",
        "Oszczędzanie energii parkuje radio między beaconami, więc pierwszy pakiet po chwili \
         ciszy czeka na wybudzenie. To te kilkadziesiąt milisekund, które widać jako zacięcie \
         na starcie każdej rozmowy i każdego kliknięcia — i jest to coś innego niż ustawienie \
         urządzenia w Windows, sterownik ma własne.";

    tw_mimo_title =>
        "Keep every antenna listening (no MIMO power save)",
        "Trzymaj wszystkie anteny włączone (bez MIMO power save)";
    tw_mimo_what =>
        "Disables spatial multiplexing power save, so the card does not shut down its extra \
         receive chains while idle.",
        "Wyłącza spatial multiplexing power save, żeby karta nie wyłączała dodatkowych torów \
         odbiorczych w bezczynności.";
    tw_mimo_why =>
        "A two-antenna card that has powered one down hears a weak router roughly 3 dB worse, \
         and 3 dB is the difference between a usable link and a dropping one at the far end of \
         the flat.",
        "Karta z dwiema antenami, która wyłączyła jedną, słyszy słaby router o jakieś 3 dB \
         gorzej, a 3 dB to różnica między łączem używalnym a zrywającym się na drugim końcu \
         mieszkania.";

    tw_w24_title =>
        "Use 20 MHz channels on 2.4 GHz",
        "Używaj kanałów 20 MHz na 2,4 GHz";
    tw_w24_what =>
        "Stops the card bonding two 2.4 GHz channels into one 40 MHz channel.",
        "Przestaje łączyć dwa kanały 2,4 GHz w jeden 40 MHz.";
    tw_w24_why =>
        "2.4 GHz has room for three non-overlapping channels. A 40 MHz link takes two of them, \
         so it collides with every neighbour and gets retried to death; the narrower channel is \
         slower on paper and faster in a block of flats, and it reaches further.",
        "Na 2,4 GHz mieszczą się trzy nienachodzące na siebie kanały. Łącze 40 MHz zabiera dwa \
         z nich, więc zderza się z każdym sąsiadem i zajezdza się retransmisjami; węższy kanał \
         jest wolniejszy na papierze, a szybszy w bloku — i sięga dalej.";

    tw_band_title =>
        "Prefer 5 GHz when the signal allows",
        "Preferuj 5 GHz, gdy sygnał pozwala";
    tw_band_what =>
        "Tells the card to pick the 5 GHz radio of a network that broadcasts on both bands.",
        "Każe karcie wybierać radio 5 GHz w sieci nadającej na obu pasmach.";
    tw_band_why =>
        "5 GHz is nearly empty compared with 2.4 and carries several times the throughput, but \
         it goes through walls far worse. Worth it in the same room as the router, wrong at the \
         other end of the flat — if the signal is already weak where you sit, leave this alone.",
        "5 GHz jest w porównaniu z 2,4 prawie puste i niesie kilka razy większą przepustowość, \
         ale znacznie gorzej przechodzi przez ściany. Opłaca się w tym samym pokoju co router, \
         szkodzi na drugim końcu mieszkania — jeśli sygnał u ciebie jest już słaby, zostaw to.";

    tw_intmod_title =>
        "Turn off interrupt moderation on the wired card",
        "Wyłącz interrupt moderation na karcie przewodowej";
    tw_intmod_what =>
        "Makes the adapter raise an interrupt per packet instead of batching them.",
        "Karta zgłasza przerwanie na każdy pakiet, zamiast zbierać je w paczki.";
    tw_intmod_why =>
        "Batching saves CPU by holding packets back for a fraction of a millisecond. That is \
         invisible on a download and measurable on a game or a call. It costs a few percent of \
         one core.",
        "Zbieranie w paczki oszczędza CPU, przetrzymując pakiety przez ułamek milisekundy. \
         Przy pobieraniu tego nie widać, przy grze albo rozmowie widać. Kosztuje kilka procent \
         jednego rdzenia.";

    tw_green_title =>
        "Turn off Green Ethernet and energy-efficient Ethernet",
        "Wyłącz Green Ethernet i energooszczędny Ethernet";
    tw_green_what =>
        "Stops the wired adapter reducing power on a short cable and idling the link between \
         frames.",
        "Karta przewodowa przestaje zmniejszać moc przy krótkim kablu i usypiać łącze między \
         ramkami.";
    tw_green_why =>
        "Both features renegotiate the link, and a renegotiation is a short disconnect. On a \
         worn cable or a cheap switch they cause exactly the kind of dropout that looks like \
         the internet failing.",
        "Obie funkcje renegocjują łącze, a renegocjacja to krótkie rozłączenie. Na zużytym \
         kablu albo tanim switchu wywołują dokładnie taki zanik, jaki wygląda jak awaria \
         internetu.";

    // -----------------------------------------------------------------------
    // tweaks: the stack
    // -----------------------------------------------------------------------
    tw_cong_title =>
        "Use BBR2 congestion control for internet connections",
        "Użyj kontroli przeciążenia BBR2 dla połączeń internetowych";
    tw_cong_what =>
        "Switches the TCP internet template from CUBIC to BBR2, falling back to CTCP on \
         Windows 10, which has no BBR2.",
        "Przełącza szablon TCP internet z CUBIC na BBR2, a na Windows 10, gdzie BBR2 nie ma, \
         na CTCP.";
    tw_cong_why =>
        "CUBIC reads a lost packet as a full queue and halves its rate. On a cable that is \
         correct; on Wi-Fi, where packets are lost to interference, it throws away throughput \
         for congestion that was never there. BBR2 paces against measured bandwidth and \
         round-trip time instead. The cost falls on the other half of the traffic: game \
         launchers like Battle.net and the Riot client open hundreds of short TLS \
         connections that finish before BBR2 has done measuring, and on a lost packet BBR2 \
         waits out a full timeout where CUBIC retransmits at once. A launcher that gives up \
         after a few seconds then sits on a spinner while the browser beside it is fine. It \
         is a real change to how every connection behaves, so measure before and after in \
         the load test, and revert if it does not help — or if something stops loading.",
        "CUBIC czyta zgubiony pakiet jako pełną kolejkę i tnie tempo o połowę. Na kablu to \
         słuszne; na Wi-Fi, gdzie pakiety giną przez zakłócenia, wyrzuca przepustowość za \
         przeciążenie, którego nie było. BBR2 zamiast tego dostraja się do zmierzonej \
         przepustowości i czasu obiegu. Koszt spada na drugą połowę ruchu: launchery gier \
         jak Battle.net czy klient Riot otwierają setki krótkich połączeń TLS, które kończą \
         się, zanim BBR2 skończy mierzyć, a po zgubionym pakiecie BBR2 czeka cały timeout \
         tam, gdzie CUBIC retransmituje od razu. Launcher, który poddaje się po kilku \
         sekundach, wisi wtedy na kręciołku, choć przeglądarka obok działa. To realna zmiana \
         zachowania wszystkich połączeń, więc zmierz test obciążeniowy przed i po, i cofnij, \
         jeśli nie pomaga — albo jeśli coś przestało się ładować.";
    tw_cong_unreadable =>
        "netsh did not report an algorithm",
        "netsh nie podał algorytmu";

    tw_negdns_title =>
        "Stop Windows remembering failed name lookups",
        "Nie pozwól Windows zapamiętywać nieudanych zapytań DNS";
    tw_negdns_what =>
        "Sets the negative DNS cache lifetime to zero.",
        "Ustawia czas życia negatywnego cache DNS na zero.";
    tw_negdns_why =>
        "By default a lookup that failed is remembered as failed for five minutes. So the \
         connection comes back, and the browser still says the site does not exist — the \
         resolver is not asking. This is the reason a working link can still look broken for \
         minutes after an outage.",
        "Domyślnie nieudane zapytanie jest pamiętane jako nieudane przez pięć minut. Czyli \
         połączenie wraca, a przeglądarka dalej twierdzi, że strony nie ma — resolver w ogóle \
         nie pyta. To dlatego działające łącze potrafi jeszcze przez kilka minut po awarii \
         wyglądać na zepsute.";

    tw_do_title =>
        "Stop Windows Update uploading to other machines",
        "Nie pozwól Windows Update wysyłać aktualizacji innym komputerom";
    tw_do_what =>
        "Sets Delivery Optimization to download from Microsoft only, with no peer-to-peer \
         sharing.",
        "Ustawia Delivery Optimization na pobieranie wyłącznie od Microsoftu, bez wymiany \
         peer-to-peer.";
    tw_do_why =>
        "Delivery Optimization seeds updates to strangers over the same uplink you are using. \
         A saturated upload is the classic invisible cause of latency: the download still looks \
         fine, and every acknowledgement is stuck behind the queue.",
        "Delivery Optimization rozsiewa aktualizacje obcym komputerom tym samym łączem, z \
         którego korzystasz. Zapchany upload to klasyczna niewidoczna przyczyna opóźnień: \
         pobieranie dalej wygląda dobrze, a każde potwierdzenie stoi w kolejce.";

    tw_hotspot_title =>
        "Stop connecting automatically to open hotspots",
        "Nie łącz się automatycznie z otwartymi hotspotami";
    tw_hotspot_what =>
        "Turns off the setting that lets Windows join suggested open networks on its own.",
        "Wyłącza ustawienie pozwalające Windows samodzielnie dołączać do proponowanych \
         otwartych sieci.";
    tw_hotspot_why =>
        "Windows will leave a working network for an operator hotspot it recognises, and the \
         hotspot wants a login before anything works. The outage that follows has no cause \
         visible from inside the machine, which is what makes it maddening.",
        "Windows potrafi porzucić działającą sieć na rzecz rozpoznanego hotspotu operatora, a \
         hotspot chce logowania, zanim cokolwiek zadziała. Powstała awaria nie ma przyczyny \
         widocznej z wnętrza komputera i właśnie dlatego doprowadza do szału.";

    tw_ipv4_title =>
        "Prefer IPv4 over IPv6",
        "Preferuj IPv4 zamiast IPv6";
    tw_ipv4_what =>
        "Reorders the address preference table so IPv4 is tried first. IPv6 stays enabled.",
        "Zmienia kolejność w tablicy preferencji adresów, żeby IPv4 był próbowany pierwszy. \
         IPv6 zostaje włączone.";
    tw_ipv4_why =>
        "Where an ISP hands out IPv6 that does not actually route, every connection tries it \
         first and waits out the timeout before falling back. Pages take seconds to start and \
         nothing in Windows says why. Wrong on a network that is genuinely IPv6-first, so \
         revert it if things get worse rather than better.",
        "Gdy operator daje IPv6, które faktycznie nie routuje, każde połączenie próbuje go \
         najpierw i odczekuje timeout, zanim zejdzie na IPv4. Strony ruszają po kilku \
         sekundach, a Windows nic nie tłumaczy. Szkodliwe w sieci naprawdę opartej na IPv6 — \
         jeśli będzie gorzej zamiast lepiej, cofnij.";

    // -----------------------------------------------------------------------
    // the air scan
    // -----------------------------------------------------------------------
    air_no_service =>
        "the Windows WLAN service is not running",
        "usługa WLAN systemu Windows nie działa";
    air_no_adapter =>
        "no Wi-Fi adapter to scan with",
        "brak karty Wi-Fi, którą można skanować";
    air_title =>
        "What else is on the air",
        "Co jeszcze jest w eterze";
    air_blurb =>
        "Every access point in range beacons its channel and its signal, so the interference \
         each channel would suffer can be measured. The channel itself is set in the router, \
         not here — this works out which one to ask it for.",
        "Każdy access point w zasięgu rozgłasza swój kanał i sygnał, więc zakłócenia na każdym \
         kanale da się zmierzyć. Sam kanał ustawia się w routerze, nie tutaj — to wylicza, o \
         który go poprosić.";
    air_btn_scan => "Scan the air", "Skanuj eter";
    air_scan_cost =>
        "Costs about a second of connectivity: the card has to leave your channel to \\
         listen to the others. The monitor is paused meanwhile, so it is not filed as \\
         an outage.",
        "Kosztuje około sekundy łączności: karta musi zejść z twojego kanału, żeby \\
         posłuchać pozostałych. Monitor jest na ten czas wstrzymany, więc nie trafia to \\
         do historii jako awaria.";
    air_scanning => "scanning, about four seconds…", "skanowanie, około czterech sekund…";
    air_empty => "Nothing scanned yet.", "Jeszcze nic nie zeskanowano.";
    air_quiet_here =>
        "Your channel is already the quietest of the three. Nothing to ask the router for.",
        "Twój kanał jest już najspokojniejszy z trzech. Nie ma o co prosić routera.";
    air_router_note =>
        "Change this on the router's settings page, under the 2.4 GHz wireless channel. Set a \
         fixed channel rather than auto: auto picks at boot and then never reconsiders.",
        "Zmień to na stronie ustawień routera, przy kanale bezprzewodowym 2,4 GHz. Ustaw kanał \
         na stałe zamiast auto: auto wybiera przy starcie i potem już nigdy tego nie rozważa.";
    air_dfs_note =>
        "Only radar-free channels are suggested on 5 GHz. A DFS channel can be perfectly quiet \
         and still cut the network for a minute when the router thinks it heard radar.",
        "Na 5 GHz proponowane są tylko kanały wolne od radaru. Kanał DFS może być idealnie \
         cichy i mimo to uciąć sieć na minutę, gdy routerowi wyda się, że usłyszał radar.";
    air_dfs_move =>
        "Move off it: pick 36, 40, 44 or 48 for range, or 149 and up for the least \\
         crowded air. Both groups are radar-free and never go quiet on their own.",
        "Zejdź z niego: wybierz 36, 40, 44 albo 48 dla zasięgu, albo 149 i wyżej dla \\
         najmniej zatłoczonego eteru. Obie grupy są wolne od radaru i nigdy nie milkną \\
         same z siebie.";
    // -----------------------------------------------------------------------
    // the optimise list: sections and status
    // -----------------------------------------------------------------------
    cat_power => "Power and sleep", "Zasilanie i uśpienie";
    cat_power_blurb =>
        "Windows and the driver switching hardware off underneath you. Most drops \
         \"for no reason\" start here.",
        "Windows i sterownik wyłączają sprzęt pod tobą. Tu zaczyna się większość zrywów \
         „bez powodu”.";
    cat_reach => "Radio and range", "Radio i zasięg";
    cat_reach_blurb =>
        "How far the card reaches, and which access point it holds on to.",
        "Jak daleko sięga karta i którego access pointa się trzyma.";
    cat_naming => "Names and addresses", "Nazwy i adresy";
    cat_naming_blurb =>
        "Turning names into addresses, and which version of IP wins.",
        "Zamiana nazw na adresy i to, która wersja IP wygrywa.";
    cat_throughput => "Throughput and latency", "Przepustowość i opóźnienia";
    cat_throughput_blurb =>
        "How fast bytes move once the link is up.",
        "Jak szybko lecą bajty, kiedy łącze już stoi.";
    cat_neighbours => "What else uses the link", "Kto jeszcze zużywa łącze";
    cat_neighbours_blurb =>
        "Things on this machine helping themselves to the uplink.",
        "Rzeczy na tym komputerze, które biorą pasmo bez pytania.";
    cat_last_resort => "Last resort", "Ostateczność";
    cat_last_resort_blurb =>
        "Blunt instruments for when the link is already broken.",
        "Narzędzia na sytuację, w której łącze jest już popsute.";

    st_set => "set", "ustawione";
    st_todo => "worth changing", "do poprawy";
    st_na => "not available", "niedostępne";
    opt_revert_available =>
        "changed by NetDoctor (can be undone)",
        "zmienione przez NetDoctor (można cofnąć)";
    opt_section_all_set => "all set", "wszystko ustawione";
    opt_show_unavailable => "show unavailable", "pokaż niedostępne";
    opt_show_unavailable_hint =>
        "Changes this machine cannot take: a Wi-Fi setting on a cable, or one the \\
         driver does not expose. The section counts ignore them either way.",
        "Zmiany, których ta maszyna nie przyjmie: ustawienie Wi-Fi przy kablu albo takie, \\
         którego sterownik nie udostępnia. Liczniki sekcji i tak ich nie liczą.";
    opt_section_none => "nothing applies here", "nic tu nie dotyczy";

    air_col_network => "Network", "Sieć";
    air_col_channel => "Channel", "Kanał";
    air_col_signal => "Signal", "Sygnał";
    air_hidden_ssid => "(hidden)", "(ukryta)";
    air_yours => "yours", "twoja";
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

/// The snapshot file exists but could not be read or parsed. Never silently
/// treated as "no snapshots": that would hide every recorded "before" value.
/// Warning before the load test: it pulls real data, which matters on a
/// metered or mobile connection.
pub fn bloat_cost_warning() -> &'static str {
    match current() {
        Lang::En => "The test downloads as fast as the line allows for about 14 seconds. On a fast connection that is well over a gigabyte — avoid it on a metered or mobile link.",
        Lang::Pl => "Test pobiera dane z pełną prędkością łącza przez około 14 sekund. Na szybkim łączu to grubo ponad gigabajt — nie uruchamiaj go na połączeniu taryfowym ani na telefonie.",
    }
}

/// How much the test actually cost, shown next to the result.
pub fn bloat_data_used(mib: f64) -> String {
    match current() {
        Lang::En => format!("Data used by this test: {mib:.0} MB"),
        Lang::Pl => format!("Dane zużyte przez ten test: {mib:.0} MB"),
    }
}

/// Every download stream died, so nothing loaded the line.
pub fn bloat_no_load_str() -> &'static str {
    match current() {
        Lang::En => "No load reached the line: every download stream failed. Nothing to grade.",
        Lang::Pl => {
            "Nie udało się obciążyć łącza: wszystkie strumienie padły. Nie ma czego oceniać."
        }
    }
}

/// Some streams died, so the grade was read under less than full load.
pub fn bloat_partial_load(alive: usize, total: usize) -> String {
    match current() {
        Lang::En => format!(
            "Only {alive} of {total} download streams held up, so the line was not fully loaded — the grade is optimistic."
        ),
        Lang::Pl => format!(
            "Utrzymało się tylko {alive} z {total} strumieni, więc łącze nie było w pełni obciążone — ocena jest zawyżona."
        ),
    }
}

/// An argument the CLI does not know.
pub fn cli_unknown_flag(flag: &str) -> String {
    match current() {
        Lang::En => format!("Unknown option: {flag}"),
        Lang::Pl => format!("Nieznana opcja: {flag}"),
    }
}

/// Shown after a panic, pointing at the file that says what happened.
pub fn crash_notice(path: &str) -> String {
    match current() {
        Lang::En => format!("NetDoctor stopped unexpectedly. Details were written to:\n{path}"),
        Lang::Pl => {
            format!("NetDoctor zatrzymał się nieoczekiwanie. Szczegóły zapisano w:\n{path}")
        }
    }
}

/// The settings file was there but unreadable, so defaults are in force.
pub fn set_load_failed(detail: &str) -> String {
    match current() {
        Lang::En => format!(
            "Settings could not be read and defaults are in use. The old file was kept: {detail}"
        ),
        Lang::Pl => format!(
            "Nie udało się odczytać ustawień, działają domyślne. Stary plik zachowano: {detail}"
        ),
    }
}

pub fn tw_snapshots_unreadable(path: &str, err: &str) -> String {
    match current() {
        Lang::En => format!("cannot read saved state ({path}): {err}"),
        Lang::Pl => format!("nie można odczytać zapisanych stanów ({path}): {err}"),
    }
}

/// The change went through but its "before" value could not be stored, so
/// Revert will not work for it. Worth saying out loud rather than burying.
pub fn tw_snapshot_save_failed(err: &str) -> String {
    match current() {
        Lang::En => format!("WARNING: applied, but the previous value could not be saved ({err}). Revert will not be available."),
        Lang::Pl => format!("UWAGA: zastosowano, ale nie udało się zapisać poprzedniej wartości ({err}). Cofnięcie nie będzie dostępne."),
    }
}

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
        Lang::En => format!("enabled, Windows may suspend the card (PnPCapabilities={value})"),
        Lang::Pl => format!("włączone, Windows może uśpić kartę (PnPCapabilities={value})"),
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

// --- verdict ---------------------------------------------------------------

pub fn verdict_confident(segment: &str, confidence: &str) -> String {
    match current() {
        Lang::En => format!("{segment} — {confidence}"),
        Lang::Pl => format!("{segment} — {confidence}"),
    }
}

/// How much of the round trip each segment of the chain adds. This is the line
/// that turns "ping is 90 ms" into an address for the complaint.
pub fn verdict_split(lan: f64, isp: f64, far: f64) -> String {
    match current() {
        Lang::En => format!(
            "Of the total round trip: {lan:.0} ms to your router, {isp:.0} ms added by the hop \
             into the provider, {far:.0} ms added by everything beyond it."
        ),
        Lang::Pl => format!(
            "Z całego czasu przelotu: {lan:.0} ms do Twojego routera, {isp:.0} ms dokłada wejście \
             do sieci dostawcy, {far:.0} ms dokłada wszystko za nim."
        ),
    }
}

pub fn cost_load(bump: f64) -> String {
    match current() {
        Lang::En => format!(
            "A download in the background adds {bump:.0} ms to everything else. Calls break up, \
             games rubber-band and pages stall while anything is transferring."
        ),
        Lang::Pl => format!(
            "Pobieranie w tle dokłada {bump:.0} ms do wszystkiego innego. Rozmowy się rwą, gry \
             teleportują, a strony stają w miejscu, kiedy cokolwiek się transferuje."
        ),
    }
}

pub fn cost_loss(pct: f64) -> String {
    match current() {
        Lang::En => format!(
            "{pct:.0}% of packets never arrive. Every one of them is a stutter in a call and a \
             retransmission that slows a download down."
        ),
        Lang::Pl => format!(
            "{pct:.0}% pakietów nie dociera. Każdy z nich to zacięcie w rozmowie i retransmisja, \
             która spowalnia pobieranie."
        ),
    }
}

pub fn cost_latency(ms: f64) -> String {
    match current() {
        Lang::En => format!(
            "Every request waits {ms:.0} ms before anything can come back. Browsing feels heavy \
             and competitive games are unplayable above roughly 80 ms."
        ),
        Lang::Pl => format!(
            "Każde żądanie czeka {ms:.0} ms, zanim cokolwiek wróci. Przeglądanie jest ociężałe, a \
             gry sieciowe powyżej mniej więcej 80 ms przestają się nadawać do grania."
        ),
    }
}

pub fn cost_jitter(ms: f64) -> String {
    match current() {
        Lang::En => format!(
            "Latency swings by {ms:.0} ms between packets. Voice and video absorb a high ping far \
             better than they absorb an unpredictable one."
        ),
        Lang::Pl => format!(
            "Opóźnienie skacze o {ms:.0} ms między pakietami. Głos i wideo znoszą wysoki ping \
             znacznie lepiej niż nieprzewidywalny."
        ),
    }
}

pub fn cost_dns(ms: f64) -> String {
    match current() {
        Lang::En => format!(
            "Every new domain costs {ms:.0} ms before the first byte is even requested. The line \
             is fine; the wait happens before it is used."
        ),
        Lang::Pl => format!(
            "Każda nowa domena kosztuje {ms:.0} ms, zanim padnie żądanie o pierwszy bajt. Łącze \
             jest sprawne; czekanie dzieje się, zanim w ogóle zostanie użyte."
        ),
    }
}

// --- findings: segment isolation, reachability, load -----------------------

pub fn f_edge_found(addr: &str, ms: f64) -> String {
    match current() {
        Lang::En => format!("Provider's first hop answers ({addr}, {ms:.0} ms)"),
        Lang::Pl => format!("Pierwszy węzeł dostawcy odpowiada ({addr}, {ms:.0} ms)"),
    }
}

pub fn f_local_hop(addr: &str, ms: f64) -> String {
    match current() {
        Lang::En => format!("Another router of your own on the path ({addr}, {ms:.0} ms)"),
        Lang::Pl => format!("Na trasie stoi Twój drugi router ({addr}, {ms:.0} ms)"),
    }
}

pub fn f_local_hop_slow(addr: &str, ms: f64) -> String {
    match current() {
        Lang::En => format!("Your second router adds {ms:.0} ms ({addr})"),
        Lang::Pl => format!("Twój drugi router dokłada {ms:.0} ms ({addr})"),
    }
}

pub fn f_edge_slow(ms: f64) -> String {
    match current() {
        Lang::En => format!("The provider's first hop adds {ms:.0} ms"),
        Lang::Pl => format!("Pierwszy węzeł dostawcy dokłada {ms:.0} ms"),
    }
}

pub fn f_edge_lossy(pct: f64) -> String {
    match current() {
        Lang::En => format!("Loss starts at the provider's first hop ({pct:.0}%)"),
        Lang::Pl => format!("Straty zaczynają się na pierwszym węźle dostawcy ({pct:.0}%)"),
    }
}

pub fn f_edge_detail(addr: &str, stats: &str) -> String {
    match current() {
        Lang::En => format!("Hop {addr}: {stats}"),
        Lang::Pl => format!("Węzeł {addr}: {stats}"),
    }
}

pub fn f_edge_silent_detail(addr: &str) -> String {
    match current() {
        Lang::En => format!("{addr} is on the path but answered nothing."),
        Lang::Pl => format!("{addr} jest na trasie, ale nie odpowiedział ani razu."),
    }
}

pub fn f_tcp_ok(host: &str, ms: f64) -> String {
    match current() {
        Lang::En => format!("Real traffic gets through ({host}:443 in {ms:.0} ms)"),
        Lang::Pl => format!("Realny ruch przechodzi ({host}:443 w {ms:.0} ms)"),
    }
}

pub fn f_tcp_slow(ms: f64) -> String {
    match current() {
        Lang::En => format!("Slow TCP handshake ({ms:.0} ms)"),
        Lang::Pl => format!("Wolny handshake TCP ({ms:.0} ms)"),
    }
}

pub fn f_tcp_detail(host: &str, ms: f64, ping: f64) -> String {
    match current() {
        Lang::En => {
            format!("{host}:443 answered in {ms:.0} ms; ping to the same network is {ping:.0} ms.")
        }
        Lang::Pl => format!(
            "{host}:443 odpowiedział w {ms:.0} ms; ping do tej samej sieci to {ping:.0} ms."
        ),
    }
}

pub fn f_tcp_blocked_detail(host: &str, err: &str) -> String {
    match current() {
        Lang::En => format!("Could not open {host}:443 — {err}"),
        Lang::Pl => format!("Nie udało się otworzyć {host}:443 — {err}"),
    }
}

pub fn f_load_bad(bump: f64) -> String {
    match current() {
        Lang::En => format!("Latency collapses under load (+{bump:.0} ms)"),
        Lang::Pl => format!("Opóźnienie załamuje się pod obciążeniem (+{bump:.0} ms)"),
    }
}

pub fn f_load_ok(bump: f64) -> String {
    match current() {
        Lang::En => format!("Latency holds under load (+{bump:.0} ms)"),
        Lang::Pl => format!("Opóźnienie trzyma się pod obciążeniem (+{bump:.0} ms)"),
    }
}

pub fn f_load_detail(idle: f64, loaded: f64, mbps: f64, grade: &str) -> String {
    match current() {
        Lang::En => format!(
            "Idle {idle:.0} ms, saturated {loaded:.0} ms, at {mbps:.0} Mbps. Bufferbloat grade {grade}."
        ),
        Lang::Pl => format!(
            "Bez obciążenia {idle:.0} ms, przy pełnym {loaded:.0} ms, przy {mbps:.0} Mbps. \
             Ocena bufferbloatu: {grade}."
        ),
    }
}

pub fn f_baseline_worse(now: f64, usual: f64) -> String {
    match current() {
        Lang::En => format!("Worse than usual for this line ({now:.0} ms vs {usual:.0} ms)"),
        Lang::Pl => format!("Gorzej niż zwykle na tym łączu ({now:.0} ms wobec {usual:.0} ms)"),
    }
}

pub fn f_baseline_detail(now: f64, usual: f64, samples: usize) -> String {
    match current() {
        Lang::En => format!(
            "Now {now:.0} ms against a seven-day median of {usual:.0} ms over {samples} samples."
        ),
        Lang::Pl => format!(
            "Teraz {now:.0} ms wobec mediany {usual:.0} ms z siedmiu dni i {samples} próbek."
        ),
    }
}

pub fn diag_checked_ok(count: usize) -> String {
    match current() {
        Lang::En => format!("Checked and fine ({count})"),
        Lang::Pl => format!("Sprawdzone i w porządku ({count})"),
    }
}

/// The same tally as a count, for the row under the plot.
///
/// The sentence version is the explanation and belongs on hover; what stays
/// on screen is how many there were and how they split, which is the part
/// that changes as the window moves.
pub fn live_spike_counts(correlated: usize, single: usize) -> String {
    let total = correlated + single;
    match current() {
        Lang::En => {
            format!("{total} spikes: {correlated} shared, {single} single")
        }
        Lang::Pl => {
            format!("skoki: {total} \u{2014} {correlated} wspólne, {single} pojedyncze")
        }
    }
}

pub fn live_spike_tally(correlated: usize, single: usize) -> String {
    match current() {
        Lang::En => format!(
            "{} spikes in view: {correlated} hit every target at once and are real; {single} hit \
             one target while the rest stayed normal, and say nothing about your link.",
            correlated + single
        ),
        Lang::Pl => format!(
            "Skoków w widoku: {}. {correlated} trafiło we wszystkie cele naraz i są prawdziwe; \
             {single} trafiło w jeden cel, gdy reszta była normalna — te nie mówią nic o Twoim \
             łączu.",
            correlated + single
        ),
    }
}

pub fn f_wired_detail(adapter: &str, mbps: u64) -> String {
    format!("{adapter}, {mbps} Mbps")
}

pub fn f_wifi_detail(adapter: &str, ssid: &str, mbps: u64) -> String {
    match current() {
        Lang::En => format!("{adapter}: {ssid}, {mbps} Mbps link rate"),
        Lang::Pl => format!("{adapter}: {ssid}, prędkość łącza {mbps} Mbps"),
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
             good ping\" actually is. It is typical of Wi-Fi and of a saturated link."
        ),
        Lang::Pl => format!(
            "Opóźnienie waha się o {spread:.0} ms między pakietami. To właśnie jest „lagowanie \
             mimo dobrego pingu”. Typowe dla Wi-Fi i wysyconego łącza."
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
        Lang::En => format!("{title} ({count}× in the last 24 h)"),
        Lang::Pl => format!("{title} ({count}× w ciągu ostatnich 24 h)"),
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
        Lang::En => format!("No reply from {host}. Test aborted."),
        Lang::Pl => format!("Brak odpowiedzi od {host}. Test przerwany."),
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
            format!("Throughput measured during the test: {mbps:.0} Mbps. Use that to set the cap.")
        }
        Lang::Pl => format!(
            "Przepustowość zmierzona w teście: {mbps:.0} Mbps. Na tej podstawie ustaw limit."
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
        "after_tweak" => {
            ("A change applied just before this", "Zmiana zastosowana tuż przed awarią")
        }
        "adapter_powered_down" => {
            ("Windows put the Wi-Fi card to sleep", "Windows uśpił kartę Wi-Fi")
        }
        "adapter_power_plan" => {
            ("The power plan may be parking the radio", "Plan zasilania może wyłączać radio")
        }
        "out_of_range" => ("Out of range of the access point", "Poza zasięgiem access pointa"),
        "adapter_or_driver" => {
            ("The adapter disappeared: driver or hardware", "Karta zniknęła: sterownik albo sprzęt")
        }
        "roaming" => ("Handover to another access point", "Przełączenie na inny access point"),
        "signal_fade" => ("The signal faded away", "Sygnał stopniowo zanikał"),
        "airtime_24ghz" => ("The 2.4 GHz channel is crowded", "Kanał 2.4 GHz jest zatłoczony"),
        "router_side" => (
            "The radio was fine, the router side was not",
            "Radio było w porządku, problem po stronie routera",
        ),
        "weak_signal" => {
            ("Weak signal at the moment of the drop", "Słaby sygnał w chwili zerwania")
        }
        "marginal_link" => ("The link was marginal", "Łącze było na granicy"),
        "cable_or_router" => ("Cable or router, not Wi-Fi", "Kabel albo router, nie Wi-Fi"),
        "isp_sustained" => ("A sustained outage at the provider", "Dłuższa awaria u dostawcy"),
        "isp_brief" => ("A brief drop on the WAN side", "Krótki zryw po stronie WAN"),
        "isp_pattern" => ("The provider drops repeatedly", "Dostawca zrywa regularnie"),
        "dns_router_only" => ("The router is the only resolver", "Router jest jedynym resolverem"),
        "dns_resolver" => ("The resolver did not answer", "Resolver nie odpowiedział"),
        "local_saturation" => {
            ("The link to the router was saturated", "Łącze do routera było wysycone")
        }
        "rate_collapse" => ("The Wi-Fi rate collapsed", "Prędkość Wi-Fi załamała się"),
        "time_pattern" => ("It happens at the same hour", "Zdarza się o tej samej godzinie"),
        "no_evidence" => ("No evidence was recorded", "Nie zapisano dowodów"),
        "unclear" => ("No single cause stands out", "Żadna przyczyna się nie wyróżnia"),

        // Read out of the Windows event log rather than inferred from probes.
        "log_sleep" => ("The machine was asleep", "Komputer spał"),
        "log_resume" => (
            "The connection was still coming back from sleep",
            "Połączenie wracało jeszcze po uśpieniu",
        ),
        "log_driver_fault" => {
            ("The adapter driver logged an error", "Sterownik karty zapisał błąd")
        }
        "log_wlan_inactivity" => {
            ("The access point dropped an idle card", "Access point odrzucił bezczynną kartę")
        }
        "log_wlan_auth" => (
            "Authentication with the access point failed",
            "Uwierzytelnianie z access pointem nie powiodło się",
        ),
        "log_wlan_ap_rejected" => {
            ("The access point turned the card away", "Access point odmówił karcie")
        }
        "log_wlan_deauth" => {
            ("Windows recorded the wireless disconnect", "Windows zapisał rozłączenie Wi-Fi")
        }
        "log_dhcp" => ("The DHCP lease failed", "Dzierżawa DHCP nie powiodła się"),
        "log_duplicate_ip" => {
            ("Another device has the same address", "Inne urządzenie ma ten sam adres")
        }
        "log_link_down" => {
            ("Windows saw the interface go down", "Windows zobaczył wyłączenie interfejsu")
        }
        "log_clean_isp" => {
            ("Nothing went wrong on this machine", "Po stronie tego komputera nic się nie zepsuło")
        }
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

        "log_sleep" => (
            "Nothing was wrong with the network: the computer suspended and the monitor kept \
             counting. If you did not expect it to sleep, the sleep timer in the power plan is the \
             setting to look at, not anything in here.",
            "Z siecią nie było nic nie tak: komputer się uśpił, a monitor dalej liczył. Jeśli nie \
             spodziewałeś się uśpienia, popatrz na licznik uśpienia w planie zasilania, a nie na \
             cokolwiek tutaj.",
        ),
        "log_resume" => (
            "After waking, the radio has to re-associate and the DHCP lease has to be confirmed, \
             and that takes a few seconds during which nothing answers. An outage that fills \
             exactly that gap is the resume sequence working, not failing.",
            "Po wybudzeniu radio musi się ponownie powiązać, a dzierżawa DHCP potwierdzić — to \
             kilka sekund, w których nic nie odpowiada. Awaria wypełniająca dokładnie tę lukę to \
             działająca sekwencja wybudzenia, a nie jej błąd.",
        ),
        "log_driver_fault" => (
            "The driver failed on its own and Windows wrote it down, so this is not a router or a \
             provider problem. Update the adapter driver from the vendor rather than through \
             Windows Update, which usually keeps an older one. Resetting the network stack clears \
             the state a fault leaves behind.",
            "Sterownik zawiódł sam z siebie i Windows to zapisał, więc to nie problem routera ani \
             dostawcy. Zaktualizuj sterownik karty od producenta, a nie przez Windows Update, \
             który zwykle trzyma starszy. Reset stosu sieciowego czyści stan, który zostaje po \
             takiej usterce.",
        ),
        "log_wlan_inactivity" => (
            "The access point stopped hearing from a card that Windows had quietly powered down, \
             so it disassociated it. This is the textbook cause of a link that dies while the \
             machine sits idle and revives the moment you touch it — turn off the adapter's power \
             saving and the power plan's, both.",
            "Access point przestał słyszeć kartę, którą Windows po cichu wyłączył, więc ją \
             rozłączył. To podręcznikowa przyczyna łącza, które umiera, gdy komputer stoi \
             bezczynnie, i ożywa, gdy tylko go dotkniesz — wyłącz oszczędzanie energii karty i \
             planu zasilania, oba.",
        ),
        "log_wlan_auth" => (
            "The card reached the access point and was not let in. That is the key or the \
             credentials, not the signal: a changed Wi-Fi password, a profile holding the old one, \
             or a router rotating keys faster than the card follows. Forget the network and \
             reconnect to rebuild the profile.",
            "Karta dotarła do access pointa i nie została wpuszczona. To klucz albo poświadczenia, \
             nie sygnał: zmienione hasło Wi-Fi, profil trzymający stare, albo router rotujący \
             klucze szybciej, niż karta nadąża. Zapomnij sieć i połącz się ponownie, żeby \
             odbudować profil.",
        ),
        "log_wlan_ap_rejected" => (
            "The access point refused the card rather than losing it — usually because it had no \
             capacity left. Check how many devices are associated, and whether a guest network or \
             a mesh node is holding slots it does not need.",
            "Access point odmówił karcie, zamiast ją zgubić — zwykle dlatego, że nie miał wolnych \
             miejsc. Sprawdź, ile urządzeń jest powiązanych i czy sieć gościnna albo węzeł mesh \
             nie trzyma miejsc, których nie potrzebuje.",
        ),
        "log_wlan_deauth" => (
            "Windows recorded the disconnect itself, so the link really was torn down rather than \
             merely going quiet. The reason code above is what to quote if you take this to the \
             router's vendor or your provider.",
            "Windows sam zapisał rozłączenie, więc łącze naprawdę zostało zerwane, a nie tylko \
             ucichło. Kod przyczyny powyżej jest tym, co warto zacytować, idąc z tym do \
             producenta routera albo do dostawcy.",
        ),
        "log_dhcp" => (
            "The adapter could not get an address from the router. Until it has one, nothing \
             routes, however good the signal is. Restart the router's DHCP server or check that \
             its address pool is not exhausted — a full pool fails exactly like this, and only for \
             whichever device asks last.",
            "Karta nie mogła dostać adresu od routera. Dopóki go nie ma, nic się nie routuje, \
             niezależnie od jakości sygnału. Zrestartuj serwer DHCP routera albo sprawdź, czy jego \
             pula adresów się nie wyczerpała — pełna pula zawodzi dokładnie w ten sposób i tylko \
             dla tego urządzenia, które pyta jako ostatnie.",
        ),
        "log_duplicate_ip" => (
            "Two devices are claiming one address, so replies go to whichever answers first. It is \
             almost always a static address set by hand inside the router's DHCP range. Move it \
             outside the pool, or hand it out as a reservation instead.",
            "Dwa urządzenia zgłaszają jeden adres, więc odpowiedzi trafiają do tego, które \
             odpowie pierwsze. Prawie zawsze to adres statyczny ustawiony ręcznie wewnątrz zakresu \
             DHCP routera. Przenieś go poza pulę albo rozdawaj jako rezerwację.",
        ),
        "log_link_down" => (
            "The interface itself went down — the OS saw it, so this is the adapter, its driver, \
             its cable or its power state, and not anything beyond the router.",
            "Sam interfejs padł — system to zobaczył, więc chodzi o kartę, jej sterownik, kabel \
             albo stan zasilania, a nie o cokolwiek za routerem.",
        ),
        "log_clean_isp" => (
            "The Windows log has entries from this period and none of them is a fault here: no \
             sleep, no driver error, no disconnect, no DHCP failure. Together with the router \
             answering throughout, that puts the outage past your own equipment — which is exactly \
             the case a provider has to answer.",
            "Dziennik Windows ma wpisy z tego okresu i żaden z nich nie jest usterką tutaj: brak \
             uśpienia, brak błędu sterownika, brak rozłączenia, brak awarii DHCP. Razem z \
             routerem odpowiadającym przez cały czas stawia to awarię za twoim sprzętem — a to \
             dokładnie ten przypadek, na który dostawca musi odpowiedzieć.",
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
        Lang::En => format!("signal was {rssi} dBm, below the usable threshold of about -75 dBm"),
        Lang::Pl => format!("sygnał wynosił {rssi} dBm, poniżej progu używalności około -75 dBm"),
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
        Lang::En => format!("signal slid from {from} to {to} dBm, {drop} dB lost before the drop"),
        Lang::Pl => {
            format!("sygnał osunął się z {from} do {to} dBm, {drop} dB straty przed zerwaniem")
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
        None => {
            return pick(
                "the router kept answering; it is still down",
                "router odpowiadał; awaria trwa",
            )
        }
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
        Lang::En => {
            format!("{count} of these outages started between {hour:02}:00 and {:02}:00", hour + 1)
        }
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

// ---------------------------------------------------------------------------
// the per-hop path
// ---------------------------------------------------------------------------

/// A chart window's name on its button.
pub fn range_name(secs: f64) -> String {
    if secs < 3600.0 {
        format!("{} min", (secs / 60.0).round() as i64)
    } else {
        pick("1 h", "1 godz")
    }
}

/// The moment the pointer is over, as a clock time and an age.
pub fn live_hover_when(clock: &str, secs_ago: f64) -> String {
    let ago = if secs_ago < 90.0 {
        format!("{secs_ago:.0} s")
    } else {
        format!("{:.0} min", secs_ago / 60.0)
    };
    match current() {
        Lang::En => format!("{clock}  ({ago} ago)"),
        Lang::Pl => format!("{clock}  ({ago} temu)"),
    }
}

/// Said under the plot when the scale was capped to keep the normal range
/// readable. A chart that quietly drops its outliers is lying; one that says
/// how many it put off the top is not.
pub fn live_above_scale(n: usize, top: f64) -> String {
    match current() {
        Lang::En => format!("{n} above {top:.0} ms, off the top"),
        Lang::Pl => format!("{n} powyżej {top:.0} ms, poza skalą"),
    }
}

/// The number written on a threshold line, in the plot.
///
/// Short on purpose: it sits on the chart, over the data, and it only has to
/// say which height this line is at. The unit is there because the y axis
/// carries bare numbers and the caption names the unit once, a long way from
/// this line.
pub fn live_threshold_mark(ms: f64) -> String {
    // The line is drawn at the exact value, so a whole-number label on a
    // threshold of 62.5 would name a height the line is not at.
    if (ms - ms.round()).abs() < 0.05 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

/// Everything the chart's markings mean, in one place.
///
/// This was a row of seven items under the plot: the unit, two markings, three
/// threshold words, an instruction, and a live count — unrelated things joined
/// by dots, wrapping wherever the window happened to end. Most of it never
/// changed and only had to be read once, so it lives here, behind one badge,
/// and the row keeps the parts that are actually about the data on screen.
pub fn live_chart_help() -> String {
    let parts = [live_ms_explainer(), live_spike_explainer(), live_hover_hint_long()];
    parts.join("\n\n")
}

/// What a given probe target is, and what a high reading on it means.
///
/// Keyed on the scope rather than on the address, so a target the user adds
/// gets an honest answer instead of the wrong canned one.
pub fn live_target_meaning(key: &str, scope: crate::settings::Scope) -> &'static str {
    use crate::settings::Scope;
    if key.starts_with("custom") {
        return live_meaning_custom();
    }
    match scope {
        Scope::Lan => live_meaning_lan(),
        Scope::Isp => live_meaning_isp(),
        Scope::Internet => live_meaning_internet(),
    }
}

pub fn path_owner(owner: crate::probe::path::Owner) -> &'static str {
    use crate::probe::path::Owner as O;
    match owner {
        O::Gateway => path_owner_gateway(),
        O::Local => path_owner_local(),
        O::Edge => path_owner_edge(),
        O::Internet => path_owner_internet(),
    }
}

/// The headline when a hop is dropping packets all the way to the end.
pub fn path_blame_loss(ttl: u32, addr: &str, loss: f64, owner: &str) -> String {
    match current() {
        Lang::En => {
            format!("Loss starts at hop {ttl}, {addr} — {loss:.0}% and it carries ({owner}).")
        }
        Lang::Pl => {
            format!("Strata zaczyna się na skoku {ttl}, {addr} — {loss:.0}% i niesie się dalej ({owner}).")
        }
    }
}

/// The headline when nothing is lost but one hop is where the time goes.
pub fn path_blame_delay(ttl: u32, addr: &str, added: f64, owner: &str) -> String {
    match current() {
        Lang::En => {
            format!("Hop {ttl}, {addr} adds {added:.0} ms and everything behind it carries that delay ({owner}).")
        }
        Lang::Pl => {
            format!("Skok {ttl}, {addr} dokłada {added:.0} ms i wszystko za nim niesie to opóźnienie ({owner}).")
        }
    }
}

/// What to do about it, which depends entirely on whose equipment it is.
pub fn path_blame_advice(mine: bool) -> &'static str {
    match (current(), mine) {
        (Lang::En, true) => {
            "That hop is your own equipment, so this one is fixable here: the router, a second \
             router behind it, or the link between them."
        }
        (Lang::Pl, true) => {
            "Ten skok to twój własny sprzęt, więc da się to naprawić u siebie: router, drugi \
             router za nim albo łącze między nimi."
        }
        (Lang::En, false) => {
            "That hop is past your equipment. Quote the hop number, the address and this loss \
             figure to the provider — it is the one form of evidence a support line cannot \
             answer with \"restart the router\"."
        }
        (Lang::Pl, false) => {
            "Ten skok jest za twoim sprzętem. Podaj dostawcy numer skoku, adres i tę wartość \
             straty — to jedyny rodzaj dowodu, na który infolinia nie odpowie „zrestartuj \
             router”."
        }
    }
}

// ---------------------------------------------------------------------------
// the Windows event log
// ---------------------------------------------------------------------------

/// Where a log line sits relative to the start of the outage. Read as part of
/// an evidence sentence, so it names no subject of its own.
pub fn clock_offset(secs: f64) -> String {
    let v = secs.round() as i64;
    if v == 0 {
        return pick("in the same second", "w tej samej sekundzie");
    }
    let amount = if v.abs() >= 90 {
        format!("{} min", (v.abs() as f64 / 60.0).round() as i64)
    } else {
        format!("{} s", v.abs())
    };
    match (current(), v < 0) {
        (Lang::En, true) => format!("{amount} before it started"),
        (Lang::En, false) => format!("{amount} after it started"),
        (Lang::Pl, true) => format!("{amount} przed jej początkiem"),
        (Lang::Pl, false) => format!("{amount} po jej początku"),
    }
}

/// An 802.11 disconnect reason in words. Codes outside the named set keep
/// their number: a support line will ask for the number anyway, and inventing
/// a sentence for an unknown code would be worse than admitting the gap.
pub fn wlan_reason(code: u32) -> String {
    let (en, pl) = match code {
        1 => ("unspecified", "nieokreślony"),
        2 => (
            "the previous authentication was no longer valid",
            "poprzednie uwierzytelnienie przestało być ważne",
        ),
        3 => ("the station is leaving the network", "stacja opuszcza sieć"),
        4 => ("disassociated for inactivity", "rozłączenie z powodu bezczynności"),
        5 => ("the access point had no capacity left", "access point nie miał już wolnych miejsc"),
        6 | 7 => (
            "a frame arrived from an unassociated station",
            "nadeszła ramka od niepowiązanej stacji",
        ),
        8 => ("the station is leaving the BSS", "stacja opuszcza BSS"),
        15 => ("the four-way handshake timed out", "four-way handshake przekroczył czas"),
        23 => ("802.1X authentication failed", "uwierzytelnianie 802.1X nie powiodło się"),
        other => return pick(&format!("reason {other}"), &format!("powód {other}")),
    };
    pick(en, pl)
}

pub fn ev_log_sleep(at: &str) -> String {
    match current() {
        Lang::En => format!("Windows logged the machine going to sleep {at}"),
        Lang::Pl => format!("Windows zapisał uśpienie komputera {at}"),
    }
}

pub fn ev_log_resume(at: &str) -> String {
    match current() {
        Lang::En => format!("Windows logged the machine waking {at}"),
        Lang::Pl => format!("Windows zapisał wybudzenie komputera {at}"),
    }
}

pub fn ev_log_driver(provider: &str, id: u32, at: &str) -> String {
    match current() {
        Lang::En => format!("the driver {provider} logged error {id} {at}"),
        Lang::Pl => format!("sterownik {provider} zapisał błąd {id} {at}"),
    }
}

pub fn ev_log_wlan_reason(code: u32, at: &str) -> String {
    let reason = wlan_reason(code);
    match current() {
        Lang::En => format!("Windows logged the disconnect {at}: {reason} (802.11 reason {code})"),
        Lang::Pl => format!("Windows zapisał rozłączenie {at}: {reason} (802.11, kod {code})"),
    }
}

pub fn ev_log_wlan_plain(at: &str) -> String {
    match current() {
        Lang::En => format!("Windows logged the wireless disconnect {at}, without a reason code"),
        Lang::Pl => format!("Windows zapisał rozłączenie Wi-Fi {at}, bez kodu przyczyny"),
    }
}

pub fn ev_log_wlan_auth(id: u32, at: &str) -> String {
    match current() {
        Lang::En => format!("WLAN-AutoConfig logged event {id} {at}: the association was refused"),
        Lang::Pl => format!("WLAN-AutoConfig zapisał zdarzenie {id} {at}: odmowa powiązania"),
    }
}

pub fn ev_log_dhcp(at: &str) -> String {
    match current() {
        Lang::En => format!("the DHCP client failed to obtain or renew a lease {at}"),
        Lang::Pl => format!("klient DHCP nie uzyskał ani nie odnowił dzierżawy {at}"),
    }
}

pub fn ev_log_duplicate_ip(at: &str) -> String {
    match current() {
        Lang::En => format!("TCP/IP logged a duplicate address on this network {at}"),
        Lang::Pl => format!("TCP/IP zapisał zduplikowany adres w tej sieci {at}"),
    }
}

pub fn ev_log_link_down(at: &str) -> String {
    match current() {
        Lang::En => format!("Windows logged the network interface going down {at}"),
        Lang::Pl => format!("Windows zapisał wyłączenie interfejsu sieciowego {at}"),
    }
}

pub fn ev_log_clean(lines: usize) -> String {
    match current() {
        Lang::En => format!(
            "{lines} log entries around this outage and not one of them a fault on this machine"
        ),
        Lang::Pl => format!(
            "{lines} wpisów w dzienniku wokół tej awarii i ani jeden z nich to usterka tego komputera"
        ),
    }
}

/// One-word label for a log line's meaning, for the raw list under the causes.
pub fn log_kind(kind: crate::probe::eventlog::Kind) -> &'static str {
    use crate::probe::eventlog::Kind as K;
    match (current(), kind) {
        (Lang::En, K::Sleep) => "sleep",
        (Lang::Pl, K::Sleep) => "uśpienie",
        (Lang::En, K::Resume) => "wake",
        (Lang::Pl, K::Resume) => "wybudzenie",
        (Lang::En, K::WlanDisconnect) => "Wi-Fi disconnect",
        (Lang::Pl, K::WlanDisconnect) => "rozłączenie Wi-Fi",
        (Lang::En, K::WlanAuthFail) => "Wi-Fi refused",
        (Lang::Pl, K::WlanAuthFail) => "odmowa Wi-Fi",
        (Lang::En, K::WlanConnect) => "Wi-Fi connected",
        (Lang::Pl, K::WlanConnect) => "połączenie Wi-Fi",
        (Lang::En, K::LinkDown) => "interface down",
        (Lang::Pl, K::LinkDown) => "interfejs w dół",
        (Lang::En, K::LinkUp) => "interface up",
        (Lang::Pl, K::LinkUp) => "interfejs w górę",
        (Lang::En, K::DhcpFail) => "DHCP failure",
        (Lang::Pl, K::DhcpFail) => "błąd DHCP",
        (Lang::En, K::DuplicateIp) => "duplicate address",
        (Lang::Pl, K::DuplicateIp) => "zduplikowany adres",
        (Lang::En, K::DriverFault) => "driver error",
        (Lang::Pl, K::DriverFault) => "błąd sterownika",
    }
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
            format!("Jitter {jitter:.0} ms. Latency is swinging, which shows up as lag in games.")
        }
        Lang::Pl => {
            format!("Jitter {jitter:.0} ms. Opóźnienie skacze, co w grach objawia się jako lagi.")
        }
    }
}

pub fn mon_ping_detail(worst: f64) -> String {
    match current() {
        Lang::En => format!("Ping {worst:.0} ms, above the playable threshold."),
        Lang::Pl => format!("Ping {worst:.0} ms, powyżej progu grywalności."),
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
        Lang::En => format!("1.1.1.1 \u{b7} min {min:.0} / max {max:.0}"),
        Lang::Pl => format!("1.1.1.1 \u{b7} min {min:.0} / maks {max:.0}"),
    }
}

/// Sub-line under the router card.
pub fn live_router_loss(pct: f64) -> String {
    match current() {
        Lang::En => format!("first hop \u{b7} loss {pct:.1}%"),
        Lang::Pl => format!("pierwszy skok \u{b7} strata {pct:.1}%"),
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

// --- the radio and the stack -----------------------------------------------

pub fn tw_adv_wrong_medium(medium: &str) -> String {
    match current() {
        Lang::En => format!("not applicable, this is a {medium} setting"),
        Lang::Pl => format!("nie dotyczy, to ustawienie dla: {medium}"),
    }
}

pub fn tw_adv_now_vs_wanted(now: &str, wanted: &str) -> String {
    match current() {
        Lang::En => format!("{now}, could be {wanted}"),
        Lang::Pl => format!("{now}, mogłoby być: {wanted}"),
    }
}

pub fn tw_adv_unset(wanted: &str) -> String {
    match current() {
        Lang::En => format!("driver default, could be {wanted}"),
        Lang::Pl => format!("domyślne sterownika, mogłoby być: {wanted}"),
    }
}

pub fn tw_adv_applied(name: &str, label: &str) -> String {
    match current() {
        Lang::En => format!("{name} set to {label}. Takes effect after a restart."),
        Lang::Pl => format!("{name} ustawione na {label}. Zadziała po ponownym uruchomieniu."),
    }
}

pub fn tw_adv_reverted(name: &str) -> String {
    match current() {
        Lang::En => format!("{name} restored. Takes effect after a restart."),
        Lang::Pl => format!("{name} przywrócone. Zadziała po ponownym uruchomieniu."),
    }
}

pub fn tw_dw_state(name: &str, value: u32) -> String {
    format!("{name} = {value}")
}

pub fn tw_dw_unset(name: &str) -> String {
    match current() {
        Lang::En => format!("{name} not set, Windows default"),
        Lang::Pl => format!("{name} nieustawione, domyślne Windows"),
    }
}

pub fn tw_dw_applied(name: &str, value: u32) -> String {
    match current() {
        Lang::En => format!("{name} set to {value}."),
        Lang::Pl => format!("{name} ustawione na {value}."),
    }
}

pub fn tw_dw_reverted(name: &str, value: u32) -> String {
    match current() {
        Lang::En => format!("{name} restored to {value}."),
        Lang::Pl => format!("{name} przywrócone do {value}."),
    }
}

pub fn tw_dw_removed(name: &str) -> String {
    match current() {
        Lang::En => format!("{name} removed, back to the Windows default."),
        Lang::Pl => format!("{name} usunięte, z powrotem domyślne Windows."),
    }
}

pub fn tw_cong_applied(provider: &str) -> String {
    match current() {
        Lang::En => format!("Internet connections now use {provider}."),
        Lang::Pl => format!("Połączenia internetowe używają teraz {provider}."),
    }
}

pub fn tw_cong_applied_fallback(provider: &str) -> String {
    match current() {
        Lang::En => format!("This Windows has no BBR2, so {provider} was set instead."),
        Lang::Pl => format!("Ten Windows nie ma BBR2, więc ustawiono {provider}."),
    }
}

pub fn tw_cong_reverted(provider: &str) -> String {
    match current() {
        Lang::En => format!("Congestion control restored to {provider}."),
        Lang::Pl => format!("Kontrola przeciążenia przywrócona do {provider}."),
    }
}

// --- the air scan ----------------------------------------------------------

pub fn air_seen(networks: usize, co_channel: usize) -> String {
    match current() {
        Lang::En => format!("{networks} networks in range, {co_channel} of them on your channel"),
        Lang::Pl => format!("{networks} sieci w zasięgu, {co_channel} na twoim kanale"),
    }
}

pub fn air_current_line(channel: u32, noise: Option<f64>) -> String {
    let noise = match noise {
        Some(n) => format!("{n:.0} dBm"),
        None => match current() {
            Lang::En => "nothing else heard".to_string(),
            Lang::Pl => "nic innego nie słychać".to_string(),
        },
    };
    match current() {
        Lang::En => format!("You are on channel {channel}; interference there: {noise}"),
        Lang::Pl => format!("Jesteś na kanale {channel}; zakłócenia tam: {noise}"),
    }
}

pub fn air_best_24(channel: u32, gain: f64) -> String {
    match current() {
        Lang::En => format!("Ask the router for 2.4 GHz channel {channel}, {gain:.0} dB quieter"),
        Lang::Pl => format!("Poproś router o kanał {channel} na 2,4 GHz, o {gain:.0} dB ciszej"),
    }
}

pub fn air_on_dfs(channel: u32) -> String {
    match current() {
        Lang::En => format!(
            "Channel {channel} is a radar channel. The router has to vacate it within \
             ten seconds of thinking it heard radar, and stay off for thirty minutes — \
             an outage with no cause visible from here."
        ),
        Lang::Pl => format!(
            "Kanał {channel} to kanał radarowy. Router musi go opuścić w ciągu dziesięciu \
             sekund od chwili, gdy wyda mu się, że usłyszał radar, i nie wraca przez pół \
             godziny — czyli awaria bez przyczyny widocznej z tej strony."
        ),
    }
}

pub fn air_best_5(channel: u32) -> String {
    match current() {
        Lang::En => format!("Quietest radar-free 5 GHz channel: {channel}"),
        Lang::Pl => format!("Najspokojniejszy kanał 5 GHz bez radaru: {channel}"),
    }
}

pub fn air_load_cell(channel: u32, aps: usize, noise: Option<f64>) -> String {
    let noise = match noise {
        Some(n) => format!("{n:.0} dBm"),
        None => "—".to_string(),
    };
    match current() {
        Lang::En => format!("ch {channel}: {aps} networks, {noise}"),
        Lang::Pl => format!("kan. {channel}: {aps} sieci, {noise}"),
    }
}

pub fn air_channel_cell(channel: u32, band: &str) -> String {
    format!("{channel} · {band}")
}

pub fn air_failed(err: &str) -> String {
    match current() {
        Lang::En => format!("scan failed: {err}"),
        Lang::Pl => format!("skanowanie nie powiodło się: {err}"),
    }
}

// --- the optimise list -----------------------------------------------------

pub fn opt_summary(set: usize, todo: usize, na: usize) -> String {
    match current() {
        Lang::En => format!("{set} set · {todo} worth changing · {na} not available"),
        Lang::Pl => format!("{set} ustawionych · {todo} do poprawy · {na} niedostępnych"),
    }
}

pub fn opt_section_count(set: usize, total: usize) -> String {
    match current() {
        Lang::En => format!("{set} of {total} set"),
        Lang::Pl => format!("{set} z {total} ustawione"),
    }
}

// --- updates ---------------------------------------------------------------

pub fn upd_running(version: &str) -> String {
    match current() {
        Lang::En => format!("Running version {version}."),
        Lang::Pl => format!("Zainstalowana wersja: {version}."),
    }
}

pub fn upd_up_to_date(version: &str) -> String {
    match current() {
        Lang::En => format!("Version {version} is the latest one."),
        Lang::Pl => format!("Wersja {version} jest najnowsza."),
    }
}

pub fn upd_available(version: &str) -> String {
    match current() {
        Lang::En => format!("Version {version} is available."),
        Lang::Pl => format!("Dostępna jest wersja {version}."),
    }
}

pub fn upd_installed(version: &str) -> String {
    match current() {
        Lang::En => format!("Version {version} is installed."),
        Lang::Pl => format!("Wersja {version} została zainstalowana."),
    }
}

pub fn upd_failed(detail: &str) -> String {
    match current() {
        Lang::En => format!("The update failed: {detail}"),
        Lang::Pl => format!("Aktualizacja się nie udała: {detail}"),
    }
}

pub fn upd_err_no_asset(version: &str, asset: &str) -> String {
    match current() {
        Lang::En => format!("Release {version} has no {asset} attached to it."),
        Lang::Pl => format!("Wydanie {version} nie ma dołączonego pliku {asset}."),
    }
}

pub fn upd_err_no_dir() -> String {
    match current() {
        Lang::En => "The running program has no directory to install into.".into(),
        Lang::Pl => "Nie udało się ustalić katalogu uruchomionego programu.".into(),
    }
}

pub fn upd_err_read_only(dir: &str) -> String {
    match current() {
        Lang::En => format!(
            "{dir} cannot be written to. Move the program somewhere you own, such as your user \
             folder, or download the new version by hand."
        ),
        Lang::Pl => format!(
            "Nie można zapisywać w katalogu {dir}. Przenieś program w miejsce, do którego masz \
             prawa zapisu, na przykład do swojego folderu użytkownika, albo pobierz nową wersję \
             ręcznie."
        ),
    }
}

pub fn upd_err_too_small(bytes: u64) -> String {
    match current() {
        Lang::En => format!("The download stopped after {bytes} bytes, which is not a full build."),
        Lang::Pl => {
            format!("Pobieranie zakończyło się po {bytes} bajtach, to nie jest cała aplikacja.")
        }
    }
}

pub fn upd_err_size_mismatch(got: u64, want: u64) -> String {
    match current() {
        Lang::En => format!("The download is {got} bytes; the release says {want}."),
        Lang::Pl => format!("Pobrany plik ma {got} bajtów, a wydanie podaje {want}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds the language lock for a whole test.
    ///
    /// Every assertion on translated text belongs inside one of these —
    /// including the English ones. The table is global, so a test reading
    /// English while another test sits inside its Polish block fails for a
    /// reason that has nothing to do with either of them. Locking only around
    /// the Polish half left exactly that race, and it showed up as an
    /// occasional failure in whichever test happened to lose.
    fn with_language_lock<T>(body: impl FnOnce() -> T) -> T {
        let _guard = test_lock();
        set(Lang::En);
        let out = body();
        set(Lang::En);
        out
    }

    /// Switches to Polish inside a `with_language_lock` block. Deliberately
    /// takes no lock of its own: `std::sync::Mutex` is not reentrant, so
    /// doing so would deadlock the caller that already holds it.
    fn in_polish<T>(body: impl FnOnce() -> T) -> T {
        set(Lang::Pl);
        let out = body();
        set(Lang::En);
        out
    }

    #[test]
    fn switching_language_changes_the_accessors() {
        with_language_lock(|| {
            assert_eq!(tab_live(), "Live");
            assert_eq!(sev_critical(), "CRITICAL");

            in_polish(|| {
                assert_eq!(tab_live(), "Na żywo");
                assert_eq!(sev_critical(), "KRYTYCZNE");
            });

            assert_eq!(tab_live(), "Live");
        });
    }

    #[test]
    fn parameterised_strings_follow_the_language_too() {
        with_language_lock(|| {
            assert!(scan_critical(2, "MTU").starts_with("2 serious"));
            in_polish(|| {
                let pl = scan_critical(2, "MTU");
                assert!(pl.starts_with("Znaleziono"), "{pl}");
                assert!(pl.contains("MTU"), "the argument must survive: {pl}");
            });
        });
    }

    #[test]
    fn event_kinds_translate_and_unknown_codes_pass_through() {
        with_language_lock(|| {
            assert_eq!(event_kind("lan_down"), "router unreachable");
            in_polish(|| assert_eq!(event_kind("lan_down"), "router nieosiągalny"));
            // A code written by a future version must not become an empty cell.
            assert_eq!(event_kind("something_new"), "something_new");
        });
    }

    #[test]
    fn no_string_carries_the_indent_of_its_own_source() {
        // A wrapped literal keeps its lines joined with a trailing backslash,
        // which eats the newline and the indent after it. Lose the backslash
        // and the indent becomes part of the string: the UI showed "Oba są
        // <ten spaces> zbudowane", in every text long enough to wrap. It is
        // invisible in the source and obvious on screen, which is the wrong
        // way round, so the test looks for it instead of a reader.
        //
        // Three spaces rather than two: a couple of strings line up columns or
        // indent a note on purpose, and none of them does it with three.
        for (name, en, pl) in ALL_STRINGS {
            for text in [en, pl] {
                // Block text — the CLI help — is laid out on purpose and is
                // the one place runs of spaces mean something. It gives itself
                // away by indenting a line; prose never starts a line with a
                // space, it only ever separates paragraphs with a blank one.
                if text.contains("\n ") || text.contains("\n\t") {
                    continue;
                }
                assert!(!text.contains("   "), "{name}: a run of spaces from the source indent");
            }
        }
    }

    #[test]
    fn polish_text_is_actually_polish() {
        with_language_lock(|| {
            in_polish(|| {
                // Guards against a key added with the English string pasted into
                // both slots, which compiles and silently ships untranslated.
                assert_ne!(mon_ok(), "Connection healthy");
                assert_ne!(f_wired(), "Wired connection");
                assert_ne!(tw_power_title(), "Stop Windows powering down the network adapter");
            })
        });
    }
}
