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
    tab_bloat => "Speed test", "Test prędkości";
    tab_optimise => "Optimise", "Optymalizacja";
    tab_history => "Outage history", "Historia awarii";
    tab_settings => "Settings", "Ustawienia";

    // The notification-area icon: its menu and the tooltip's two states that
    // are not a verdict.
    tray_open => "Open NetDoctor", "Otwórz NetDoctor";
    tray_quit => "Quit", "Zakończ";
    tray_game_no_game =>
        "Prepare the connection for a game (start a game first)",
        "Przygotuj łącze do gry (najpierw uruchom grę)";
    tray_game_no_admin =>
        "Prepare the connection for a game (needs administrator)",
        "Przygotuj łącze do gry (wymaga administratora)";
    tray_game_end => "End game mode and undo its changes", "Zakończ tryb gry i cofnij zmiany";
    tray_game_title => "NetDoctor game mode", "NetDoctor: tryb gry";
    game_needs_admin =>
        "Game mode changes need administrator rights. Restart NetDoctor as administrator.",
        "Zmiany trybu gry wymagają uprawnień administratora. Uruchom NetDoctora ponownie jako \
         administrator.";
    game_already =>
        "Game mode is already on. End it first.",
        "Tryb gry jest już włączony. Najpierw go zakończ.";
    game_restored =>
        "Game mode ended: everything it changed is back as it was.",
        "Tryb gry zakończony: wszystko, co zmienił, wróciło do poprzedniego stanu.";
    game_restore_partial =>
        "Game mode could not undo everything. NetDoctor will try again when it next starts.",
        "Tryb gry nie cofnął wszystkiego. NetDoctor spróbuje ponownie przy następnym \
         uruchomieniu.";
    game_nothing_to_do =>
        "Nothing to change: updates are not downloading and Wi-Fi power saving is already off.",
        "Nie ma nic do zmiany: aktualizacje nie są pobierane, a oszczędzanie energii Wi-Fi jest \
         już wyłączone.";
    ov_not_measuring => "Not measuring", "Nie mierzę";
    ov_router_label => "LAN", "LAN";
    ov_internet_label => "NET", "NET";
    ov_local => "Spike: Wi-Fi/router", "Skok: Wi-Fi/router";
    ov_beyond => "Spike: line/provider", "Skok: łącze/dostawca";
    set_game_overlay =>
        "Show the ping overlay while a game runs",
        "Pokazuj nakładkę z pingiem podczas gry";
    set_overlay_corner => "Corner", "Róg ekranu";
    set_overlay_opacity => "Visibility", "Widoczność";
    set_overlay_size => "Size", "Rozmiar";
    size_small => "small", "mały";
    size_medium => "medium", "średni";
    size_large => "large", "duży";
    set_overlay_content => "Shows", "Pokazuje";
    content_full => "ping and verdict", "ping i ocenę";
    content_ping => "router and internet", "router i internet";
    content_internet => "internet only", "sam internet";
    corner_top_left => "top left", "lewy górny";
    corner_top_right => "top right", "prawy górny";
    corner_bottom_left => "bottom left", "lewy dolny";
    corner_bottom_right => "bottom right", "prawy dolny";
    tray_paused => "NetDoctor: measuring paused", "NetDoctor: pomiar wstrzymany";
    tray_waiting => "NetDoctor: waiting for the first reading", "NetDoctor: czekam na pierwszy pomiar";

    btn_dismiss => "Dismiss", "Zamknij";
    btn_apply => "Apply", "Zastosuj";
    btn_refresh => "Refresh", "Odśwież";
    opt_card_what => "What it does", "Co robi";
    opt_card_why => "Why it helps", "Dlaczego pomaga";
    opt_more => "Click for the full description", "Kliknij, żeby zobaczyć pełny opis";
    opt_less => "Click to fold it away", "Kliknij, żeby zwinąć";
    opt_btn_apply_anyway => "Apply anyway", "Zastosuj mimo to";
    opt_confirm_risky =>
        "This change is high risk or cannot be undone. Check the description before applying it.",
        "Ta zmiana jest ryzykowna albo nieodwracalna. Sprawdź opis przed zastosowaniem.";
    opt_headline_done => "Everything here is already set.", "Wszystko tu jest już ustawione.";

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

EXIT CODES (--scan):
    0  the line is sound, or only a setting on this PC is worth changing
    1  a fault was found now, or outages were recorded in the last 24 hours
    2  nothing could be decided (pings could not be sent, bad flag, no database)

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

KODY WYJŚCIA (--scan):
    0  łącze sprawne albo warto zmienić tylko ustawienie na tym komputerze
    1  znaleziono usterkę teraz albo w ostatnich 24 godzinach zapisano awarie
    2  nic nie ustalono (nie dało się wysłać pingów, zła flaga, brak bazy)

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

    set_interval => "Interval between sweeps", "Odstęp między seriami";
    set_interval_hint =>
        "Lower is more detailed but adds traffic. Between 300 ms and 30 s.",
        "Mniej znaczy dokładniej, ale więcej ruchu. Od 300 ms do 30 s.";
    set_ping_timeout => "Ping timeout", "Limit czasu pingu";
    set_fails_before_alarm => "Failed sweeps before an alarm", "Nieudane serie przed alarmem";
    set_fails_hint =>
        "Guards against logging a single dropped packet as an outage.",
        "Chroni przed zapisaniem pojedynczego zgubionego pakietu jako awarii.";
    set_keep_days => "Keep measurements for", "Przechowuj pomiary przez";

    set_targets_hint =>
        "One host per line, for example the game server you play on. Names are resolved when \
         settings are saved.",
        "Jeden host w linii, na przykład serwer gry, na którym grasz. Nazwy są rozwiązywane \
         przy zapisie ustawień.";

    set_lat_good => "Latency still good up to", "Opóźnienie jeszcze dobre do";
    set_lat_bad => "Latency bad above", "Opóźnienie złe powyżej";
    set_jitter_good => "Jitter good up to", "Jitter dobry do";
    set_jitter_ok => "Jitter acceptable up to", "Jitter akceptowalny do";
    set_loss_ok => "Acceptable packet loss", "Akceptowalna utrata pakietów";

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
    set_btn_discard => "Discard changes", "Odrzuć zmiany";
    set_unsaved => "Unsaved changes", "Niezapisane zmiany";
    set_all_saved => "All changes saved", "Wszystko zapisane";
    set_defaults_loaded =>
        "Default values filled in. Save to keep them, or discard to go back.",
        "Wpisano wartości domyślne. Zapisz, żeby je zachować, albo odrzuć, żeby wrócić.";
    set_instant_hint =>
        "Changes on this page take effect immediately, without saving.",
        "Zmiany na tej stronie działają od razu, bez zapisywania.";
    set_unit_days => "days", "dni";

    set_page_measure => "Measurement", "Pomiar";
    set_page_targets => "Ping targets", "Cele pingowania";
    set_page_overlay => "Game overlay", "Nakładka w grze";
    set_page_general => "General", "Ogólne";
    set_page_ai => "AI explanation", "Wyjaśnienie AI";

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
    live_card_loss => "Packet loss", "Utrata pakietów";
    live_card_too_few => "too few readings yet", "jeszcze za mało pomiarów";
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
        "The share of packets that never came back over the last minute, from whichever \
         internet target lost the least: loss on the line shows on every target, loss on one \
         alone is that target. It is the figure the headline is judged on.

Whatever goes \
         missing has to be sent again, which is why a percent or two can hurt more than a high \
         latency. Steady loss points at the link; loss in short bursts usually means something \
         on the way was briefly overloaded.",
        "Udział pakietów, które nigdy nie wróciły, z ostatniej minuty, z tego celu w internecie, \
         który stracił najmniej: stratę na łączu widać na każdym celu, strata na jednym to sprawa \
         tego celu. Na tej liczbie opiera się nagłówek.

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
    diag_scan_hint =>
        "A quick scan takes about 30 seconds; with the load test, about 50.",
        "Szybki skan trwa około 30 sekund, z testem obciążenia około 50.";
    diag_scan_done => "Scan complete.", "Skan zakończony.";
    diag_scan_starting => "Starting…", "Uruchamianie…";
    diag_btn_scan => "Run scan", "Uruchom skan";
    diag_title =>
        "Connection diagnosis",
        "Diagnoza połączenia";
    diag_blurb =>
        "The scan checks every link of the connection, from the network card to the internet, and \
         names the one where the problem starts.",
        "Skan sprawdza każde ogniwo połączenia, od karty sieciowej po internet, i wskazuje to, na \
         którym zaczyna się problem.";
    diag_btn_rescan =>
        "Scan again",
        "Skanuj ponownie";
    diag_quick_note =>
        "Without the load test the scan sends only pings and a few small connections.",
        "Bez testu obciążenia skan wysyła tylko pingi i kilka małych połączeń.";
    diag_running_title =>
        "Scan in progress",
        "Trwa skan";
    diag_empty_title =>
        "What the scan checks",
        "Co sprawdza skan";
    diag_empty_1 =>
        "Connection: the network card, the Wi-Fi signal and band, power saving, a VPN and a \
         proxy.",
        "Połączenie: karta sieciowa, sygnał i pasmo Wi-Fi, oszczędzanie energii, VPN i proxy.";
    diag_empty_2 =>
        "Path: ping and loss on every link at once, from this PC to the router, the provider and \
         the internet.",
        "Trasa: ping i straty na każdym odcinku naraz, od komputera do routera, dostawcy i \
         internetu.";
    diag_empty_3 =>
        "Services: DNS (yours and a public one), IPv6, MTU and a real connection on port 443.",
        "Usługi: DNS (Twój i publiczny), IPv6, MTU i prawdziwe połączenie na porcie 443.";
    diag_empty_4 =>
        "Optionally, the load test: whether the ping survives a download and an upload.",
        "Opcjonalnie test obciążenia: czy ping wytrzymuje pobieranie i wysyłanie.";
    diag_empty_5 =>
        "Context: this line's usual ping from the last 7 days, the outage history and the Windows \
         log from the scan.",
        "Kontekst: typowy ping tego łącza z ostatnich 7 dni, historia awarii i dziennik Windows z \
         czasu skanu.";
    measure_route =>
        "Route",
        "Trasa";
    measure_route_direct =>
        "directly through the card",
        "bezpośrednio przez kartę";
    measure_proxy =>
        "Windows proxy",
        "Proxy w Windows";
    verdict_vpn_first =>
        "Turn the VPN off and scan again: this result measures the VPN, not your own line.",
        "Wyłącz VPN i uruchom skan ponownie: ten wynik mierzy VPN, a nie Twoje łącze.";
    step_route =>
        "Checking VPN and proxy",
        "Sprawdzanie VPN i proxy";
    f_vpn =>
        "Traffic goes through a VPN",
        "Ruch idzie przez VPN";
    f_vpn_advice =>
        "Every number in this scan includes the trip to the VPN server, and the \"router\" is the \
         VPN's own end. To diagnose your own line, turn the VPN off and scan again.",
        "Każdy wynik tego skanu zawiera drogę do serwera VPN, a „router” to koniec tunelu VPN. \
         Aby zdiagnozować własne łącze, wyłącz VPN i uruchom skan ponownie.";
    f_proxy =>
        "Windows sends programs through a proxy",
        "Windows kieruje programy przez proxy";
    f_proxy_advice =>
        "Browsers use it and pings do not, so pages can fail while ping works. If it is not \
         needed, turn it off in Settings > Network and internet > Proxy.",
        "Korzystają z niego przeglądarki, a pingi nie, więc strony mogą nie działać, choć ping \
         działa. Jeśli nie jest potrzebne, wyłącz je w Ustawieniach > Sieć i Internet > Serwer \
         proxy.";
    f_other_medium =>
        "Connected through another adapter",
        "Połączenie przez inną kartę";
    set_sec_line =>
        "Your line",
        "Twoje łącze";
    set_line_kind =>
        "Kind of line",
        "Rodzaj łącza";
    set_line_hint =>
        "The app cannot tell this by itself: a radio antenna or an LTE modem looks like plain \
         Ethernet to this computer. The kind sets the ping to expect until the app has its own \
         history, and fits the advice to the line.",
        "Aplikacja nie rozpozna tego sama: antena radiowa czy modem LTE wyglądają dla komputera \
         jak zwykły Ethernet. Rodzaj łącza wyznacza oczekiwany ping, dopóki aplikacja nie ma \
         własnej historii, i dopasowuje porady do łącza.";
    set_plan_down =>
        "Plan speed: download",
        "Prędkość z umowy: pobieranie";
    set_plan_up =>
        "Plan speed: upload",
        "Prędkość z umowy: wysyłanie";
    set_plan_hint =>
        "0 means not given. The speed test compares its result with it.",
        "0 oznacza, że nie podano. Test prędkości porówna z nią swój wynik.";
    line_unknown =>
        "Don't know",
        "Nie wiem";
    line_fibre =>
        "Fibre",
        "Światłowód";
    line_cable =>
        "Cable (cable TV network)",
        "Kablowe (sieć telewizji kablowej)";
    line_dsl =>
        "DSL (phone line)",
        "DSL (linia telefoniczna)";
    line_radio =>
        "Radio (antenna on the building)",
        "Radiowe (antena na budynku)";
    line_mobile =>
        "LTE / 5G",
        "LTE / 5G";
    line_sat_leo =>
        "Satellite (Starlink and similar)",
        "Satelitarne (Starlink i podobne)";
    line_sat_geo =>
        "Satellite (geostationary)",
        "Satelitarne (geostacjonarne)";
    f_line_detail =>
        "The range is what a healthy line of this kind usually has to a nearby server (1.1.1.1). \
         The kind of line is set in Settings > Measurement.",
        "Zakres to typowy ping zdrowego łącza tego rodzaju do pobliskiego serwera (1.1.1.1). \
         Rodzaj łącza ustawia się w Ustawieniach > Pomiar.";
    measure_line =>
        "Kind of line",
        "Rodzaj łącza";
    measure_line_unset =>
        "not given (Settings > Measurement)",
        "nie podano (Ustawienia > Pomiar)";
    rep_line =>
        "Line",
        "Łącze";
    rep_plan =>
        "Plan",
        "Pakiet";
    diag_select_finding =>
        "Select a finding to see what it means.",
        "Wybierz wynik, żeby zobaczyć, co oznacza.";
    diag_btn_fix => "Fix this", "Napraw to";
    diag_deep =>
        "Include the load test",
        "Dołącz test obciążenia";
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
        "Each entry keeps the connection state at the moment of the outage: signal, channel, \
         access point, and whether the router was still answering. That last one tells a fault on \
         this computer from a fault at the provider.",
        "Każdy wpis zawiera stan połączenia z chwili awarii: sygnał, kanał, access point i to, \
         czy router nadal odpowiadał. To ostatnie odróżnia usterkę po stronie komputera od \
         usterki u dostawcy.";
    hist_nothing_logged => "Nothing logged yet.", "Nic jeszcze nie zapisano.";
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
    btn_back_to_list => "Back to the list", "Wróć do listy";
    hist_btn_clear => "Clear history…", "Wyczyść historię…";
    hist_btn_clear_confirm => "Delete outages", "Usuń awarie";
    hist_btn_cancel => "Cancel", "Anuluj";
    hist_clear_confirm =>
        "Delete every recorded outage? This cannot be undone. To keep them as evidence for the \
         provider, save a report first.",
        "Usunąć wszystkie zapisane awarie? Tej operacji nie można cofnąć. Aby zachować je jako \
         dowód dla dostawcy, najpierw zapisz raport.";
    hist_clear_keeps_running =>
        "An outage still in progress is kept, so its end can be recorded.",
        "Trwająca awaria zostaje zachowana, żeby zapisać jej koniec.";
    hist_list_heading => "Recorded outages", "Zapisane awarie";
    hist_conn_heading => "Connection", "Połączenie";
    st_signal => "Signal", "Sygnał";
    st_channel => "Channel", "Kanał";
    st_band => "Band", "Pasmo";
    st_gateway => "Gateway", "Brama";
    hist_no_leadup =>
        "No lead-up recorded for this outage: the entry comes from an earlier version.",
        "Brak zapisanego przebiegu dla tej awarii: wpis pochodzi z wcześniejszej wersji.";
    hist_no_state =>
        "No connection state was stored for this entry.",
        "Dla tego wpisu nie zapisano stanu połączenia.";
    hist_leadup_rssi => "Signal", "Sygnał";
    hist_leadup_rtt => "Router", "Router";
    hist_leadup_net => "Internet", "Internet";
    hist_leadup_lost => "No reply", "Brak odpowiedzi";
    hist_leadup_outage => "Outage", "Awaria";
    hist_leadup_rtt_caption =>
        "Round trip (ms). Red dots: pings that got no reply; the shaded span is the outage.",
        "Czas odpowiedzi (ms). Czerwone kropki: pingi bez odpowiedzi; zacieniony pas to awaria.";
    hist_leadup_rssi_caption =>
        "Wi-Fi signal (dBm) up to the moment of the outage. Closer to zero is stronger.",
        "Sygnał Wi-Fi (dBm) do chwili awarii. Im bliżej zera, tym mocniejszy.";
    hist_leadup_weak => "Weak below -70 dBm", "Słaby poniżej -70 dBm";
    hist_leadup_no_signal =>
        "No Wi-Fi signal recorded for this outage: a wired connection, or an entry from an \
         earlier version.",
        "Brak zapisanego sygnału Wi-Fi dla tej awarii: połączenie kablowe albo wpis z \
         wcześniejszej wersji.";
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
    path_owner_local => "private (yours or the provider's)", "prywatny (twój albo dostawcy)";
    path_owner_edge => "provider edge", "brzeg dostawcy";
    path_owner_internet => "beyond the provider", "za dostawcą";

    hist_log_heading => "What Windows wrote down", "Co zapisał Windows";
    hist_log_loading =>
        "Reading the event log…",
        "Odczyt dziennika zdarzeń…";
    hist_log_none =>
        "No Windows event log entries from this period. Either nothing failed on this computer, \
         or the relevant log channels are turned off.",
        "Brak wpisów w dzienniku zdarzeń Windows z tego okresu. Albo nic nie zawiodło na tym \
         komputerze, albo odpowiednie kanały dziennika są wyłączone.";
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
    verdict_actions_heading => "What to do, in order", "Co zrobić, po kolei";
    chain_heading => "The connection, link by link", "Połączenie, ogniwo po ogniwie";
    chain_note =>
        "Under each link: the milliseconds it added and the packets lost on the way to its far end. \
         The link at fault is marked in colour.",
        "Pod każdym ogniwem: ile milisekund dokłada i ile pakietów ginie w drodze do jego końca. \
         Ogniwo winne jest wyróżnione kolorem.";
    node_pc => "This PC", "Ten komputer";
    node_router => "Router", "Router";
    node_isp => "Provider", "Dostawca";
    node_internet => "Internet", "Internet";
    link_silent => "no answer", "brak odpowiedzi";
    link_filtered => "ignores ping, traffic passes", "ignoruje ping, ruch przechodzi";
    link_unknown => "not established", "nie ustalono";
    link_not_measured => "not measured", "nie zmierzono";
    link_local => "your own second router", "Twój drugi router";
    measure_heading => "Measurements", "Pomiary";
    measure_col_leg => "Leg", "Odcinek";
    measure_col_addr => "Address", "Adres";
    measure_col_sent => "Sent", "Wysłano";
    measure_col_loss => "Loss", "Straty";
    measure_col_min => "Min", "Min";
    measure_col_avg => "Avg", "Śr.";
    measure_col_max => "Max", "Maks";
    measure_col_jitter => "Jitter", "Jitter";
    measure_leg_router => "To the router", "Do routera";
    measure_leg_isp => "To the provider", "Do dostawcy";
    measure_leg_internet => "To the internet", "Do internetu";
    measure_dns => "DNS answer", "Odpowiedź DNS";
    measure_tcp => "TCP connection (port 443)", "Połączenie TCP (port 443)";
    measure_tcp_blocked => "refused or timed out", "odrzucone lub przekroczony czas";
    measure_baseline => "Usual ping here (7-day median)", "Typowy ping tutaj (mediana 7 dni)";
    measure_baseline_none =>
        "not enough history yet", "za mało historii";
    measure_medium => "Connection", "Połączenie";
    measure_load => "Under load", "Pod obciążeniem";
    measure_load_skipped =>
        "not tested (tick the load test to include it)",
        "nie testowano (zaznacz test obciążenia, żeby go dołączyć)";
    measure_none => "not measured in this scan", "nie zmierzono w tym skanie";
    measure_blind =>
        "No ping could be sent, so no latency was measured. Reason:",
        "Nie dało się wysłać żadnego pingu, więc żaden czas nie został zmierzony. Powód:";
    measure_cumulative =>
        "Times are the full round trip to the end of each leg, not the leg alone.",
        "Czasy to pełny przelot do końca danego odcinka, nie sam odcinek.";
    findings_heading => "Individual checks", "Poszczególne kontrole";
    f_long_failed => "The long measurement could not run", "Długi pomiar nie mógł się odbyć";
    step_dns_compare =>
        "Comparing DNS with a public resolver",
        "Porównanie DNS z publicznym";
    step_ipv6 =>
        "Checking IPv6",
        "Sprawdzanie IPv6";
    step_syslog =>
        "Reading the Windows log",
        "Odczyt dziennika Windows";
    f_ipv6_ok_detail =>
        "A connection over IPv6 opened. Programs that prefer IPv6 get it without waiting.",
        "Połączenie przez IPv6 się otworzyło. Programy, które wolą IPv6, dostają je bez czekania.";
    f_ipv6_absent => "No IPv6 on this connection", "Brak IPv6 na tym połączeniu";
    f_ipv6_absent_detail =>
        "This machine has no IPv6 route, so everything goes over IPv4. That is normal on many \
         home lines and costs nothing.",
        "Ten komputer nie ma trasy IPv6, więc wszystko idzie przez IPv4. Na wielu domowych \
         łączach to normalne i niczego nie kosztuje.";
    f_ipv6_broken => "IPv6 is set up but does not work", "IPv6 jest ustawione, ale nie działa";
    f_ipv6_broken_detail =>
        "Windows has an IPv6 route, but connections over it time out. Browsers and games that \
         try IPv6 first wait for it to fail before falling back to IPv4, which feels like a slow \
         first load or a slow login.",
        "Windows ma trasę IPv6, ale połączenia przez nią nie dochodzą. Przeglądarki i gry, które \
         najpierw próbują IPv6, czekają, aż się nie uda, i dopiero wtedy przechodzą na IPv4. \
         Odczuwa się to jako wolne pierwsze ładowanie albo wolne logowanie.";
    f_ipv6_broken_advice =>
        "Restart the router. If it comes back, telling Windows to prefer IPv4 stops programs \
         waiting for IPv6 (Fix), and the provider can say whether IPv6 is meant to work on \
         your line.",
        "Zrestartuj router. Jeśli wróci, ustawienie Windowsa „preferuj IPv4” sprawi, że programy \
         przestaną czekać na IPv6 (Napraw), a dostawca powie, czy IPv6 ma na Twojej linii \
         działać.";
    f_ipv6_unknown => "IPv6 could not be checked", "Nie udało się sprawdzić IPv6";
    f_dns_compare_own_advice =>
        "This is a resolver you run yourself (a Pi-hole or similar). Check its upstream servers \
         and its load; the app will not offer to replace it.",
        "To Twój własny resolver (Pi-hole albo podobny). Sprawdź jego serwery nadrzędne i \
         obciążenie; aplikacja nie zaproponuje jego wymiany.";
    f_dns_compare_advice =>
        "Every new site waits for this answer before it starts loading. Switching to a public \
         resolver removes the wait.",
        "Każda nowa strona czeka na tę odpowiedź, zanim zacznie się ładować. Przejście na \
         publiczny resolver usuwa to czekanie.";
    f_dns_compare_public_failed =>
        "The public resolver did not answer", "Publiczny resolver nie odpowiedział";
    f_dns_compare_public_failed_detail =>
        "1.1.1.1 did not answer a DNS query from here. Some networks only allow their own \
         resolver; the comparison could not be made.",
        "1.1.1.1 nie odpowiedział stąd na zapytanie DNS. Niektóre sieci pozwalają tylko na \
         własny resolver; porównania nie dało się zrobić.";
    f_syslog_wifi =>
        "Windows logged the Wi-Fi dropping during the scan",
        "Windows zapisał rozłączenie Wi-Fi w trakcie skanu";
    f_syslog_wifi_advice =>
        "The card itself lost the access point. That is the air or the card: distance, \
         interference, or Windows putting the card to sleep. A cable test settles which.",
        "Karta sama zgubiła punkt dostępowy. To powietrze albo karta: odległość, zakłócenia albo \
         Windows usypiający kartę. Test na kablu rozstrzygnie, które.";
    measure_ipv6 => "IPv6", "IPv6";
    measure_dns_public => "DNS: yours vs public (best of 3)", "DNS: Twój kontra publiczny (najlepszy z 3)";
    measure_syslog => "Windows log during the scan", "Dziennik Windows w trakcie skanu";
    measure_syslog_none => "no connection faults", "brak usterek połączenia";
    ipv6_absent => "not available", "niedostępne";
    ipv6_broken => "set up, but connections time out", "ustawione, ale połączenia nie dochodzą";
    f_long_advice_lan =>
        "The drops start between this PC and the router. Try the same test on a cable, or \
         closer to the router: if they stop, it is the Wi-Fi (distance, channel, the card's \
         power saving).",
        "Zrywy zaczynają się między komputerem a routerem. Powtórz test na kablu albo bliżej \
         routera: jeśli znikną, winne jest Wi-Fi (odległość, kanał, oszczędzanie energii karty).";
    f_long_advice_isp =>
        "The drops start past your router, where the provider's network begins. Report them to \
         the provider with the times listed under the long measurement; nothing on this PC \
         will fix them.",
        "Zrywy zaczynają się za routerem, tam gdzie zaczyna się sieć dostawcy. Zgłoś je dostawcy \
         z godzinami z listy pod długim pomiarem; nic na tym komputerze tego nie naprawi.";
    f_long_advice_far =>
        "The drops happen past your provider's first hop, out in the internet. If only one \
         service suffers, it is that service; if everything does, show the provider the times.",
        "Zrywy powstają za pierwszym węzłem dostawcy, dalej w internecie. Jeśli cierpi tylko \
         jedna usługa, to jej problem; jeśli wszystko, pokaż dostawcy godziny zrywów.";
    diag_duration => "Measure for", "Czas pomiaru";
    diag_duration_quick => "quick (about 30 s)", "szybki (ok. 30 s)";
    diag_duration_hint =>
        "A quick scan sees half a minute. Drops that come a few times an hour need minutes: the \
         long measurement pings every link once a second and records where each drop starts.",
        "Szybki skan widzi pół minuty. Zrywy, które zdarzają się kilka razy na godzinę, wymagają \
         minut: długi pomiar pinguje każde ogniwo raz na sekundę i zapisuje, gdzie zaczyna się \
         każdy zryw.";
    diag_btn_stop => "Stop and show results", "Przerwij i pokaż wynik";
    long_heading => "Long measurement", "Długi pomiar";
    long_rows_note =>
        "One column per second. A mark shows the link where that second's trouble started: \
         red for lost packets, yellow for a latency spike.",
        "Jedna kolumna to jedna sekunda. Znacznik pokazuje ogniwo, na którym zaczął się kłopot w \
         tej sekundzie: czerwony to utrata pakietów, żółty to skok opóźnienia.";
    long_col_time => "Time", "Godzina";
    long_col_what => "What", "Co";
    long_col_where => "Where", "Gdzie";
    long_col_len => "Length", "Długość";
    long_no_episodes => "No drops or spikes recorded.", "Nie zapisano zrywów ani skoków.";
    ai_heading =>
        "AI explanation",
        "Wyjaśnienie AI";
    ai_off_hint =>
        "Optional: with your own OpenRouter key in Settings, a language model can explain this \
         result in plain words.",
        "Opcjonalnie: z własnym kluczem OpenRouter w Ustawieniach model językowy wyjaśni ten \
         wynik prostymi słowami.";
    ai_sec_why =>
        "What it rests on",
        "Na czym to opiera";
    ai_sec_steps =>
        "What to do",
        "Co zrobić";
    ai_sec_unknown =>
        "What the scan could not tell",
        "Czego skan nie ustalił";
    ai_btn_ask =>
        "Explain with AI",
        "Wyjaśnij z AI";
    ai_btn_again =>
        "Explain again",
        "Wyjaśnij ponownie";
    ai_preview => "Show exactly what will be sent", "Pokaż dokładnie, co zostanie wysłane";
    ai_running =>
        "The model is reading the scan…",
        "Model analizuje wynik skanu…";
    ai_err_no_key => "No OpenRouter key. Paste one in Settings.", "Brak klucza OpenRouter. Wklej go w Ustawieniach.";
    ai_err_empty => "it was empty", "była pusta";
    set_sec_ai => "AI explanation (optional)", "Wyjaśnienie AI (opcjonalne)";
    set_ai_key => "OpenRouter key", "Klucz OpenRouter";
    set_ai_model => "Model", "Model";
    set_ai_hint =>
        "Off while the key is empty. The key is your own (openrouter.ai/keys) and is kept in \
         this PC's settings file. Nothing is sent until you press the button on the Diagnose tab.",
        "Wyłączone, dopóki klucz jest pusty. Klucz jest Twój (openrouter.ai/keys) i zostaje w \
         pliku ustawień na tym komputerze. Nic nie jest wysyłane, dopóki nie naciśniesz przycisku \
         w zakładce Diagnostyka.";
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
    seg_healthy =>
        "Nowhere: the connection is sound",
        "Nigdzie: połączenie jest sprawne";
    seg_unmeasured => "Not established: the scan could not ping", "Nie ustalono: skan nie mógł pingować";
    cost_unmeasured =>
        "Unknown. The pings that cut the chain into segments could not be sent, so this scan \
         cannot say where a fault is, or whether there is one.",
        "Nieznany. Nie dało się wysłać pingów, które dzielą łańcuch na odcinki, więc ten skan \
         nie powie, gdzie jest usterka ani czy w ogóle jest.";
    f_icmp_blind =>
        "Pings could not be sent",
        "Nie dało się wysłać pingów";
    f_router_mute =>
        "The router does not answer pings, but traffic passes",
        "Router nie odpowiada na pingi, ale ruch przechodzi";
    f_icmp_filtered =>
        "Pings are filtered, but the internet is reachable",
        "Pingi są filtrowane, ale internet jest osiągalny";
    f_icmp_blind_advice =>
        "Something on this PC is blocking ICMP for the app (security software, policy). Run the \
         scan again; if it repeats, check what is blocking it.",
        "Coś na tym komputerze blokuje aplikacji ICMP (program zabezpieczający, zasady). Uruchom \
         skan ponownie; jeśli się powtórzy, sprawdź, co to blokuje.";

    cost_none =>
        "Nothing measurable. Calls, games and streaming should all behave.",
        "Nic mierzalnego. Rozmowy, gry i streaming powinny działać bez zarzutu.";
    cost_down =>
        "Nothing gets through this segment right now, so everything that depends on it is \
         stopped rather than slow.",
        "Przez ten odcinek w tej chwili nic nie przechodzi, więc wszystko, co od niego zależy, \
         stoi, a nie działa wolno.";
    cost_intermittent =>
        "Everything measures clean right now, so the fault is not constant, but outages were \
         recorded in the last 24 hours. A scan covers only half a minute; the outage history is \
         the better evidence here.",
        "W tej chwili wszystko mierzy się czysto, więc usterka nie jest stała, ale w ciągu \
         ostatnich 24 godzin zapisano awarie. Skan obejmuje tylko pół minuty; lepszym dowodem \
         jest tu historia awarii.";
    cost_config =>
        "The line itself measures clean. What is left are settings on this computer that work \
         against it: worth changing, but not the cause of a bad call.",
        "Samo łącze mierzy się czysto. Zostają ustawienia na tym komputerze, które mu szkodzą: \
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
        "This hop is still your own equipment: a second router, a mesh node, or a modem left in \
         router mode. Its latency counts as your network, not the provider's, so the split above \
         charges it to the LAN.",
        "Ten węzeł to nadal Twój własny sprzęt: drugi router, węzeł mesh albo modem w trybie \
         routera. Jego opóźnienie liczy się jako Twoja sieć, a nie dostawcy, dlatego podział \
         powyżej przypisuje je do LAN-u.";
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
        "ICMP gets through and TCP on port 443 does not. That points to a login page waiting on a \
         public network (captive portal), a firewall or a proxy, not a broken line. Open any page \
         in a browser to check.",
        "ICMP przechodzi, a TCP na porcie 443 już nie. To wskazuje na stronę logowania w sieci \
         publicznej (captive portal), firewall albo proxy, a nie na zepsute łącze. Sprawdzisz to, \
         otwierając dowolną stronę w przeglądarce.";
    f_tcp_slow_advice =>
        "The handshake takes far longer than the ping to the same place, which points at \
         filtering or an overloaded middlebox rather than at the line itself.",
        "Handshake trwa znacznie dłużej niż ping w to samo miejsce, co wskazuje na filtrowanie \
         albo przeciążony middlebox, a nie na samo łącze.";

    f_load_skipped => "Behaviour under load not measured", "Nie zmierzono zachowania pod obciążeniem";
    f_load_skipped_detail =>
        "The load test was left out of this scan. It is the most telling check for \"the internet \
         feels slow\", because it is the only one that reproduces it.",
        "Test obciążenia został pominięty w tym skanie. To najbardziej miarodajna kontrola przy \
         objawie „internet działa wolno”, bo jako jedyna go odtwarza.";
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
    f_apipa => "No address from the router", "Brak adresu od routera";
    f_apipa_advice =>
        "Restart the router, then reconnect. If it comes back, check that DHCP is on in the \
         router's settings and that the cable runs to a LAN port, not a WAN one.",
        "Zrestartuj router i połącz się ponownie. Jeśli wróci, sprawdź w ustawieniach routera, \
         czy DHCP jest włączone, i czy kabel idzie do portu LAN, a nie WAN.";
    f_wired => "Wired connection", "Połączenie przewodowe";
    f_wired_advice =>
        "The best possible starting point for latency.",
        "Najlepszy możliwy punkt wyjścia dla opóźnień.";
    f_link_errors_now => "The cable link is corrupting frames", "Łącze kablowe psuje ramki";
    f_link_errors_before =>
        "The cable link has corrupted frames since it came up",
        "Łącze kablowe psuło ramki od chwili podłączenia";
    f_link_errors_advice =>
        "Corrupted frames on a cable are almost always physical: a damaged or kinked cable, a \
         loose plug, a worn port. Try another cable first, then another port on the router.",
        "Uszkodzone ramki na kablu to prawie zawsze sprawa fizyczna: uszkodzony lub zagięty \
         kabel, luźna wtyczka, zużyty port. Najpierw spróbuj innego kabla, potem innego portu \
         w routerze.";
    f_link_slow => "The cable link runs at 100 Mbps or less", "Łącze kablowe działa z prędkością 100 Mbps lub niższą";
    f_link_slow_advice =>
        "If the router and this computer both have gigabit ports, a link this slow usually \
         means a cable with a broken pair or an old category 5 cable. If either side is a \
         100 Mbps port, this is simply its limit.",
        "Jeśli router i ten komputer mają porty gigabitowe, tak wolne łącze zwykle oznacza \
         kabel z uszkodzoną parą żył albo stary kabel kategorii 5. Jeśli któraś strona ma \
         port 100 Mbps, to po prostu jej limit.";
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
        "At this level the card drops frames and will disconnect now and then. Move closer, \
         reposition the router, or switch to 2.4 GHz for range at the cost of speed.",
        "Przy takim poziomie karta gubi ramki i co jakiś czas się rozłącza. Przenieś komputer \
         bliżej, przestaw router albo przejdź na 2,4 GHz: większy zasięg kosztem prędkości.";
    f_signal_mid_advice =>
        "Fine for browsing, but latency will spike under load.",
        "Do przeglądania wystarczy, ale pod obciążeniem opóźnienie będzie skakać.";
    f_band_24 =>
        "Running on the 2.4 GHz band",
        "Praca w paśmie 2,4 GHz";
    f_band_24_advice =>
        "2.4 GHz is shared with microwaves, Bluetooth and every neighbour. If the router offers 5 \
         GHz, connect to that network instead.",
        "Pasmo 2,4 GHz jest współdzielone z mikrofalówkami, Bluetooth i sieciami sąsiadów. Jeśli \
         router udostępnia 5 GHz, połącz się z tamtą siecią.";

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
    f_dns_own_failing_advice =>
        "This is a DNS server on your own network (Pi-hole, AdGuard Home or a company server). \
         Check that it is running. NetDoctor does not offer to replace it, because that would \
         switch off what you run it for.",
        "To serwer DNS w Twojej własnej sieci (Pi-hole, AdGuard Home albo serwer firmowy). \
         Sprawdź, czy działa. NetDoctor nie proponuje jego zamiany, bo wyłączyłoby to to, do \
         czego go używasz.";
    f_dns_slow_advice =>
        "Every new connection waits for this, which is why pages seem to stall before loading.",
        "Każde nowe połączenie czeka na tę odpowiedź, dlatego strony wyglądają, jakby zawieszały \
         się przed załadowaniem.";
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
         PC-to-router link: Wi-Fi, cabling or an overloaded router, not the provider.",
        "Ping do własnego routera powinien być poniżej 5 ms i bez strat. To wskazuje na odcinek \
         komputer-router: Wi-Fi, okablowanie albo przeciążony router, a nie dostawcę.";
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
        "Above 2% games start to stutter and TCP throughput collapses. Check the link to the \
         router first; if it is clean, the problem is further along.",
        "Powyżej 2% gry zaczynają się ciąć, a przepustowość TCP spada. Najpierw sprawdź łącze do \
         routera; jeśli jest czyste, problem leży dalej.";
    f_ping_high_advice =>
        "The link-by-link picture above shows where the delay is added; the Live tab traces the \
         path hop by hop.",
        "Rysunek ogniw powyżej pokazuje, gdzie powstaje opóźnienie; trasę skok po skoku pokazuje \
         zakładka Na żywo.";

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
        "The router answered but the internet did not. No Windows setting fixes this; save a \
         report with these times for the provider.",
        "Router odpowiadał, a internet nie. Żadne ustawienie Windowsa tego nie naprawi; zapisz \
         raport z tymi godzinami dla dostawcy.";
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
    mon_icmp_filtered =>
        "The internet is reachable, but this network blocks pings to it, so latency and loss \
         cannot be measured here.",
        "Internet jest osiągalny, ale ta sieć blokuje do niego pingi, więc opóźnień i strat \
         nie da się tu zmierzyć.";
    mon_no_gateway =>
        "No default gateway. This machine has no route to the network.",
        "Brak bramy domyślnej. Ten komputer nie ma trasy do sieci.";
    // The plain-language summary at the top of the Live tab.
    sum_ok => "Your internet is working normally.", "Internet działa normalnie.";
    sum_ok_todo =>
        "Nothing to do. NetDoctor keeps watching in the background and records every break, \
         with the evidence of where it happened.",
        "Nic nie musisz robić. NetDoctor pilnuje połączenia w tle i zapisze każdą przerwę, \
         razem z dowodem, gdzie do niej doszło.";
    sum_slow =>
        "Your internet works, but it is unstable right now.",
        "Internet działa, ale teraz jest niestabilny.";
    sum_slow_todo =>
        "Pages may load slowly and calls or games may stutter. Your router answers quickly, so \
         the trouble is past it: at your provider or further out on the internet. If it \
         lasts, run a check to see more.",
        "Strony mogą ładować się wolno, a rozmowy i gry mogą się zacinać. Router odpowiada \
         szybko, więc kłopot jest za nim: u dostawcy albo dalej w internecie. Jeśli to \
         potrwa, uruchom sprawdzenie, żeby zobaczyć więcej.";
    sum_slow_local_todo =>
        "Pages may load slowly and calls or games may stutter. Even your router is answering \
         slowly, so the trouble is in your home: often another device downloading a lot, or \
         the router being overloaded. Restarting the router often helps.",
        "Strony mogą ładować się wolno, a rozmowy i gry mogą się zacinać. Nawet router \
         odpowiada wolno, więc kłopot jest w domu: często inne urządzenie dużo pobiera albo \
         router jest przeciążony. Ponowne uruchomienie routera często pomaga.";
    sum_slow_weak_todo =>
        "The Wi-Fi signal here is weak. Move closer to the router or remove what stands \
         between you (walls, a microwave, a fish tank), or connect with a cable.",
        "Sygnał Wi-Fi jest tu słaby. Podejdź bliżej routera, usuń to, co stoi między wami \
         (ściany, mikrofalówka, akwarium), albo podłącz się kablem.";
    sum_dns =>
        "You are connected, but websites cannot be found by name.",
        "Połączenie jest, ale strony nie dają się znaleźć po nazwie.";
    sum_dns_todo =>
        "The service that turns names like google.com into addresses (DNS) is not answering. \
         Restarting the router often fixes it. The check shows which server is failing.",
        "Usługa, która zamienia nazwy takie jak google.pl na adresy (DNS), nie odpowiada. \
         Często pomaga ponowne uruchomienie routera. Sprawdzenie pokaże, który serwer zawodzi.";
    sum_isp =>
        "No internet: your router works, but it has no connection to your provider.",
        "Brak internetu: router działa, ale nie ma połączenia z dostawcą.";
    sum_isp_todo =>
        "Restarting this computer will not help. Look at the router's Internet or WAN light, \
         then check your provider's outage page or call them. NetDoctor is recording this \
         break: save a report and you have the evidence to show them.",
        "Ponowne uruchomienie komputera nie pomoże. Spójrz na lampkę Internet lub WAN na \
         routerze, potem sprawdź stronę awarii dostawcy albo zadzwoń do niego. NetDoctor \
         zapisuje tę przerwę: zapisz raport i masz dowód, który możesz mu pokazać.";
    sum_lan =>
        "No internet: this computer cannot reach your router.",
        "Brak internetu: ten komputer nie może połączyć się z routerem.";
    sum_lan_wifi_todo =>
        "Check that the router is switched on and its lights are lit. Move closer to it, or \
         turn Wi-Fi off and on again on this computer. If other devices have no internet \
         either, restart the router.",
        "Sprawdź, czy router jest włączony i świecą się jego lampki. Podejdź bliżej albo \
         wyłącz i włącz Wi-Fi w tym komputerze. Jeśli inne urządzenia też nie mają internetu, \
         uruchom router ponownie.";
    sum_lan_cable_todo =>
        "Check that the cable is firmly plugged in at both ends and that the router is \
         switched on. If other devices have no internet either, restart the router.",
        "Sprawdź, czy kabel jest dobrze wpięty z obu stron i czy router jest włączony. Jeśli \
         inne urządzenia też nie mają internetu, uruchom router ponownie.";
    sum_adapter =>
        "No internet: this computer is not connected to any network.",
        "Brak internetu: ten komputer nie jest połączony z żadną siecią.";
    sum_adapter_wifi_todo =>
        "Click the network icon in the corner of the screen and connect to your Wi-Fi again. \
         If it keeps dropping, the check can tell why.",
        "Kliknij ikonę sieci w rogu ekranu i połącz się ponownie ze swoim Wi-Fi. Jeśli \
         połączenie wciąż się zrywa, sprawdzenie może powiedzieć dlaczego.";
    sum_adapter_cable_todo =>
        "Check that the network cable is plugged in at both ends. A cable that clicks into \
         place and a light next to the socket mean it is in.",
        "Sprawdź, czy kabel sieciowy jest wpięty z obu stron. Kabel, który wskoczył na \
         miejsce z kliknięciem, i lampka przy gnieździe oznaczają, że jest wpięty.";
    sum_paused => "Watching is paused.", "Pilnowanie połączenia jest wstrzymane.";
    sum_paused_todo =>
        "While paused, NetDoctor records nothing, so a break now would go unnoticed.",
        "Podczas wstrzymania NetDoctor niczego nie zapisuje, więc przerwa w tym czasie \
         przejdzie niezauważona.";
    sum_waiting => "Checking your connection…", "Sprawdzam połączenie…";
    sum_waiting_todo =>
        "The first reading takes a moment.",
        "Pierwszy pomiar potrwa chwilę.";
    sum_blind =>
        "NetDoctor cannot measure right now.",
        "NetDoctor nie może teraz mierzyć.";
    sum_blind_todo =>
        "Windows did not let it send its test messages. This is not a verdict about your \
         internet. It usually passes; if it does not, restart NetDoctor.",
        "Windows nie pozwolił wysłać wiadomości testowych. To nie jest ocena Twojego \
         internetu. Zwykle mija samo, a jeśli nie, uruchom NetDoctora ponownie.";
    sum_unrecorded =>
        "The internet answers, but NetDoctor cannot record.",
        "Internet odpowiada, ale NetDoctor nie może zapisywać pomiarów.";
    sum_unrecorded_todo =>
        "Its database refused the latest readings, so loss and jitter are not judged and an \
         outage would not be recorded. Check the free space on the disk, then restart NetDoctor.",
        "Baza danych odrzuciła ostatnie pomiary, więc straty i jitter nie są oceniane, a awaria \
         nie zostałaby zapisana. Sprawdź wolne miejsce na dysku, potem uruchom NetDoctora \
         ponownie.";
    sum_stale =>
        "Measuring has stopped.",
        "Pomiar się zatrzymał.";
    sum_stale_todo =>
        "The last reading is too old to say anything about now. Restart NetDoctor.",
        "Ostatni pomiar jest zbyt stary, żeby mówić o tym, co dzieje się teraz. Uruchom \
         NetDoctora ponownie.";
    sum_node_pc => "This computer", "Ten komputer";
    sum_node_router => "Router", "Router";
    sum_node_internet => "Internet", "Internet";
    sum_link_wifi => "Wi-Fi", "Wi-Fi";
    sum_link_cable => "Cable", "Kabel";
    sum_link_provider => "Provider", "Dostawca";
    sum_link_good => "working", "działa";
    sum_link_slow => "slow", "wolno";
    sum_link_broken => "broken", "przerwane";
    sum_link_unknown => "unknown", "nie wiadomo";
    sum_btn_diagnose => "Run a check", "Uruchom sprawdzenie";
    sum_btn_report => "Save a report for my provider", "Zapisz raport dla dostawcy";
    live_details => "Technical details: the route hop by hop", "Szczegóły techniczne: trasa krok po kroku";
    word_signal_good => "good", "dobry";
    word_signal_fair => "fair", "średni";
    word_signal_weak => "weak", "słaby";
    mon_paused => "Measuring paused", "Pomiar wstrzymany";
    mon_waiting => "Waiting for the first reading", "Czekam na pierwszy pomiar";
    mon_stale =>
        "No fresh reading: measuring has stopped",
        "Brak świeżego pomiaru: pomiar przestał działać";
    mon_unrecorded =>
        "Not recording: readings cannot be saved",
        "Brak zapisu: nie da się zapisać pomiarów";
    mon_blind =>
        "Not measuring: Windows would not let the app send pings",
        "Brak pomiaru: Windows nie pozwolił aplikacji wysłać pingów";

    // -----------------------------------------------------------------------
    // load test tab
    // -----------------------------------------------------------------------
    bloat_title => "Speed and latency under load", "Prędkość i opóźnienie pod obciążeniem";
    bloat_blurb =>
        "Measures download and upload speed, and how far the ping rises while the line is busy. \
         Lag in games usually comes from that rise, not from the idle ping.",
        "Mierzy prędkość pobierania i wysyłania oraz to, o ile rośnie ping, gdy łącze jest \
         zajęte. Lagi w grach zwykle biorą się z tego wzrostu, a nie z pingu na bezczynnym łączu.";
    bloat_btn_run => "Run test", "Uruchom test";
    bloat_btn_rerun => "Run again", "Uruchom ponownie";
    bloat_btn_stop => "Stop", "Przerwij";
    bloat_cancelled =>
        "Test stopped. The previous result is unchanged.",
        "Test przerwany. Poprzedni wynik został bez zmian.";
    bloat_busy_scan =>
        "Wait for the scan in Diagnose to finish: both of them load the line.",
        "Poczekaj, aż skończy się skanowanie w Diagnostyce: oba obciążają łącze.";

    diag_busy_load =>
        "A speed test is running. The scan becomes available once it ends.",
        "Trwa test prędkości. Skanowanie będzie dostępne po jego zakończeniu.";

    bloat_no_answer => "no reply", "brak odpowiedzi";
    bloat_speed_ping => "Ping", "Ping";
    bloat_speed_idle => "idle line", "bezczynne łącze";
    bloat_tip_speed =>
        "The speed the test reached on four connections to Cloudflare's servers.\n\n\
         It can be lower than your plan if the server, the Wi-Fi or this computer limits it. \
         Ping is measured with the line idle; the ping under each speed is the average while \
         the line was loaded that way, and the rise over idle is what the grade is read from.",
        "Prędkość, jaką test osiągnął na czterech połączeniach z serwerami Cloudflare.\n\n\
         Może być niższa niż w pakiecie, jeśli ogranicza ją serwer, Wi-Fi albo ten komputer. \
         Ping jest mierzony na bezczynnym łączu; ping pod każdą prędkością to średnia w trakcie \
         pobierania albo wysyłania, a z jego wzrostu wynika ocena.";

    bloat_running_title => "Test in progress", "Trwa test";
    bloat_step_idle => "Idle", "Bez obciążenia";
    bloat_step_down => "Downloading", "Pobieranie";
    bloat_step_up => "Uploading", "Wysyłanie";
    bloat_finishing => "finishing…", "kończenie…";
    bloat_monitor_paused =>
        "Background monitoring is paused for the test, so its own pings do not count as load.",
        "Monitorowanie w tle jest wstrzymane na czas testu, żeby jego pingi nie liczyły się do \
         obciążenia.";

    bloat_empty_title => "How the test goes", "Jak przebiega test";
    bloat_empty_1 =>
        "For 6 seconds it pings 1.1.1.1 with nothing else on the line.",
        "Przez 6 sekund pinguje 1.1.1.1, gdy nic innego nie obciąża łącza.";
    bloat_empty_2 =>
        "For about 14 seconds it downloads from Cloudflare as fast as the line allows, measuring \
         the speed and the ping.",
        "Przez około 14 sekund pobiera dane z Cloudflare z pełną prędkością łącza i mierzy \
         prędkość oraz ping.";
    bloat_empty_3 =>
        "For about 14 more it uploads the same way.",
        "Przez kolejne około 14 sekund tak samo wysyła dane.";
    bloat_empty_grade =>
        "The grade is how far the ping rises under load, in the worse of the two directions:",
        "Ocena zależy od tego, o ile rośnie ping pod obciążeniem, w gorszym z dwóch kierunków:";

    bloat_chart_title => "Course of the test", "Przebieg testu";
    bloat_chart_caption =>
        "The ping to 1.1.1.1 at every moment of the test, in ms",
        "Ping do 1.1.1.1 w każdej chwili testu, w ms";
    bloat_chart_lost => "no reply", "bez odpowiedzi";
    bloat_chart_axis => "seconds since the test started", "sekundy od startu testu";

    bloat_prog_idle => "Measuring idle latency…", "Pomiar opóźnienia bezczynnego…";
    bloat_prog_load =>
        "Saturating the link and measuring latency under load…",
        "Wysycanie łącza i pomiar opóźnienia pod obciążeniem…";
    bloat_prog_upload =>
        "Saturating the upload and measuring latency…",
        "Wysycanie wysyłania i pomiar opóźnienia…";
    bloat_prog_done => "Done", "Gotowe";

    grade_a =>
        "Excellent. The link does not bloat under load.",
        "Doskonale. Łącze nie puchnie pod obciążeniem.";
    grade_b => "Good. A mild rise, unnoticeable in games.", "Dobrze. Lekki wzrost, w grach niezauważalny.";
    grade_c =>
        "Fair. Latency climbs noticeably while the line is busy.",
        "Średnio. Opóźnienie wyraźnie rośnie, gdy łącze jest zajęte.";
    grade_d =>
        "Poor. Games will stutter during any download or upload.",
        "Słabo. Gry będą się ciąć przy każdym pobieraniu lub wysyłaniu.";
    grade_f => "Very poor. Textbook bufferbloat.", "Bardzo słabo. Podręcznikowy bufferbloat.";
    grade_unknown => "Not measured.", "Nie zmierzono.";

    bloat_silent_under_load =>
        "Under load the host stopped replying entirely. That is itself the result.",
        "Pod obciążeniem host przestał odpowiadać całkowicie. To już jest wynik.";
    bloat_advice_title => "What to do about it", "Co z tym zrobić";
    bloat_advice_run => "Run the test to get a result.", "Uruchom test, żeby zobaczyć wynik.";
    bloat_advice_ok =>
        "Nothing to do: the link copes with load. If you still get lag, the cause is elsewhere \
         (Wi-Fi, driver, or the route to that particular server).",
        "Nie ma co poprawiać: łącze radzi sobie z obciążeniem. Jeśli mimo to masz lagi, \
         przyczyna leży gdzie indziej (Wi-Fi, sterownik albo trasa do konkretnego serwera).";
    bloat_advice_header => "What helps, most effective first:", "Co pomaga, od najskuteczniejszego:";
    bloat_advice_1 =>
        "Enable SQM / Smart Queue / QoS on the router (look for \"cake\" or \"fq_codel\") and \
         cap it at about 90% of the real line speed.",
        "Włącz SQM / Smart Queue / QoS na routerze (szukaj „cake” albo „fq_codel”) i ustaw \
         limit na około 90% rzeczywistej prędkości łącza.";
    bloat_advice_2 =>
        "If the router has no such option, that is the best possible reason to replace it. No \
         Windows setting can fix this.",
        "Jeśli router nie ma takiej opcji, to najlepszy możliwy powód, żeby go wymienić. Żadne \
         ustawienie Windowsa tego nie naprawi.";
    bloat_advice_3 =>
        "As a stopgap, throttle whatever saturates the link (Steam, torrents, updates) to \
         about 80% of capacity.",
        "Doraźnie ogranicz to, co wysyca łącze (Steam, torrenty, aktualizacje) do około 80% \
         przepustowości.";

    // -----------------------------------------------------------------------
    // exported report
    //
    // Section headings and row labels follow the UI language. Row labels are
    // padded to a fixed width by the caller, so they stay aligned whatever
    // their length.
    // -----------------------------------------------------------------------
    rep_file_stem =>
        "NetDoctor report",
        "NetDoctor raport";
    set_sec_reports =>
        "Reports",
        "Raporty";
    set_report_dir_hint =>
        "Reports are saved as PDF files with the date in the name, in this folder:",
        "Raporty zapisują się jako pliki PDF z datą w nazwie, w tym folderze:";
    set_btn_change_dir =>
        "Change…",
        "Zmień…";
    set_btn_open_dir =>
        "Open folder",
        "Otwórz folder";
    set_btn_default_dir =>
        "Use the default",
        "Przywróć domyślny";
    rep_title => "NetDoctor report", "Raport NetDoctor";
    rep_sec_connection => "CONNECTION", "POŁĄCZENIE";
    rep_sec_measurements => "MEASUREMENTS (last hour)", "POMIARY (ostatnia godzina)";
    rep_log_heading => "Windows log around it:", "Dziennik Windows wokół niej:";
    rep_log_quiet =>
        "Windows log around it: nothing relevant logged.",
        "Dziennik Windows wokół niej: nic istotnego nie zapisano.";
    rep_log_not_read =>
        "Windows log: not read (only the 30 most recent outages are).",
        "Dziennik Windows: nie odczytano (tylko dla 30 najnowszych awarii).";
    rep_path_then => "Path when it began:", "Ścieżka w chwili rozpoczęcia:";
    rep_path_unrecorded =>
        "Path when it began: not recorded (older version of the app, or not walked yet).",
        "Ścieżka w chwili rozpoczęcia: niezapisana (starsza wersja aplikacji albo trasa jeszcze nie zbadana).";
    rep_ongoing => "still going", "trwa";
    hist_report_range => "Report covers", "Raport obejmuje";
    hist_report_saving =>
        "Saving the report…",
        "Zapisywanie raportu…";
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
    tw_before_unknown =>
        "The current value could not be read, so Revert would have nothing to go back to. Nothing was changed.",
        "Nie udało się odczytać obecnej wartości, więc Cofnij nie miałoby do czego wrócić. Niczego nie zmieniono.";
    tw_state_unreadable => "cannot read", "nie można odczytać";
    tw_snapshots_unreadable_hint =>
        "The saved-state file is damaged, so Revert is unavailable for every change. The file was not overwritten, so it can still be repaired by hand.",
        "Plik zapisanych stanów jest uszkodzony, więc Cofnij jest niedostępne dla wszystkich zmian. Plik nie został nadpisany, więc nadal da się go naprawić ręcznie.";
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
        "The most common cause of drops on laptops. Windows switches the card off when idle, and \
         waking it takes long enough for connections to break.",
        "Najczęstsza przyczyna zrywania połączeń na laptopach. Windows usypia kartę w \
         bezczynności, a wybudzenie trwa na tyle długo, że połączenia się zrywają.";
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
        "An independent power setting. Even with the driver's power management off, the power \
         plan can lower transmit power and cause drops.",
        "Niezależne ustawienie zasilania. Nawet przy wyłączonym zarządzaniu energią w sterowniku \
         plan zasilania może obniżać moc nadawania i powodować zrywanie połączenia.";
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
        "When the router is the only DNS server, its failure looks like no internet: ping by IP \
         works, but pages do not load. Downside: lookups go to Cloudflare and Google instead of \
         your ISP, and on a company network or a VPN internal names stop resolving. Affects IPv4 \
         only.",
        "Gdy router jest jedynym serwerem DNS, jego awaria wygląda jak brak internetu: ping po IP \
         działa, ale strony się nie ładują. Minus: zapytania trafiają do Cloudflare i Google \
         zamiast do dostawcy, a w sieci firmowej lub przez VPN przestają działać nazwy \
         wewnętrzne. Dotyczy tylko IPv4.";
    tw_dns_none => "none / from DHCP", "brak / z DHCP";
    tw_dns_own_note =>
        "  (your own DNS server: left as it is)",
        "  (Twój własny serwer DNS: zostaje bez zmian)";
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
        "Some \"ping boost\" guides recommend disabling it, which limits throughput on fast \
         connections. Normal is the Windows default.",
        "Niektóre poradniki „na lepszy ping” zalecają wyłączenie tej funkcji, co ogranicza \
         przepustowość na szybkich łączach. Normal to domyślna wartość Windows.";
    tw_autotune_applied => "Auto-tuning set to normal.", "Auto-tuning ustawiony na normal.";

    // Nagle
    tw_nagle_title => "Disable Nagle's algorithm (for games)", "Wyłącz algorytm Nagle'a (pod gry)";
    tw_nagle_what =>
        "Writes TcpAckFrequency=1 and TCPNoDelay=1 for the active interface.",
        "Zapisuje TcpAckFrequency=1 i TCPNoDelay=1 dla aktywnego interfejsu.";
    tw_nagle_why =>
        "Windows groups small TCP packets and delays acknowledgements, which can add a few ms in \
         games that use TCP. No effect on games that use UDP (League of Legends, Counter-Strike \
         2, VALORANT) or on downloads.",
        "Windows grupuje małe pakiety TCP i opóźnia potwierdzenia, co może dodać kilka ms w grach \
         korzystających z TCP. Nie wpływa na gry używające UDP (League of Legends, Counter-Strike \
         2, VALORANT) ani na pobieranie.";
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
        "While media is playing, Windows limits network traffic to about 10,000 packets/s. Gaming \
         with music or a stream on can lag as a result.",
        "Podczas odtwarzania multimediów Windows ogranicza ruch sieciowy do ok. 10 tys. \
         pakietów/s. Granie z włączoną muzyką lub streamem może przez to lagować.";
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
        "Too large an MTU makes packets get dropped along the way. Symptom: pages that never \
         finish loading while ping works.",
        "Zbyt duże MTU powoduje odrzucanie pakietów po drodze. Objaw: strony nie doładowują się \
         do końca, choć ping działa.";
    tw_mtu_unknown => "MTU unknown", "MTU nieznane";
    tw_mtu_probe_failed =>
        "MTU probe produced no result (is DF-flagged ICMP blocked?)",
        "Sonda MTU nic nie zwróciła (czy ICMP z flagą DF jest blokowane?)";

    // stack reset
    tw_reset_title => "Reset the network stack (repair action)", "Zresetuj stos sieciowy (akcja naprawcza)";
    tw_reset_why =>
        "For a connection that stopped working and does not come back. Clears damaged Winsock and \
         IP settings that would otherwise last until a restart.",
        "Dla połączenia, które przestało działać i nie wraca. Czyści uszkodzone ustawienia \
         Winsock i IP, które inaczej zostają do restartu.";
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
        "Each change can be reverted on its own, also after a restart.",
        "Każdą zmianę można cofnąć osobno, także po restarcie komputera.";
    opt_btn_apply_all => "Apply all safe changes", "Zastosuj wszystkie bezpieczne zmiany";
    opt_needs_admin => "Requires administrator rights", "Wymaga uprawnień administratora";
    opt_read_only =>
        "read-only (restart as administrator to apply)",
        "tylko do odczytu (uruchom ponownie jako administrator, żeby zastosować)";
    opt_nothing_to_revert => "Nothing saved to revert to", "Nie zapisano nic, do czego można wrócić";
    opt_needs_reboot => "needs a restart", "wymaga restartu";
    opt_irreversible => "cannot be undone", "nie da się cofnąć";
    opt_reading =>
        "reading the current state…",
        "odczyt bieżącego stanu…";
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
        "Laptops ship with reduced power to save battery. The router then hears the laptop worse \
         than the other way round, so uploads fail first at the edge of the range.",
        "Laptopy mają fabrycznie obniżoną moc, żeby oszczędzać baterię. Router słyszy wtedy \
         laptopa gorzej niż laptop router, więc na skraju zasięgu jako pierwsze zawodzi \
         wysyłanie.";

    tw_roam_title =>
        "Let the card switch to a stronger access point sooner",
        "Pozwól karcie szybciej przełączać się na mocniejszy access point";
    tw_roam_what =>
        "Sets roaming aggressiveness to the highest level the driver offers.",
        "Ustawia agresywność roamingu na najwyższy poziom oferowany przez sterownik.";
    tw_roam_why =>
        "Helps with a mesh or several access points: the card moves to a stronger one before the \
         signal fades. No effect with a single access point. Among many nearby networks the card \
         can switch too often; revert it if the connection starts stuttering.",
        "Pomaga przy sieci mesh lub kilku access pointach: karta przechodzi na mocniejszy, zanim \
         sygnał osłabnie. Przy jednym access poincie nie ma wpływu. Przy wielu sieciach w pobliżu \
         karta może przełączać się zbyt często; wtedy cofnij zmianę.";

    tw_psm_title =>
        "Stop the Wi-Fi radio going to sleep between packets",
        "Nie pozwól radiu Wi-Fi zasypiać między pakietami";
    tw_psm_what =>
        "Sets the adapter's power save mode to maximum performance.",
        "Ustawia tryb oszczędzania energii karty na maksymalną wydajność.";
    tw_psm_why =>
        "In power save the radio sleeps between beacons, so the first packet after a pause waits \
         tens of ms for it to wake. This is a driver setting, separate from the Windows device \
         option.",
        "W trybie oszczędzania radio śpi między beaconami, więc pierwszy pakiet po przerwie czeka \
         kilkadziesiąt ms na wybudzenie. To ustawienie sterownika, niezależne od opcji urządzenia \
         w Windows.";

    tw_mimo_title =>
        "Keep every antenna listening (no MIMO power save)",
        "Trzymaj wszystkie anteny włączone (bez MIMO power save)";
    tw_mimo_what =>
        "Disables spatial multiplexing power save, so the card does not shut down its extra \
         receive chains while idle.",
        "Wyłącza spatial multiplexing power save, żeby karta nie wyłączała dodatkowych torów \
         odbiorczych w bezczynności.";
    tw_mimo_why =>
        "With one antenna switched off, the card receives a weak signal about 3 dB worse, which \
         at the edge of the range is often enough to break the connection.",
        "Z jedną wyłączoną anteną karta odbiera słaby sygnał o ok. 3 dB gorzej, co na skraju \
         zasięgu często wystarcza do zerwania połączenia.";

    tw_w24_title =>
        "Use 20 MHz channels on 2.4 GHz",
        "Używaj kanałów 20 MHz na 2,4 GHz";
    tw_w24_what =>
        "Stops the card bonding two 2.4 GHz channels into one 40 MHz channel.",
        "Przestaje łączyć dwa kanały 2,4 GHz w jeden 40 MHz.";
    tw_w24_why =>
        "2.4 GHz has only three non-overlapping channels, and a 40 MHz link takes two of them. \
         Among neighbouring networks that means collisions and retransmissions; 20 MHz is usually \
         faster in practice and has better range.",
        "Na 2,4 GHz są tylko trzy nienachodzące na siebie kanały, a łącze 40 MHz zajmuje dwa z \
         nich. Przy sieciach sąsiadów oznacza to kolizje i retransmisje; 20 MHz jest w praktyce \
         zwykle szybsze i ma lepszy zasięg.";

    tw_band_title =>
        "Prefer 5 GHz when the signal allows",
        "Preferuj 5 GHz, gdy sygnał pozwala";
    tw_band_what =>
        "Tells the card to pick the 5 GHz radio of a network that broadcasts on both bands.",
        "Każe karcie wybierać radio 5 GHz w sieci nadającej na obu pasmach.";
    tw_band_why =>
        "5 GHz is less crowded and faster, but passes through walls much worse. Best in the same \
         room as the router; with a weak signal leave this off.",
        "5 GHz jest mniej zatłoczone i szybsze, ale znacznie gorzej przechodzi przez ściany. \
         Najlepsze w tym samym pomieszczeniu co router; przy słabym sygnale lepiej tego nie \
         włączać.";

    tw_intmod_title =>
        "Turn off interrupt moderation on the wired card",
        "Wyłącz interrupt moderation na karcie przewodowej";
    tw_intmod_what =>
        "Makes the adapter raise an interrupt per packet instead of batching them.",
        "Karta zgłasza przerwanie na każdy pakiet, zamiast zbierać je w paczki.";
    tw_intmod_why =>
        "Batching saves CPU but holds packets for a fraction of a millisecond. Not noticeable in \
         downloads, noticeable in games and calls. Costs a few percent of one CPU core.",
        "Grupowanie oszczędza procesor, ale przetrzymuje pakiety przez ułamek milisekundy. \
         Niezauważalne przy pobieraniu, odczuwalne w grach i rozmowach. Kosztuje kilka procent \
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
        "Both features renegotiate the link, which is a brief disconnect. With a worn cable or a \
         cheap switch this causes drops that look like an internet outage.",
        "Obie funkcje renegocjują łącze, co oznacza krótkie rozłączenie. Przy zużytym kablu lub \
         tanim switchu powoduje to zaniki wyglądające jak awaria internetu.";

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
        "CUBIC treats every lost packet as congestion and halves its speed, which on Wi-Fi wastes \
         throughput. BBR2 adjusts to the measured bandwidth and latency instead. Downside: game \
         launchers such as Battle.net or the Riot client can hang while loading, because they \
         open many short connections. Compare the speed test before and after, and revert if it \
         does not help or something stops loading.",
        "CUBIC traktuje każdy zgubiony pakiet jak przeciążenie i zmniejsza prędkość o połowę, co \
         na Wi-Fi marnuje przepustowość. BBR2 dostosowuje się do zmierzonej przepustowości i \
         opóźnienia. Minus: launchery gier, np. Battle.net czy klient Riot, mogą zawieszać się na \
         ładowaniu, bo otwierają wiele krótkich połączeń. Porównaj test prędkości przed i po \
         zmianie i cofnij ją, jeśli nie pomaga albo coś przestało się ładować.";
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
        "By default Windows remembers a failed lookup for 5 minutes. After an outage the \
         connection is back, but sites still do not open until that time passes.",
        "Domyślnie Windows pamięta nieudane zapytanie przez 5 minut. Po awarii łącze już działa, \
         ale strony nie otwierają się, dopóki ten czas nie minie.";

    tw_do_title =>
        "Stop Windows Update uploading to other machines",
        "Nie pozwól Windows Update wysyłać aktualizacji innym komputerom";
    tw_do_what =>
        "Sets Delivery Optimization to download from Microsoft only, with no peer-to-peer \
         sharing.",
        "Ustawia Delivery Optimization na pobieranie wyłącznie od Microsoftu, bez wymiany \
         peer-to-peer.";
    tw_do_why =>
        "Delivery Optimization sends updates to other computers over your upload. A full upload \
         raises latency even though downloads look normal.",
        "Delivery Optimization wysyła aktualizacje innym komputerom przez Twoje łącze. Zapchane \
         wysyłanie podnosi opóźnienia, nawet gdy pobieranie wygląda normalnie.";

    tw_hotspot_title =>
        "Stop connecting automatically to open hotspots",
        "Nie łącz się automatycznie z otwartymi hotspotami";
    tw_hotspot_what =>
        "Turns off the setting that lets Windows join suggested open networks on its own.",
        "Wyłącza ustawienie pozwalające Windows samodzielnie dołączać do proponowanych \
         otwartych sieci.";
    tw_hotspot_why =>
        "Windows may leave a working network for a recognised operator hotspot that requires \
         signing in. The result is an outage with no visible cause.",
        "Windows może porzucić działającą sieć na rzecz rozpoznanego hotspotu operatora, który \
         wymaga logowania. Skutkiem jest przerwa bez widocznej przyczyny.";

    tw_ipv4_title =>
        "Prefer IPv4 over IPv6",
        "Preferuj IPv4 zamiast IPv6";
    tw_ipv4_what =>
        "Reorders the address preference table so IPv4 is tried first. IPv6 stays enabled.",
        "Zmienia kolejność w tablicy preferencji adresów, żeby IPv4 był próbowany pierwszy. \
         IPv6 zostaje włączone.";
    tw_ipv4_why =>
        "If the ISP provides IPv6 that does not work, every connection first waits for an IPv6 \
         timeout and pages start seconds late. On a network that relies on IPv6 this makes things \
         worse; revert it then.",
        "Jeśli operator udostępnia niedziałające IPv6, każde połączenie najpierw czeka na timeout \
         IPv6 i strony ruszają z kilkusekundowym opóźnieniem. W sieci opartej na IPv6 zmiana \
         pogarsza działanie; wtedy ją cofnij.";

    // -----------------------------------------------------------------------
    // the air scan
    // -----------------------------------------------------------------------
    air_no_service =>
        "the Windows WLAN service is not running",
        "usługa WLAN systemu Windows nie działa";
    air_no_adapter =>
        "no Wi-Fi adapter to scan with",
        "brak karty Wi-Fi, którą można skanować";
    air_title => "Wi-Fi channel on your router", "Kanał Wi-Fi w routerze";
    air_blurb =>
        "Shows which channels nearby networks use and which channel to set on the router. The \
         channel itself is changed in the router's settings.",
        "Pokazuje, na których kanałach nadają sieci w pobliżu i który kanał ustawić w routerze. \
         Sam kanał zmienia się w ustawieniach routera.";
    air_btn_scan => "Check the channels", "Sprawdź kanały";
    air_btn_rescan => "Check again", "Sprawdź ponownie";
    air_scan_cost =>
        "takes about 4 s; Wi-Fi disconnects briefly during the scan",
        "trwa ok. 4 s; Wi-Fi na chwilę się rozłącza podczas skanowania";
    air_scanning =>
        "scanning, about 4 s…",
        "skanowanie, ok. 4 s…";
    air_verdict_fine_why =>
        "The current channel is no busier than the alternatives. No change needed.",
        "Obecny kanał nie jest bardziej zatłoczony niż pozostałe. Zmiana nie jest potrzebna.";
    air_verdict_unknown =>
        "Scan finished, but the current channel could not be read.",
        "Skanowanie zakończone, ale nie udało się odczytać bieżącego kanału.";
    air_bars_heading =>
        "How crowded the 2.4 GHz channels are (longer is busier)",
        "Jak zatłoczone są kanały 2,4 GHz (dłuższy pasek to większy tłok)";
    air_bars_heading_5 =>
        "How crowded your 5 GHz channel is next to the radar-free ones (longer is busier)",
        "Jak zatłoczony jest twój kanał 5 GHz na tle kanałów bez radaru (dłuższy pasek to większy tłok)";
    air_tag_yours => "your channel", "twój kanał";
    air_tag_best => "recommended", "polecany";
    air_router_note =>
        "Set it in the router's settings, under the 2.4 GHz wireless channel. Choose a fixed \
         channel instead of Auto: Auto picks a channel only when the router starts.",
        "Kanał ustawia się w ustawieniach routera, w polu kanału sieci 2,4 GHz. Wybierz stały \
         kanał zamiast Auto: tryb Auto wybiera kanał tylko przy starcie routera.";
    air_dfs_move =>
        "Recommended: 36, 40, 44 or 48 for range, or 149 and above for less crowded air. These \
         channels need no radar detection and do not switch off on their own.",
        "Zalecane: 36, 40, 44 lub 48 dla zasięgu albo 149 i wyżej dla mniejszego tłoku. Te kanały \
         nie wymagają wykrywania radaru i nie wyłączają się same.";
    // -----------------------------------------------------------------------
    // the optimise list: sections and status
    // -----------------------------------------------------------------------
    cat_power => "Power and sleep", "Zasilanie i uśpienie";
    cat_power_blurb =>
        "Power saving that switches the network card off. The most common cause of drops with no \
         visible reason.",
        "Oszczędzanie energii, które wyłącza kartę sieciową. Najczęstsza przyczyna zrywania \
         połączenia bez widocznego powodu.";
    cat_reach => "Radio and range", "Radio i zasięg";
    cat_reach_blurb =>
        "Range of the Wi-Fi card and the choice of access point.",
        "Zasięg karty Wi-Fi i wybór access pointa.";
    cat_naming => "Names and addresses", "Nazwy i adresy";
    cat_naming_blurb =>
        "DNS, and the choice between IPv4 and IPv6.",
        "DNS oraz wybór między IPv4 a IPv6.";
    cat_throughput => "Throughput and latency", "Przepustowość i opóźnienia";
    cat_throughput_blurb =>
        "Transfer speed and latency on a working connection.",
        "Prędkość transferu i opóźnienia na działającym łączu.";
    cat_neighbours => "What else uses the link", "Kto jeszcze zużywa łącze";
    cat_neighbours_blurb =>
        "Background services on this computer that use the connection.",
        "Usługi w tle na tym komputerze, które korzystają z łącza.";
    cat_last_resort => "Last resort", "Ostateczność";
    cat_last_resort_blurb =>
        "Repair actions for a connection that has stopped working.",
        "Akcje naprawcze dla połączenia, które przestało działać.";

    st_set => "set", "ustawione";
    st_todo => "worth changing", "do poprawy";
    st_todo_heading => "Worth changing", "Do poprawy";
    st_na => "not available", "niedostępne";
    opt_revert_available =>
        "changed by NetDoctor (can be undone)",
        "zmienione przez NetDoctor (można cofnąć)";
    opt_section_all_set => "all set", "wszystko ustawione";
    opt_show_unavailable => "show unavailable", "pokaż niedostępne";
    opt_show_unavailable_hint =>
        "Changes this machine cannot take: a Wi-Fi setting on a cable, or one the \
         driver does not expose. The section counts ignore them either way.",
        "Zmiany, których ta maszyna nie przyjmie: ustawienie Wi-Fi przy kablu albo takie, \
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

/// Refusal of a probe interval too long for an outage to ever be recorded.
pub fn set_err_interval_max(max_s: u64) -> String {
    match current() {
        Lang::En => format!(
            "An interval above {max_s} s would stop outages from being recorded at all: sweeps \
             that far apart read as a machine that slept."
        ),
        Lang::Pl => format!(
            "Odstęp powyżej {max_s} s wyłączyłby zapisywanie awarii: tak rzadkie pomiary \
             wyglądają jak uśpiony komputer."
        ),
    }
}

/// A router that ignores pings addressed to itself while traffic passes.
/// The route row when traffic goes through a tunnel.
pub fn measure_route_vpn(name: &str) -> String {
    match current() {
        Lang::En => format!("through a VPN ({name})"),
        Lang::Pl => format!("przez VPN ({name})"),
    }
}

/// The line over the verdict: when the scan ran and on which connection.
pub fn verdict_meta(when: &str, place: &str) -> String {
    match current() {
        Lang::En => format!("Scan at {when} · {place}"),
        Lang::Pl => format!("Skan z {when} · {place}"),
    }
}

/// What the scan before this one found, and how the ping moved since.
pub fn verdict_since(
    when: &str,
    segment: &str,
    router: Option<(f64, f64)>,
    internet: Option<(f64, f64)>,
) -> String {
    let mut moved = Vec::new();
    if let Some((a, b)) = router {
        moved.push(format!("{} {a:.0} → {b:.0} ms", pick("router", "router")));
    }
    if let Some((a, b)) = internet {
        moved.push(format!("{} {a:.0} → {b:.0} ms", pick("internet", "internet")));
    }
    let moved = if moved.is_empty() { String::new() } else { format!("; {}", moved.join(", ")) };
    match current() {
        Lang::En => format!("Previous scan at {when}: {}{moved}", segment.to_lowercase()),
        Lang::Pl => format!("Poprzedni skan z {when}: {}{moved}", segment.to_lowercase()),
    }
}

pub fn f_vpn_detail(name: &str) -> String {
    match current() {
        Lang::En => format!("Windows routes the internet through {name}."),
        Lang::Pl => format!("Windows kieruje ruch do internetu przez {name}."),
    }
}

pub fn f_proxy_detail(proxy: &str) -> String {
    match current() {
        Lang::En => format!("Set to {proxy}."),
        Lang::Pl => format!("Ustawiony: {proxy}."),
    }
}

pub fn f_other_medium_detail(name: &str, desc: &str) -> String {
    if desc.is_empty() || desc == name {
        return name.to_string();
    }
    format!("{name} ({desc})")
}

pub fn f_line_ok(avg: f64, kind: &str, lo: f64, hi: f64) -> String {
    match current() {
        Lang::En => {
            format!("Ping {avg:.0} ms is typical for this line: {kind} ({lo:.0}-{hi:.0} ms)")
        }
        Lang::Pl => {
            format!("Ping {avg:.0} ms jest typowy dla tego łącza: {kind} ({lo:.0}-{hi:.0} ms)")
        }
    }
}

pub fn f_line_slow(avg: f64, kind: &str, hi: f64) -> String {
    match current() {
        Lang::En => format!(
            "Ping {avg:.0} ms is higher than typical for this line: {kind} (up to {hi:.0} ms)"
        ),
        Lang::Pl => format!(
            "Ping {avg:.0} ms jest wyższy niż typowy dla tego łącza: {kind} (do {hi:.0} ms)"
        ),
    }
}

/// What to look at first on a line of this kind when the ping is high or the
/// provider's side is at fault.
pub fn line_advice(kind: crate::settings::LineKind) -> String {
    use crate::settings::LineKind as K;
    // Only what the user can do themselves, or tell the provider. An antenna
    // or a modem the provider manages is out of their reach, so advice to
    // look inside one is advice they cannot follow.
    let (en, pl) = match kind {
        K::Fibre => (
            "On fibre a high ping is usually the Wi-Fi or the router: connect this computer to the router with a cable and scan again. If it is the same on a cable, report it to the provider with a saved report.",
            "Na światłowodzie wysoki ping to zwykle Wi-Fi albo router: podłącz komputer kablem do routera i uruchom skan ponownie. Jeśli na kablu jest tak samo, zgłoś to dostawcy z zapisanym raportem.",
        ),
        K::Cable => (
            "On a cable line the ping usually rises in the evening, when neighbours share the same network segment. Scan again at another time; if mornings are fine and evenings are not, report it to the provider with a saved report.",
            "Na łączu kablowym ping rośnie zwykle wieczorem, gdy sąsiedzi korzystają z tego samego segmentu sieci. Uruchom skan o innej porze; jeśli rano jest dobrze, a wieczorem źle, zgłoś to dostawcy z zapisanym raportem.",
        ),
        K::Dsl => (
            "On DSL the ping depends on the phone line. Plug the modem straight into the main phone socket, without splitters or extension cables, and scan again; if that does not help, report the line to the provider with a saved report.",
            "Na DSL ping zależy od stanu linii telefonicznej. Podłącz modem bezpośrednio do głównego gniazdka, bez rozgałęźników i przedłużaczy, i uruchom skan ponownie; jeśli nie pomoże, zgłoś linię dostawcy z zapisanym raportem.",
        ),
        K::Radio => (
            "On a radio line the problem is usually the signal between the antenna and the operator's station: weather, leaves, a shifted antenna. Check that the cables to the antenna's power adapter are plugged in firmly; if it keeps happening, save a report and tell the operator the radio signal is unstable.",
            "Na łączu radiowym problem leży zwykle w sygnale między anteną a stacją operatora: pogoda, liście, przesunięta antena. Sprawdź, czy kable do zasilacza anteny są dobrze wpięte; jeśli problem się powtarza, zapisz raport i zgłoś operatorowi niestabilny sygnał radiowy.",
        ),
        K::Mobile => (
            "On LTE / 5G moving the modem helps most: by a window, higher up, away from thick walls. Scan again after moving it; the cell is often busy in the evening, so compare with another time too.",
            "Na LTE / 5G najwięcej daje przestawienie modemu: przy oknie, wyżej, z dala od grubych ścian. Uruchom skan po przestawieniu; wieczorem komórka bywa przeciążona, więc porównaj też z inną porą.",
        ),
        K::SatelliteLeo => (
            "On satellite the dish needs a clear view of the sky: trees and buildings cause short drops. The operator's own app shows what blocks it.",
            "Na łączu satelitarnym antena potrzebuje czystego nieba: drzewa i budynki powodują krótkie przerwy. Aplikacja operatora pokazuje, co ją zasłania.",
        ),
        K::SatelliteGeo => (
            "On a geostationary link the high ping comes from the distance to the satellite and cannot be lowered; fast-paced games will not play well on it.",
            "Na łączu geostacjonarnym wysoki ping wynika z odległości do satelity i nie da się go obniżyć; szybkie gry nie będą na nim działać dobrze.",
        ),
        K::Unknown => ("", ""),
    };
    pick(en, pl)
}

/// Under a good grade, when the plan is known and the test fell short of it.
pub fn bloat_plan_caveat(pct: f64, plan: f64) -> String {
    match current() {
        Lang::En => format!(
            "The test reached {pct:.0}% of your plan's {plan:.0} Mbps, so the line may not have been full and the grade may be better than the line."
        ),
        Lang::Pl => format!(
            "Test osiągnął {pct:.0}% z {plan:.0} Mbps z umowy, więc łącze mogło nie być zapchane i ocena może być lepsza niż łącze."
        ),
    }
}

/// Under a speed: how much of the plan it is.
pub fn bloat_speed_plan(pct: f64, plan: f64) -> String {
    match current() {
        Lang::En => format!("{pct:.0}% of the plan's {plan:.0} Mbps"),
        Lang::Pl => format!("{pct:.0}% z {plan:.0} Mbps z umowy"),
    }
}

pub fn rep_plan_line(down: Option<f64>, up: Option<f64>) -> String {
    let v = |x: Option<f64>| x.map_or_else(|| "?".to_string(), |x| format!("{x:.0}"));
    format!("{} / {} Mbps", v(down), v(up))
}

pub fn f_router_mute_detail(gw: &str) -> String {
    match current() {
        Lang::En => format!(
            "{gw} did not answer pings, but the internet did. Many routers ignore pings \
             addressed to themselves; that is a setting, not a fault. The latency of this \
             stretch cannot be measured separately."
        ),
        Lang::Pl => format!(
            "{gw} nie odpowiedział na pingi, ale internet tak. Wiele routerów ignoruje pingi \
             skierowane do siebie; to ustawienie, a nie usterka. Opóźnienia tego odcinka nie da \
             się zmierzyć osobno."
        ),
    }
}

/// Neither anchor answered pings, but a connection to one of them worked.
pub fn f_icmp_filtered_detail(hosts: &str) -> String {
    match current() {
        Lang::En => format!(
            "No ping to {hosts} came back, but a connection to port 443 went through. The \
             internet works; something on the way filters pings, so loss and latency could not \
             be measured."
        ),
        Lang::Pl => format!(
            "Żaden ping do {hosts} nie wrócił, ale połączenie na port 443 przeszło. Internet \
             działa; coś po drodze filtruje pingi, więc strat i opóźnienia nie dało się zmierzyć."
        ),
    }
}

/// A length of time the way a person says it: "11 s", "5 min", "3 h 20 min".
/// Under a minute it stays in seconds, so a short outage is never "0 min".
pub fn span(secs: f64) -> String {
    let secs = secs.max(0.0);
    if secs.round() < 60.0 {
        return format!("{:.0} s", secs);
    }
    let mins = (secs / 60.0).round() as u64;
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// No outages in the last day, but the day was only partly watched.
pub fn f_hist_none_partial(watched: &str) -> String {
    match current() {
        Lang::En => format!("No outages in the {watched} watched of the last 24 hours"),
        Lang::Pl => format!("Brak awarii w {watched} obserwacji z ostatnich 24 godzin"),
    }
}

// --- the outage section of the report --------------------------------------

pub fn rep_range(days: Option<u32>) -> String {
    match (current(), days) {
        (Lang::En, Some(1)) => "last 24 hours".into(),
        (Lang::Pl, Some(1)) => "ostatnie 24 godziny".into(),
        (Lang::En, Some(d)) => format!("last {d} days"),
        (Lang::Pl, Some(d)) => format!("ostatnie {d} dni"),
        (Lang::En, None) => "everything on record".into(),
        (Lang::Pl, None) => "wszystko, co zapisane".into(),
    }
}

/// The heading of the outage section, with the span it covers.
pub fn rep_outages_heading(range: &str, from: &str, to: &str) -> String {
    match current() {
        Lang::En => format!("OUTAGES, {range} ({from} to {to})"),
        Lang::Pl => format!("AWARIE, {range} (od {from} do {to})"),
    }
}

/// Samples are kept for fewer days than the report reaches back.
pub fn rep_samples_kept(days: i64) -> String {
    match current() {
        Lang::En => format!(
            "  Measurements are kept for {days} days, so watching before that cannot be shown; \
             outages are kept longer."
        ),
        Lang::Pl => format!(
            "  Pomiary są przechowywane {days} dni, więc obserwacji sprzed tego nie da się \
             pokazać; awarie są przechowywane dłużej."
        ),
    }
}

/// Count and total length of one kind of outage. `slow` for a line that was
/// slow rather than down, which is not downtime.
pub fn rep_total(kind: &str, count: usize, length: &str, slow: bool) -> String {
    match (current(), slow) {
        (Lang::En, false) => format!("  {kind}: {count} time(s), {length} down in total"),
        (Lang::En, true) => format!("  {kind}: {count} time(s), {length} of poor quality in total"),
        (Lang::Pl, false) => format!("  {kind}: {count} raz(y), łącznie {length} przestoju"),
        (Lang::Pl, true) => format!("  {kind}: {count} raz(y), łącznie {length} słabej jakości"),
    }
}

pub fn rep_unwatched(secs: f64) -> String {
    match current() {
        Lang::En => format!("{secs:.0} s of it were not watched (the monitor was paused)"),
        Lang::Pl => format!("{secs:.0} s z tego nie było obserwowane (monitor był wstrzymany)"),
    }
}

pub fn rep_cause(title: &str, confidence: &str, evidence: &str) -> String {
    match current() {
        Lang::En => format!("Cause: {title} ({confidence}): {evidence}"),
        Lang::Pl => format!("Przyczyna: {title} ({confidence}): {evidence}"),
    }
}

pub fn rep_also(title: &str, confidence: &str, evidence: &str) -> String {
    match current() {
        Lang::En => format!("Also:  {title} ({confidence}): {evidence}"),
        Lang::Pl => format!("Także: {title} ({confidence}): {evidence}"),
    }
}

/// The minute of sweeps before an outage, reduced to a line.
pub fn rep_lead_minute(router: &str, internet: &str, answered: usize, total: usize) -> String {
    match current() {
        Lang::En => format!(
            "The minute before: router {router} ms, internet {internet} ms, \
             {answered} of {total} sweeps reached the internet"
        ),
        Lang::Pl => format!(
            "Minuta przed: router {router} ms, internet {internet} ms, \
             {answered} z {total} pomiarów dotarło do internetu"
        ),
    }
}

pub fn rep_lead_signal(from: i32, to: i32) -> String {
    match current() {
        Lang::En => format!(", signal {from} to {to} dBm"),
        Lang::Pl => format!(", sygnał od {from} do {to} dBm"),
    }
}

/// How much of a report's window was actually watched.
pub fn rep_watched(watched: &str, window: &str) -> String {
    match current() {
        Lang::En => {
            format!("  Watched {watched} of the last {window}; outside that nothing is known.")
        }
        Lang::Pl => {
            format!(
                "  Obserwowano {watched} z ostatnich {window}; poza tym czasem nic nie wiadomo."
            )
        }
    }
}

/// Under the jitter card: the window the verdict judged it over.
pub fn live_card_jitter_window(window: &str) -> String {
    match current() {
        Lang::En => format!("swing, last {window}"),
        Lang::Pl => format!("wahania, ostatnie {window}"),
    }
}

/// Under the loss card: the window the verdict judged it over.
pub fn live_card_loss_window(window: &str) -> String {
    match current() {
        Lang::En => format!("last {window}"),
        Lang::Pl => format!("ostatnie {window}"),
    }
}

/// A resolver that did not answer a direct query in time.
pub fn dns_server_silent(server: &str) -> String {
    match current() {
        Lang::En => format!("{server} did not answer"),
        Lang::Pl => format!("{server} nie odpowiedział"),
    }
}

/// A resolver that answered a direct query with an error code.
pub fn dns_server_error(server: &str, code: &str) -> String {
    match current() {
        Lang::En => format!("{server} answered with {code}"),
        Lang::Pl => format!("{server} odpowiedział błędem {code}"),
    }
}

/// The tray's tooltip when the newest reading is `age` seconds old.
pub fn tray_stale(age: f64) -> String {
    match current() {
        Lang::En => format!(
            "NetDoctor: no fresh reading\nThe last one is {age:.0} s old. Measuring has stopped."
        ),
        Lang::Pl => format!(
            "NetDoctor: brak świeżego pomiaru\nOstatni ma {age:.0} s. Pomiar przestał działać."
        ),
    }
}

/// Why a sweep measured nothing: no ICMP handle could be opened.
pub fn mon_unrecorded_detail(err: &str) -> String {
    match current() {
        Lang::En => format!(
            "The readings could not be saved ({err}). Loss and jitter are read back from them, \
             so the line is not judged healthy, and no outage is recorded until saving works."
        ),
        Lang::Pl => format!(
            "Nie udało się zapisać pomiarów ({err}). Straty i jitter są liczone z zapisanych \
             pomiarów, więc łącze nie jest oceniane jako zdrowe, a awarie nie są zapisywane, \
             dopóki zapis nie zacznie działać."
        ),
    }
}

pub fn mon_blind_detail(err: &str) -> String {
    match current() {
        Lang::En => format!(
            "No ICMP handle could be opened ({err}). Nothing was sent, so there is no verdict \
             about the connection, and no outage is recorded."
        ),
        Lang::Pl => format!(
            "Nie udało się otworzyć uchwytu ICMP ({err}). Nic nie zostało wysłane, więc nie ma \
             werdyktu o połączeniu i żadna awaria nie jest zapisywana."
        ),
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

/// A measured number, or a dash where nothing was measured, right-aligned in
/// `width`. A missing reading printed as 0 reads as the best value there is.
pub fn figure_or_dash(v: Option<f64>, width: usize, precision: usize) -> String {
    match v {
        Some(v) => format!("{v:>width$.precision$}"),
        None => format!("{:>width$}", "-"),
    }
}

pub fn rep_rates_line(rx: Option<u32>, tx: Option<u32>) -> String {
    let (rx, tx) = (
        rx.map_or_else(|| "?".into(), |v| v.to_string()),
        tx.map_or_else(|| "?".into(), |v| v.to_string()),
    );
    match current() {
        Lang::En => format!("{rx} Mbps receive / {tx} Mbps transmit"),
        Lang::Pl => format!("odbiór {rx} Mbps / nadawanie {tx} Mbps"),
    }
}

pub fn rep_stats_line(
    label: &str,
    count: usize,
    loss: f64,
    avg: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    jitter: Option<f64>,
) -> String {
    let (avg, min, max, jitter) = (
        figure_or_dash(avg, 7, 2),
        figure_or_dash(min, 6, 2),
        figure_or_dash(max, 7, 2),
        figure_or_dash(jitter, 6, 2),
    );
    match current() {
        Lang::En => format!(
            "  {label:<14} samples {count:>5}, loss {loss:>5.1}%, avg {avg} ms, \
             min {min}, max {max}, jitter {jitter}"
        ),
        Lang::Pl => format!(
            "  {label:<14} próbek {count:>5}, straty {loss:>5.1}%, śr. {avg} ms, \
             min {min}, maks {max}, jitter {jitter}"
        ),
    }
}

pub fn rep_loaded_line(avg: f64, max: Option<f64>, loss: f64) -> String {
    let max = figure_or_dash(max, 0, 0);
    match current() {
        Lang::En => format!("{avg:.1} ms (max {max}, loss {loss:.0}%)"),
        Lang::Pl => format!("{avg:.1} ms (maks {max}, straty {loss:.0}%)"),
    }
}

// --- tweaks ----------------------------------------------------------------

/// The snapshot file exists but could not be read or parsed. Never silently
/// treated as "no snapshots": that would hide every recorded "before" value.
/// Warning before the load test: it pulls real data, which matters on a
/// metered or mobile connection.
pub fn bloat_cost_warning() -> &'static str {
    match current() {
        Lang::En => "The test downloads, then uploads, as fast as the line allows. On a fast connection that is well over a gigabyte, so avoid it on a metered or mobile link.",
        Lang::Pl => "Test pobiera, a potem wysyła dane z pełną prędkością łącza. Na szybkim łączu to grubo ponad gigabajt, więc nie uruchamiaj go na połączeniu taryfowym ani na telefonie.",
    }
}

/// How much the test actually cost, shown next to the result.
/// The line under a speed: the ping while the line was loaded that way, and
/// how far it rose over idle.
pub fn bloat_speed_sub(ping: f64, rise: f64) -> String {
    // A ping lower under load than idle is noise, not a negative rise.
    let rise = rise.max(0.0);
    match current() {
        Lang::En => format!("ping {ping:.0} ms, +{rise:.0} ms rise"),
        Lang::Pl => format!("ping {ping:.0} ms, wzrost +{rise:.0} ms"),
    }
}

/// The line under the verdict: where the ping went, and in which direction
/// it went furthest.
pub fn bloat_summary(idle: f64, loaded: f64, upload: bool) -> String {
    match (current(), upload) {
        (Lang::En, false) => format!(
            "The ping goes from {idle:.0} ms to {loaded:.0} ms when the line is busy, most while downloading."
        ),
        (Lang::En, true) => format!(
            "The ping goes from {idle:.0} ms to {loaded:.0} ms when the line is busy, most while uploading."
        ),
        (Lang::Pl, false) => format!(
            "Ping rośnie z {idle:.0} ms do {loaded:.0} ms, gdy łącze jest zajęte, najbardziej przy pobieraniu."
        ),
        (Lang::Pl, true) => format!(
            "Ping rośnie z {idle:.0} ms do {loaded:.0} ms, gdy łącze jest zajęte, najbardziej przy wysyłaniu."
        ),
    }
}

/// How long the running test has left.
pub fn bloat_remaining(secs: f64) -> String {
    match current() {
        Lang::En => format!("about {secs:.0} s left"),
        Lang::Pl => format!("zostało ok. {secs:.0} s"),
    }
}

/// When the result on screen was measured.
pub fn bloat_measured_at(clock: &str) -> String {
    match current() {
        Lang::En => format!("Measured at {clock}"),
        Lang::Pl => format!("Zmierzono o {clock}"),
    }
}

pub fn bloat_data_used(mib: f64) -> String {
    match current() {
        Lang::En => format!("Data used by this test: {mib:.0} MB"),
        Lang::Pl => format!("Dane zużyte przez ten test: {mib:.0} MB"),
    }
}

/// Every upload stream died: the download half still stands on its own.
pub fn bloat_no_upload() -> &'static str {
    match current() {
        Lang::En => "The upload could not be loaded: every stream failed. Only the download is graded.",
        Lang::Pl => "Nie udało się obciążyć wysyłania: wszystkie strumienie padły. Oceniane jest tylko pobieranie.",
    }
}

/// Under a good grade: the load it was measured at, which only the user can
/// hold against the speed they pay for.
pub fn bloat_saturation_caveat(down: f64, up: Option<f64>) -> String {
    let up = up.map_or_else(|| "?".to_string(), |u| format!("{u:.0}"));
    match current() {
        Lang::En => format!(
            "Measured at {down:.0} Mbps down and {up} Mbps up. If your plan is clearly faster, \
             the test did not fill the line, and the grade may be better than the line."
        ),
        Lang::Pl => format!(
            "Zmierzone przy {down:.0} Mbps pobierania i {up} Mbps wysyłania. Jeśli Twój pakiet \
             jest wyraźnie szybszy, test nie zapchał łącza i ocena może być lepsza niż łącze."
        ),
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
pub fn bloat_partial_load(alive: usize, total: usize, upload: bool) -> String {
    match (current(), upload) {
        (Lang::En, false) => format!(
            "Only {alive} of {total} download streams held up, so the line was not fully loaded and the grade is optimistic."
        ),
        (Lang::En, true) => format!(
            "Only {alive} of {total} upload streams held up, so the line was not fully loaded and the grade is optimistic."
        ),
        (Lang::Pl, false) => format!(
            "Przy pobieraniu utrzymało się tylko {alive} z {total} strumieni, więc łącze nie było w pełni obciążone i ocena jest zawyżona."
        ),
        (Lang::Pl, true) => format!(
            "Przy wysyłaniu utrzymało się tylko {alive} z {total} strumieni, więc łącze nie było w pełni obciążone i ocena jest zawyżona."
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
        Lang::En => format!("{segment} ({confidence})"),
        Lang::Pl => format!("{segment} ({confidence})"),
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
        Lang::En => format!("Could not open {host}:443: {err}"),
        Lang::Pl => format!("Nie udało się otworzyć {host}:443: {err}"),
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

pub fn f_load_detail_up(loaded: f64, mbps: f64) -> String {
    match current() {
        Lang::En => format!("Uploading: {loaded:.0} ms at {mbps:.0} Mbps."),
        Lang::Pl => format!("Przy wysyłaniu: {loaded:.0} ms przy {mbps:.0} Mbps."),
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

pub fn ai_sends(model: &str) -> String {
    match current() {
        Lang::En => format!(
            "Sends this scan's results to {model} through OpenRouter, without your network's \
             name, the access point's MAC or public addresses. It costs a fraction of a cent on \
             your key."
        ),
        Lang::Pl => format!(
            "Wyśle wyniki tego skanu do {model} przez OpenRouter, bez nazwy Twojej sieci, adresu \
             MAC punktu dostępu i adresów publicznych. Kosztuje ułamek centa z Twojego klucza."
        ),
    }
}

pub fn ai_disclaimer(model: &str) -> String {
    match current() {
        Lang::En => format!(
            "Written by {model}. An interpretation, not a measurement: the verdict above comes \
             from the numbers, and where the two disagree, the numbers win."
        ),
        Lang::Pl => format!(
            "Napisane przez {model}. To interpretacja, nie pomiar: werdykt powyżej wynika z \
             liczb, a gdy się nie zgadzają, rację mają liczby."
        ),
    }
}

pub fn ai_err_network(e: &str) -> String {
    match current() {
        Lang::En => format!("Could not reach OpenRouter: {e}"),
        Lang::Pl => format!("Nie udało się połączyć z OpenRouter: {e}"),
    }
}

pub fn ai_err_status(code: u16, detail: &str) -> String {
    let hint = match (code, current()) {
        (401, Lang::En) => " The key is wrong or was revoked.",
        (401, Lang::Pl) => " Klucz jest błędny albo został unieważniony.",
        (402, Lang::En) => " The key has no credits left.",
        (402, Lang::Pl) => " Na kluczu skończyły się środki.",
        _ => "",
    };
    match current() {
        Lang::En => format!("OpenRouter refused the request (HTTP {code}).{hint} {detail}"),
        Lang::Pl => format!("OpenRouter odrzucił zapytanie (HTTP {code}).{hint} {detail}"),
    }
    .trim_end()
    .to_string()
}

pub fn ai_err_reply(e: &str) -> String {
    match current() {
        Lang::En => format!("The reply could not be used: {e}"),
        Lang::Pl => format!("Nie da się użyć odpowiedzi: {e}"),
    }
}

pub fn set_ai_model_hint(default: &str) -> String {
    match current() {
        Lang::En => format!("Any OpenRouter model id. Empty uses {default}."),
        Lang::Pl => format!("Dowolny identyfikator modelu z OpenRouter. Puste oznacza {default}."),
    }
}

pub fn f_ipv6_ok(ms: f64) -> String {
    match current() {
        Lang::En => format!("IPv6 works ({ms:.0} ms to connect)"),
        Lang::Pl => format!("IPv6 działa (połączenie w {ms:.0} ms)"),
    }
}

pub fn f_dns_compare_slow(own: f64, public: f64) -> String {
    match current() {
        Lang::En => format!("Your DNS is slower than a public one ({own:.0} vs {public:.0} ms)"),
        Lang::Pl => {
            format!("Twój DNS jest wolniejszy od publicznego ({own:.0} wobec {public:.0} ms)")
        }
    }
}

pub fn f_dns_compare_ok(own: f64, public: f64) -> String {
    match current() {
        Lang::En => format!("Your DNS keeps up with a public one ({own:.0} vs {public:.0} ms)"),
        Lang::Pl => {
            format!("Twój DNS nie odstaje od publicznego ({own:.0} wobec {public:.0} ms)")
        }
    }
}

pub fn f_dns_compare_detail(own: f64, public: f64) -> String {
    match current() {
        Lang::En => format!(
            "The same question, asked three times of each, best answer counted: the resolvers \
             this connection was given answered in {own:.0} ms, 1.1.1.1 in {public:.0} ms."
        ),
        Lang::Pl => format!(
            "To samo pytanie, zadane każdemu trzy razy, liczy się najlepsza odpowiedź: resolwery \
             przydzielone temu połączeniu odpowiedziały w {own:.0} ms, 1.1.1.1 w {public:.0} ms."
        ),
    }
}

pub fn f_syslog_other(count: usize) -> String {
    match current() {
        Lang::En => format!("Windows logged {count} network event(s) during the scan"),
        Lang::Pl => format!("Zdarzenia sieciowe w dzienniku Windows w trakcie skanu: {count}"),
    }
}

pub fn f_syslog_other_detail(list: &str) -> String {
    match current() {
        Lang::En => format!(
            "{list}. These may belong to another adapter (a VPN, a virtual switch), so they are \
             shown here and not counted against this connection."
        ),
        Lang::Pl => format!(
            "{list}. Mogą dotyczyć innej karty (VPN, przełącznik wirtualny), więc są tu pokazane, \
             ale nie liczą się przeciwko temu połączeniu."
        ),
    }
}

/// Seconds as m:ss, the way a stopwatch shows them.
pub fn mm_ss(secs: usize) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

pub fn step_long(done: usize, total: usize) -> String {
    match current() {
        Lang::En => format!("Long measurement: {} of {}", mm_ss(done), mm_ss(total)),
        Lang::Pl => format!("Długi pomiar: {} z {}", mm_ss(done), mm_ss(total)),
    }
}

fn stopped_early(cancelled: bool) -> &'static str {
    match (cancelled, current()) {
        (false, _) => "",
        (true, Lang::En) => " (stopped early)",
        (true, Lang::Pl) => " (przerwany)",
    }
}

pub fn f_long_clean(watched_s: usize, cancelled: bool) -> String {
    match current() {
        Lang::En => {
            format!("No drops in {} of watching{}", mm_ss(watched_s), stopped_early(cancelled))
        }
        Lang::Pl => {
            format!("Bez zrywów przez {} pomiaru{}", mm_ss(watched_s), stopped_early(cancelled))
        }
    }
}

pub fn f_long_clean_detail(lone_seconds: usize, spikes: usize) -> String {
    match current() {
        Lang::En => format!(
            "Every link was probed once a second. Single lost seconds: {lone_seconds} (every line \
             has a few). Latency spikes: {spikes}. The fault did not happen while this ran, which \
             is not proof it never does: the outage history covers the rest of the day."
        ),
        Lang::Pl => format!(
            "Każde ogniwo pingowane raz na sekundę. Pojedyncze zgubione sekundy: {lone_seconds} \
             (każde łącze ma ich kilka). Skoki opóźnienia: {spikes}. Usterka nie wystąpiła w \
             trakcie pomiaru, co nie dowodzi, że nie występuje: resztę doby pokazuje historia awarii."
        ),
    }
}

pub fn f_long_found(count: usize, watched_s: usize, segment: &str, cancelled: bool) -> String {
    match current() {
        Lang::En => format!(
            "{count} drop(s) in {}, mostly {}{}",
            mm_ss(watched_s),
            segment.to_lowercase(),
            stopped_early(cancelled)
        ),
        Lang::Pl => format!(
            "Zrywy w {} pomiaru: {count}, głównie {}{}",
            mm_ss(watched_s),
            segment.to_lowercase(),
            stopped_early(cancelled)
        ),
    }
}

pub fn f_long_detail(drops: usize, spikes: usize, bad_s: usize, share_pct: f64) -> String {
    match current() {
        Lang::En => format!(
            "Lost packets for two seconds or more: {drops}. Latency spikes above 50 ms: {spikes}. \
             Bad seconds in total: {bad_s}, of which {share_pct:.0}% started at the link named. \
             Each second was measured on every link at once, so the link where the trouble first \
             appears is where it comes from."
        ),
        Lang::Pl => format!(
            "Utrata pakietów przez co najmniej dwie sekundy: {drops}. Skoki opóźnienia ponad 50 ms: \
             {spikes}. Złych sekund łącznie: {bad_s}, z czego {share_pct:.0}% zaczęło się na \
             wskazanym ogniwie. Każda sekunda była mierzona na wszystkich ogniwach naraz, więc \
             ogniwo, na którym kłopot pojawia się pierwszy, jest jego źródłem."
        ),
    }
}

pub fn cost_long(count: usize, watched_s: usize) -> String {
    match current() {
        Lang::En => format!(
            "{count} interruption(s) in {} of watching. Each one is a call that stutters or a game \
             that freezes, and they come back.",
            mm_ss(watched_s)
        ),
        Lang::Pl => format!(
            "Przerwy w {} pomiaru: {count}. Każda to zacinająca się rozmowa albo zamrożona gra, i \
             one wracają.",
            mm_ss(watched_s)
        ),
    }
}

pub fn long_episode_kind(loss: bool) -> &'static str {
    match (loss, current()) {
        (true, Lang::En) => "packets lost",
        (true, Lang::Pl) => "utrata pakietów",
        (false, Lang::En) => "latency spike",
        (false, Lang::Pl) => "skok opóźnienia",
    }
}

pub fn long_episode_len(secs: usize, worst_ms: Option<f64>) -> String {
    match worst_ms {
        Some(ms) => format!("{secs} s, +{ms:.0} ms"),
        None => format!("{secs} s"),
    }
}

/// The caption under one link of the chain: what it added, what it lost.
pub fn link_caption(added_ms: Option<f64>, loss_pct: f64) -> String {
    let loss = match current() {
        Lang::En => format!("{loss_pct:.0}% lost"),
        Lang::Pl => format!("straty {loss_pct:.0}%"),
    };
    match added_ms {
        Some(ms) => format!("+{ms:.0} ms · {loss}"),
        None => loss,
    }
}

pub fn measure_signal(pct: u32) -> String {
    match current() {
        Lang::En => format!("Wi-Fi, signal {pct}%"),
        Lang::Pl => format!("Wi-Fi, sygnał {pct}%"),
    }
}

pub fn measure_baseline_value(ms: f64, readings: usize) -> String {
    match current() {
        Lang::En => format!("{ms:.0} ms (from {readings} readings)"),
        Lang::Pl => format!("{ms:.0} ms (odczyty: {readings})"),
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

pub fn f_apipa_detail(adapter: &str, ip: &str) -> String {
    match current() {
        Lang::En => format!(
            "{adapter} gave itself {ip}, which Windows does when no DHCP server answers. \
             The link is up; the router never handed out an address."
        ),
        Lang::Pl => format!(
            "{adapter} nadała sobie adres {ip}, co Windows robi, gdy żaden serwer DHCP nie \
             odpowiada. Łącze działa, ale router nie przydzielił adresu."
        ),
    }
}

pub fn f_wired_detail(adapter: &str, mbps: u64) -> String {
    format!("{adapter}, {mbps} Mbps")
}

pub fn tray_game_prepare(game: &str) -> String {
    match current() {
        Lang::En => format!("Prepare the connection for {game}"),
        Lang::Pl => format!("Przygotuj łącze do gry: {game}"),
    }
}

pub fn game_prepared(changes: usize) -> String {
    match current() {
        Lang::En => format!(
            "Connection prepared: {changes} change(s) made. They are undone when the game closes."
        ),
        Lang::Pl => format!(
            "Łącze przygotowane, liczba zmian: {changes}. Zostaną cofnięte po zamknięciu gry."
        ),
    }
}

pub fn ov_beyond_busy(mbps: f64) -> String {
    match current() {
        Lang::En => format!("Spike: line, PC uses {mbps:.0} Mb/s"),
        Lang::Pl => format!("Skok: łącze, ruch PC {mbps:.0} Mb/s"),
    }
}

pub fn f_link_errors_detail(errors: u64, packets: u64, pct: f64) -> String {
    match current() {
        Lang::En => format!("{errors} corrupted of {packets} frames ({pct:.2}%)"),
        Lang::Pl => format!("{errors} uszkodzonych z {packets} ramek ({pct:.2}%)"),
    }
}

pub fn f_link_slow_detail(adapter: &str, mbps: u64) -> String {
    match current() {
        Lang::En => format!("{adapter}: negotiated {mbps} Mbps"),
        Lang::Pl => format!("{adapter}: wynegocjowano {mbps} Mbps"),
    }
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
pub fn f_stats_line(
    avg: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    jitter: Option<f64>,
    loss: f64,
) -> String {
    let (avg, min, max, jitter) = (
        figure_or_dash(avg, 0, 1),
        figure_or_dash(min, 0, 1),
        figure_or_dash(max, 0, 1),
        figure_or_dash(jitter, 0, 1),
    );
    match current() {
        Lang::En => {
            format!("avg {avg} ms, min {min}, max {max}, jitter {jitter} ms, loss {loss:.0}%")
        }
        Lang::Pl => {
            format!("śr. {avg} ms, min {min}, maks {max}, jitter {jitter} ms, straty {loss:.0}%")
        }
    }
}

/// Over the before/after figures of a change on the Optimise tab.
pub fn opt_effect_heading(applied: &str) -> String {
    match current() {
        Lang::En => format!("The line around the change made {applied}"),
        Lang::Pl => format!("Łącze wokół zmiany z {applied}"),
    }
}

pub fn opt_effect_before() -> String {
    pick("before", "przed")
}

pub fn opt_effect_after() -> String {
    pick("after", "po")
}

/// One side of the comparison: "before (24 h): ping 18.0 ms, jitter 3.1 ms, loss 0.4%".
pub fn opt_effect_side(
    label: &str,
    span: &str,
    median: Option<f64>,
    jitter: Option<f64>,
    loss: f64,
) -> String {
    let (median, jitter) = (figure_or_dash(median, 0, 1), figure_or_dash(jitter, 0, 1));
    match current() {
        Lang::En => {
            format!("{label} ({span}): ping {median} ms, jitter {jitter} ms, loss {loss:.1}%")
        }
        Lang::Pl => {
            format!("{label} ({span}): ping {median} ms, jitter {jitter} ms, straty {loss:.1}%")
        }
    }
}

pub fn opt_effect_no_data(label: &str) -> String {
    match current() {
        Lang::En => format!("{label}: too few readings to compare"),
        Lang::Pl => format!("{label}: za mało pomiarów, żeby porównać"),
    }
}

pub fn opt_effect_caveat() -> String {
    pick(
        "Measured, not proven: the time of day or another change can move these figures too.",
        "To pomiar, nie dowód: pora dnia albo inna zmiana też mogą przesunąć te liczby.",
    )
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
             queueing, either in your router or at the ISP."
        ),
        Lang::Pl => format!(
            "Opóźnienie rośnie o {bump:.0} ms, gdy łącze jest zajęte, czyli pakiety ustawiają się \
             w kolejce: w Twoim routerze albo u dostawcy."
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
        "airtime_24ghz" => (
            "Strong signal on 2.4 GHz, and it dropped anyway",
            "Mocny sygnał w paśmie 2,4 GHz, a łącze i tak padło",
        ),
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
            "Revert that change and check whether the outages stop. If they do, it was the cause; \
             if not, apply it again and check the next cause.",
            "Cofnij tę zmianę i sprawdź, czy awarie ustaną. Jeśli tak, to ona była przyczyną; \
             jeśli nie, zastosuj ją ponownie i sprawdź kolejną przyczynę.",
        ),
        "adapter_powered_down" => (
            "Turn off power saving for the network card (Optimise tab). This is the most common \
             cause of a connection that drops while the computer is idle.",
            "Wyłącz oszczędzanie energii karty sieciowej (zakładka Optymalizacja). To najczęstsza \
             przyczyna zrywania połączenia, gdy komputer jest bezczynny.",
        ),
        "adapter_power_plan" => (
            "Set the wireless adapter to maximum performance in the power plan (Optimise tab). \
             This is separate from the card's own power saving; both need to be off.",
            "Ustaw kartę bezprzewodową na maksymalną wydajność w planie zasilania (zakładka \
             Optymalizacja). To ustawienie niezależne od oszczędzania energii samej karty; oba \
             muszą być wyłączone.",
        ),
        "out_of_range" | "weak_signal" => (
            "Move closer to the access point or add another one. Below about -75 dBm the \
             connection is unreliable regardless of the router.",
            "Przenieś komputer bliżej access pointa albo dodaj kolejny. Poniżej ok. -75 dBm \
             połączenie jest niestabilne niezależnie od routera.",
        ),
        "adapter_or_driver" => (
            "Reinstall or roll back the network card driver. If the card also disappears from \
             Device Manager, the hardware or its power supply is the likely cause.",
            "Przeinstaluj albo przywróć poprzedni sterownik karty sieciowej. Jeśli karta znika \
             też z Menedżera urządzeń, prawdopodobną przyczyną jest sprzęt lub jego zasilanie.",
        ),
        "roaming" => (
            "The connection broke while switching to another access point. With several access \
             points, give each its own channel and similar transmit power. With a single one, the \
             router switched band by itself.",
            "Połączenie zerwało się podczas przełączania na inny access point. Przy kilku access \
             pointach ustaw każdemu osobny kanał i podobną moc nadawania. Przy jednym oznacza to, \
             że router sam zmienił pasmo.",
        ),
        "signal_fade" => (
            "The signal was falling steadily before the outage: the computer moved away or \
             something blocked the signal. Not a router fault.",
            "Sygnał spadał stopniowo przed awarią: komputer się oddalił albo coś zasłoniło \
             sygnał. To nie jest usterka routera.",
        ),
        "airtime_24ghz" => (
            "2.4 GHz is shared with neighbouring networks, microwaves and Bluetooth. Switch to 5 \
             GHz if the card supports it, or set channel 1, 6 or 11. The Wi-Fi channel check in \
             Optimise shows which is least crowded.",
            "Pasmo 2,4 GHz jest współdzielone z sieciami sąsiadów, mikrofalówkami i Bluetooth. \
             Przejdź na 5 GHz, jeśli karta to obsługuje, albo ustaw kanał 1, 6 lub 11. \
             Sprawdzenie kanałów w zakładce Optymalizacja pokazuje, który jest najmniej \
             zatłoczony.",
        ),
        "router_side" => (
            "The signal was strong and steady until the outage, so the Wi-Fi link was fine. Check \
             whether other devices lose the connection at the same time, make sure the router is \
             not hot or covered, and restart it.",
            "Sygnał był mocny i stabilny aż do awarii, więc łącze Wi-Fi działało poprawnie. \
             Sprawdź, czy inne urządzenia tracą połączenie w tym samym czasie, czy router nie \
             jest gorący albo zasłonięty, i uruchom go ponownie.",
        ),
        "marginal_link" => (
            "The connection broke on a weak signal and came back on a much stronger one, so it \
             works at the edge of range. Small changes (a closed door, a person nearby) can keep \
             breaking it.",
            "Połączenie zerwało się przy słabym sygnale i wróciło przy znacznie mocniejszym, więc \
             działa na granicy zasięgu. Drobne zmiany (zamknięte drzwi, osoba w pobliżu) mogą je \
             dalej zrywać.",
        ),
        "cable_or_router" => (
            "Wired connection: check the cable and the port first. Reconnect both ends, then try \
             another port and another cable. A faulty cable looks the same as a faulty router.",
            "Połączenie kablowe: najpierw sprawdź kabel i port. Podłącz ponownie oba końce, potem \
             spróbuj innego portu i innego kabla. Uszkodzony kabel wygląda tak samo jak \
             uszkodzony router.",
        ),
        "isp_sustained" | "isp_brief" => (
            "The router kept answering, so the break was past it, on the provider side. Nothing \
             on this computer will fix it; save a report as evidence for the provider.",
            "Router cały czas odpowiadał, więc przerwa była za nim, po stronie dostawcy. Nic na \
             tym komputerze tego nie naprawi; zapisz raport jako dowód dla dostawcy.",
        ),
        "isp_pattern" => (
            "Repeated WAN outages point to a fault in the service. Save a report with the dates \
             and times of the outages and send it to the provider.",
            "Powtarzające się awarie WAN wskazują na usterkę usługi. Zapisz raport z datami i \
             godzinami awarii i przekaż go dostawcy.",
        ),
        "dns_router_only" => (
            "All name lookups go through the router, so when its DNS stalls, the internet looks \
             down even though it is reachable. Add a public DNS server (Optimise tab).",
            "Wszystkie zapytania o nazwy przechodzą przez router, więc gdy jego DNS się zatnie, \
             internet wygląda na niedostępny, choć działa. Dodaj publiczny serwer DNS (zakładka \
             Optymalizacja).",
        ),
        "dns_resolver" => (
            "Names stopped resolving while the network was up. Switching to a public DNS server \
             tells a DNS fault from a connection fault.",
            "Nazwy przestały się rozwiązywać, choć sieć działała. Przełączenie na publiczny \
             serwer DNS pozwala odróżnić awarię DNS od awarii połączenia.",
        ),
        "local_saturation" => (
            "Latency to the router rose before the quality dropped, so something on this side \
             filled the connection: an upload, a backup or an update. The speed test confirms it.",
            "Opóźnienie do routera rosło, zanim spadła jakość, więc coś po tej stronie zapchało \
             łącze: wysyłanie, kopia zapasowa albo aktualizacja. Test prędkości pozwala to \
             potwierdzić.",
        ),
        "rate_collapse" => (
            "The Wi-Fi link rate dropped before the quality did. The cause is interference or \
             distance, not router performance, so a faster router will not help.",
            "Prędkość łącza Wi-Fi spadła, zanim spadła jakość. Przyczyną są zakłócenia albo \
             odległość, a nie wydajność routera, więc szybszy router nie pomoże.",
        ),
        "time_pattern" => (
            "Outages at the same hour usually follow a schedule: a device nearby, the router's \
             nightly restart, a backup job, or provider maintenance.",
            "Awarie o tej samej godzinie zwykle wynikają z harmonogramu: urządzenie w pobliżu, \
             nocny restart routera, kopia zapasowa albo prace serwisowe dostawcy.",
        ),
        "no_evidence" => (
            "This entry comes from a version that did not record the state before an outage. New \
             entries include it.",
            "Ten wpis pochodzi z wersji, która nie zapisywała stanu przed awarią. Nowe wpisy go \
             zawierają.",
        ),
        "unclear" => (
            "The recorded state does not point to a single cause. If the outage repeats, compare \
             the lead-up charts to see what changes each time.",
            "Zapisany stan nie wskazuje jednej przyczyny. Jeśli awaria się powtórzy, porównaj \
             wykresy przebiegu, żeby zobaczyć, co zmienia się za każdym razem.",
        ),
        "log_sleep" => (
            "The network was fine: the computer went to sleep. If it should not sleep, change the \
             sleep time in the Windows power settings.",
            "Sieć działała poprawnie: komputer przeszedł w stan uśpienia. Jeśli nie powinien się \
             usypiać, zmień czas uśpienia w ustawieniach zasilania Windows.",
        ),
        "log_resume" => (
            "After waking, the card has to reconnect and renew its address, which takes a few \
             seconds. An outage of that length is normal behaviour, not a fault.",
            "Po wybudzeniu karta musi ponownie się połączyć i odnowić adres, co trwa kilka \
             sekund. Przerwa tej długości to normalne działanie, a nie usterka.",
        ),
        "log_driver_fault" => (
            "The network card driver reported an error, so this is not a router or provider \
             problem. Update the driver from the manufacturer's site (Windows Update often has an \
             older one). Resetting the network stack (Optimise tab) clears the leftover state.",
            "Sterownik karty sieciowej zgłosił błąd, więc to nie jest problem routera ani \
             dostawcy. Zaktualizuj sterownik ze strony producenta (Windows Update często ma \
             starszy). Reset stosu sieciowego (zakładka Optymalizacja) czyści pozostały stan.",
        ),
        "log_wlan_inactivity" => (
            "The access point disconnected the card after Windows put it into power saving. This \
             is a typical cause of drops while the computer is idle. Turn off power saving for \
             the card and in the power plan (Optimise tab).",
            "Access point rozłączył kartę, gdy Windows przełączył ją w oszczędzanie energii. To \
             typowa przyczyna zrywania połączenia, gdy komputer jest bezczynny. Wyłącz \
             oszczędzanie energii karty i w planie zasilania (zakładka Optymalizacja).",
        ),
        "log_wlan_auth" => (
            "The card reached the access point but was not let in. The cause is the password or \
             credentials, not the signal. Forget the network in Windows and connect again.",
            "Karta dotarła do access pointa, ale nie została wpuszczona. Przyczyną jest hasło \
             albo dane logowania, a nie sygnał. Usuń sieć w Windows (Zapomnij) i połącz się \
             ponownie.",
        ),
        "log_wlan_ap_rejected" => (
            "The access point refused the connection, usually because it has no free slots. Check \
             how many devices are connected to it and whether a guest network or mesh node takes \
             up slots.",
            "Access point odrzucił połączenie, zwykle z powodu braku wolnych miejsc. Sprawdź, ile \
             urządzeń jest do niego podłączonych i czy sieć gościnna albo węzeł mesh nie zajmuje \
             miejsc.",
        ),
        "log_wlan_deauth" => (
            "Windows recorded the disconnect, so the connection was actually broken, not just \
             silent. Quote the reason code above when reporting it to the router manufacturer or \
             the provider.",
            "Windows zapisał rozłączenie, więc połączenie zostało faktycznie zerwane, a nie tylko \
             ucichło. Podaj kod przyczyny powyżej, zgłaszając problem producentowi routera albo \
             dostawcy.",
        ),
        "log_dhcp" => (
            "The card could not get an address from the router, and without one nothing works \
             regardless of the signal. Restart the router or check that its DHCP address pool is \
             not full.",
            "Karta nie otrzymała adresu od routera, a bez niego nic nie działa niezależnie od \
             sygnału. Uruchom router ponownie albo sprawdź, czy pula adresów DHCP nie jest pełna.",
        ),
        "log_duplicate_ip" => (
            "Two devices use the same IP address. Usually one has a static address set inside the \
             router's DHCP range. Move it outside the range or use a DHCP reservation instead.",
            "Dwa urządzenia używają tego samego adresu IP. Zwykle jedno ma adres statyczny \
             ustawiony w zakresie DHCP routera. Przenieś go poza ten zakres albo użyj rezerwacji \
             DHCP.",
        ),
        "log_link_down" => (
            "Windows recorded the network interface going down. The cause is on this computer: \
             the card, its driver, the cable or power saving, not past the router.",
            "Windows zapisał wyłączenie interfejsu sieciowego. Przyczyna leży po stronie \
             komputera: karta, sterownik, kabel albo oszczędzanie energii, a nie za routerem.",
        ),
        "log_clean_isp" => (
            "The Windows log shows no fault on this computer in this period: no sleep, driver \
             error, disconnect or DHCP failure. With the router answering throughout, the outage \
             was past your equipment, on the provider side.",
            "Dziennik Windows nie wykazuje w tym okresie usterki na tym komputerze: brak \
             uśpienia, błędu sterownika, rozłączenia i awarii DHCP. Router cały czas odpowiadał, \
             więc awaria była poza Twoim sprzętem, po stronie dostawcy.",
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
        "the active power plan can switch the radio off independently of the card's own setting",
        "aktywny plan zasilania może wyłączać radio niezależnie od ustawienia samej karty",
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
        Lang::En => format!("signal fell from {from} to {to} dBm ({drop} dB) before the drop"),
        Lang::Pl => {
            format!("sygnał spadł z {from} do {to} dBm ({drop} dB) przed zerwaniem")
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
        "the router is the only configured DNS server, so when it stalls every lookup fails",
        "router jest jedynym skonfigurowanym serwerem DNS, więc gdy się zatnie, zawodzą \
         wszystkie zapytania",
    )
}

pub fn ev_router_wan_down(status: &str) -> String {
    match current() {
        Lang::En => format!(
            "the router itself reported its internet connection as \"{status}\" during the outage"
        ),
        Lang::Pl => format!(
            "router sam zgłaszał w trakcie awarii swoje połączenie z internetem jako „{status}”"
        ),
    }
}

pub fn ev_router_restarted() -> String {
    pick(
        "the router's uptime counter started again during the outage: the router, or its \
         connection to the provider, restarted",
        "licznik czasu działania routera ruszył od nowa w trakcie awarii: uruchomił się ponownie \
         router albo jego połączenie z dostawcą",
    )
}

pub fn ev_wan_new_ip() -> String {
    pick(
        "the router came back with a different public address: its session with the provider \
         was set up anew",
        "router wrócił z innym adresem publicznym: jego sesja u dostawcy została nawiązana od nowa",
    )
}

pub fn ev_router_wan_up() -> String {
    pick(
        "the router reported its internet connection as up throughout: the break was past it, \
         in the provider's network or beyond",
        "router cały czas zgłaszał połączenie z internetem jako aktywne: przerwa była dalej, w \
         sieci dostawcy albo za nią",
    )
}

pub fn ev_dns_own_resolver(err: &str) -> String {
    let what = ev_dns_error(err);
    match current() {
        Lang::En => {
            format!("{what}; the resolver is on your own network, so check that it is running")
        }
        Lang::Pl => {
            format!("{what}; resolver jest w Twojej własnej sieci, więc sprawdź, czy działa")
        }
    }
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
pub fn path_blame_advice(mine: Option<bool>) -> &'static str {
    match (current(), mine) {
        (Lang::En, None) => {
            "That hop has a private address behind your router. It is either a second router of \
             yours (a mesh node, a provider box in router mode) or the provider's own network, \
             which is often numbered privately. If you have no second router, it is the \
             provider's: quote the hop number, the address and this loss figure to them."
        }
        (Lang::Pl, None) => {
            "Ten skok ma prywatny adres za twoim routerem. To albo twój drugi router (węzeł mesh, \
             modem dostawcy w trybie routera), albo sieć samego dostawcy, która często ma \
             prywatne adresy. Jeśli nie masz drugiego routera, to sieć dostawcy: podaj mu numer \
             skoku, adres i tę wartość straty."
        }
        (Lang::En, Some(true)) => {
            "That hop is your own equipment, so this one is fixable here: the router, a second \
             router behind it, or the link between them."
        }
        (Lang::Pl, Some(true)) => {
            "Ten skok to twój własny sprzęt, więc da się to naprawić u siebie: router, drugi \
             router za nim albo łącze między nimi."
        }
        (Lang::En, Some(false)) => {
            "That hop is past your equipment. Quote the hop number, the address and this loss \
             figure to the provider — it is the one form of evidence a support line cannot \
             answer with \"restart the router\"."
        }
        (Lang::Pl, Some(false)) => {
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
        Lang::En => {
            format!("{lines} log entries around this outage, none of them a fault on this computer")
        }
        Lang::Pl => format!(
            "wpisy w dzienniku wokół tej awarii: {lines}, żaden nie wskazuje usterki tego komputera"
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
/// After the outage history was cleared.
pub fn hist_cleared(count: usize) -> String {
    match current() {
        Lang::En => format!("Outage history cleared ({count} deleted)."),
        Lang::Pl => format!("Historia awarii wyczyszczona (usunięto: {count})."),
    }
}

pub fn hist_clear_failed(detail: &str) -> String {
    match current() {
        Lang::En => format!("Could not clear the history: {detail}"),
        Lang::Pl => format!("Nie udało się wyczyścić historii: {detail}"),
    }
}

pub fn hist_cause_for(when: &str) -> String {
    match current() {
        Lang::En => format!("Outage of {when}"),
        Lang::Pl => format!("Awaria z {when}"),
    }
}

/// Ping by address works but names do not resolve.
pub fn dns_no_answer(secs: u64) -> String {
    match current() {
        Lang::En => format!("resolver did not answer within {secs} s"),
        Lang::Pl => format!("resolver nie odpowiedział w ciągu {secs} s"),
    }
}

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
        Lang::En => {
            format!("Worst sample while downloading: {max:.0} ms, packet loss {loss_pct:.0}%.")
        }
        Lang::Pl => format!(
            "Najgorsza próbka przy pobieraniu: {max:.0} ms, utrata pakietów {loss_pct:.0}%."
        ),
    }
}

pub fn bloat_worst_up(max: f64, loss_pct: f64) -> String {
    match current() {
        Lang::En => {
            format!("Worst sample while uploading: {max:.0} ms, packet loss {loss_pct:.0}%.")
        }
        Lang::Pl => {
            format!("Najgorsza próbka przy wysyłaniu: {max:.0} ms, utrata pakietów {loss_pct:.0}%.")
        }
    }
}

/// Labels in the tweak detail panel.
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
        Lang::Pl => {
            // Polish counts in three forms: 1 awaria, 2-4 awarie (but 12-14
            // awarii), everything else awarii.
            let noun = match (count % 10, count % 100) {
                _ if count == 1 => "awaria",
                (2..=4, r) if !(12..=14).contains(&r) => "awarie",
                _ => "awarii",
            };
            format!("{count} {noun} w ciągu ostatnich 24 godzin, głównie {where_text}.")
        }
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

/// Headline of the "uninterrupted" card while the history is still shorter
/// than the 24 h window the card talks about.
pub fn live_uninterrupted_for(secs: f64) -> String {
    let mins = (secs / 60.0).floor().max(0.0);
    if mins < 90.0 {
        return format!("{mins:.0} min");
    }
    // "min" and "h" read the same in both languages, so there is nothing to
    // switch on here.
    format!("{:.0} h", secs / 3600.0)
}

/// Sub-line for that card: says the number is the length of the record, not a
/// promise about the line before the app was installed.
pub fn live_uninterrupted_short() -> &'static str {
    match current() {
        Lang::En => "watched since app start",
        Lang::Pl => "tyle trwa obserwacja",
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
/// The footer of a PDF report page.
pub fn pdf_page(n: usize, total: usize) -> String {
    match current() {
        Lang::En => format!("NetDoctor · page {n} of {total}"),
        Lang::Pl => format!("NetDoctor · strona {n} z {total}"),
    }
}

/// Heading over the list of periods of poor quality, after the outages.
pub fn rep_slow_heading(count: usize) -> String {
    match current() {
        Lang::En => {
            format!("PERIODS OF POOR QUALITY ({count}): the connection worked, with loss or lag")
        }
        Lang::Pl => format!(
            "OKRESY POGORSZONEJ JAKOŚCI ({count}): połączenie działało, ale ze stratami lub lagami"
        ),
    }
}

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

pub fn air_on_dfs(channel: u32) -> String {
    match current() {
        Lang::En => format!(
            "Channel {channel} is a radar (DFS) channel. When the router detects radar it \
             must leave the channel within 10 s and stay off it for 30 min, which shows up \
             as an outage with no visible cause."
        ),
        Lang::Pl => format!(
            "Kanał {channel} to kanał radarowy (DFS). Po wykryciu radaru router musi \
             opuścić kanał w ciągu 10 s i nie wraca na niego przez 30 min. Objawia się to \
             przerwą bez widocznej przyczyny."
        ),
    }
}

/// A channel number as the bars label it.
pub fn air_channel_no(channel: u32) -> String {
    match current() {
        Lang::En => format!("Channel {channel}"),
        Lang::Pl => format!("Kanał {channel}"),
    }
}

/// The figures beside one channel's bar.
///
/// With no level measured and networks heard, only the count is given: that
/// is a channel whose loudness was not read, not a quiet one.
pub fn air_bar_note(aps: usize, noise: Option<f64>) -> String {
    let noise = match (noise, aps) {
        (Some(n), _) => format!(" · {n:.0} dBm"),
        (None, 0) => format!(" · {}", pick("nothing heard", "cisza")),
        (None, _) => String::new(),
    };
    match current() {
        Lang::En => format!("{aps} networks{noise}"),
        Lang::Pl => format!("sieci: {aps}{noise}"),
    }
}

pub fn air_networks(count: usize) -> String {
    match current() {
        Lang::En => format!("Networks nearby ({count})"),
        Lang::Pl => format!("Sieci w pobliżu ({count})"),
    }
}

pub fn air_verdict_move(channel: u32) -> String {
    match current() {
        Lang::En => format!("Set your router to 2.4 GHz channel {channel}"),
        Lang::Pl => format!("Ustaw w routerze kanał {channel} na 2,4 GHz"),
    }
}

pub fn air_verdict_move_why(current_channel: u32, gain: f64) -> String {
    match current() {
        Lang::En => {
            format!("{gain:.0} dB quieter than the current channel {current_channel}.")
        }
        Lang::Pl => {
            format!("O {gain:.0} dB ciszej niż na obecnym kanale {current_channel}.")
        }
    }
}

pub fn air_verdict_fine(channel: u32) -> String {
    match current() {
        Lang::En => format!("Channel {channel} is fine"),
        Lang::Pl => format!("Kanał {channel} jest w porządku"),
    }
}

pub fn air_verdict_dfs(channel: u32, best: Option<u32>) -> String {
    match (current(), best) {
        (Lang::En, Some(b)) => {
            format!("Channel {channel} can switch itself off: set channel {b} on your router")
        }
        (Lang::Pl, Some(b)) => {
            format!("Kanał {channel} potrafi sam się wyłączyć: ustaw w routerze kanał {b}")
        }
        (Lang::En, None) => format!("Channel {channel} can switch itself off: move to another"),
        (Lang::Pl, None) => format!("Kanał {channel} potrafi sam się wyłączyć: przejdź na inny"),
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

pub fn upd_err_no_digest(asset: &str) -> String {
    match current() {
        Lang::En => format!("The release's checksum file does not list {asset}."),
        Lang::Pl => format!("Plik sum kontrolnych wydania nie wymienia {asset}."),
    }
}

pub fn upd_err_checksum(got: &str, want: &str) -> String {
    match current() {
        Lang::En => format!(
            "The download does not match the release's checksum. \
             Expected {want}, got {got}. Nothing was installed."
        ),
        Lang::Pl => format!(
            "Pobrany plik nie zgadza się z sumą kontrolną wydania. \
             Oczekiwano {want}, jest {got}. Nic nie zostało zainstalowane."
        ),
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
    fn a_short_span_is_not_rounded_to_nothing() {
        // The report summed two provider outages of 3 s and 8 s as
        // "0 min down in total".
        assert_eq!(span(11.0), "11 s");
        assert_eq!(span(0.4), "0 s");
        assert_eq!(span(59.4), "59 s");
        assert_eq!(span(300.0), "5 min");
        assert_eq!(span(3.0 * 3600.0 + 20.0 * 60.0), "3 h 20 min");
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
                // The other half of the same mistake: a doubled backslash is a
                // literal one, so the newline and indent survive and a `\`
                // shows in the middle of the sentence. It hid behind the
                // exception below, because the indent it keeps looks exactly
                // like laid-out block text.
                assert!(!text.contains("\\\n"), "{name}: a literal backslash before a line break");
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
