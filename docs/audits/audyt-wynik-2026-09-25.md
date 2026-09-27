# Audyt funkcji NetDoctor: wynik (2026-09-25, gałąź fix/steady-layout, 61fa5fd)

Stan repozytorium: `cargo clippy -- -D warnings` czysto, `cargo test` 375 zaliczonych, 15 pominiętych.
Punkt odniesienia: [audyt-wynik-2026-09-23.md](audyt-wynik-2026-09-23.md). Od tamtej pory 31 commitów.

**Aktualizacja (ten sam dzień):** ustalenia G, B i H są naprawione, niezacommitowane. G objęło też piąty kod bez tekstu, `dns_own_resolver`, którego ten audyt nie wymienił. Kod przyczyny jest teraz typem (`cause::Code`), a tytuł i porada to wyczerpujący `match` bez gałęzi domyślnej: przyczyna bez tekstów nie skompiluje się. Test `every_cause_has_words_in_both_languages` sprawdza, że żaden z tekstów nie jest pusty w żadnym języku. Potem naprawione także C, D, E, F i I. Otwarte zostają ustalenie 10 i hipotezy A, J.

## Co i jak zostało sprawdzone

| Obszar | Metoda | Zakres |
|---|---|---|
| 13 ustaleń z 23.09 | kod czytany linijka po linijce, nie opisy commitów | wszystkie 13 |
| monitor, przyczyny, optymalizacja | kod i testy | `monitor.rs` (werdykt, TCP, eskalacja), `cause.rs` (reguły zmian, logów, routera), `optimize/mod.rs` (snapshot, apply, revert, DNS), `effect.rs`, `game.rs` |
| moduły nowe od 23.09 | kod w całości | `probe/igd.rs` (nagłówek i reguły), `longrun.rs`, `overlay.rs` (werdykt i brak danych), `update.rs`, `ai.rs` (co wysyła) |
| test obciążenia i kanały Wi-Fi | kod w całości | `bandwidth.rs` 140-400, `airscan.rs` 150-255 |
| teksty PL i EN | skrypt porównujący 1060 par (symbole zastępcze, liczby, jednostki, długość, puste strony) plus ręczna lektura wszystkich tytułów i porad przyczyn | cały `i18n.rs` |
| przechwytywanie TCP 443 (hipoteza A) | test na tym komputerze: połączenie na adresy, których nie ma (192.0.2.1, 198.51.100.7) | Windows Defender |
| ekran | zrzut działającej aplikacji 1.5.0 | nagłówek i Ustawienia |

## 1. Werdykt

- **Sprawdza internet: spełnia, z jedną luką.** Pingi filtrowane na zewnątrz nie udają już awarii dostawcy, a błąd zapisu do bazy szarzeje werdykt. Luka: gdy pingi są filtrowane, awaria DNS pokazuje się jako "działa" (ustalenie C).
- **Szuka problemów: częściowo.** Eskalacja wymaga serii odczytów, nieudana zmiana nie jest obwiniana, a router po UPnP daje nowy dowód. Trzy błędy psują jednak właśnie dowód: cofnięta zmiana jest nadal "prawdopodobną przyczyną" (B), długi pomiar obwinia sieć domową za router, który tylko gubi pingi do siebie (H), a przyczyny z UPnP trafiają do Historii i raportu jako surowe kody bez porady (G).
- **Optymalizuje sieć: częściowo.** Revert DNS wraca do DHCP, test obciążenia mierzy oba kierunki, a skutek zmiany jest pokazany jako pomiar, bez orzekania. Revert DNS może jednak zamknąć aplikację (D).

## 2. Ustalenia

### Stan ustaleń z 23 września

| # | Ustalenie z 23.09 | Stan | Dowód |
|---|---|---|---|
| 1 | Revert DNS przypinał adres z DHCP | naprawione: źródło z `NameServer`, stare snapshoty porównywane z `DhcpNameServer`, nieznane źródło blokuje Apply | [optimize/mod.rs:689-692](../src/optimize/mod.rs#L689-L692), [optimize/mod.rs:807-812](../src/optimize/mod.rs#L807-L812) |
| 2 | Jeden zgubiony ping routera zmieniał awarię ISP w LAN | naprawione: eskalacja po `outage_after_fails` kolejnych odczytach | [monitor.rs:1145-1158](../src/monitor.rs#L1145-L1158), [monitor.rs:1465](../src/monitor.rs#L1465) |
| 3 | Blokada ICMP = stała awaria ISP | naprawione (próba TCP co 2 s, tylko rozpoczęta po zamilknięciu pingów, ważna 6 s); patrz C | [monitor.rs:742-759](../src/monitor.rs#L742-L759), [monitor.rs:486-489](../src/monitor.rs#L486-L489) |
| 4 | Nieudana zmiana obwiniana za awarię | naprawione (`apply_failed`, filtr w `analyse`); patrz B | [ui/opt.rs:575](../src/ui/opt.rs#L575), [cause.rs:211](../src/cause.rs#L211) |
| 5 | Różne progi jittera w Live i Diagnozie | naprawione | [monitor.rs:1641-1645](../src/monitor.rs#L1641-L1645) |
| 6 | "Szybki DNS" doradzany przy Pi-hole | naprawione | [netstate.rs:131-133](../src/probe/netstate.rs#L131-L133), [optimize/mod.rs:679-683](../src/optimize/mod.rs#L679-L683) |
| 7 | Brak pomiaru skutku | naprawione: doba przed i czas po, próg 600 próbek, koniec na cofnięciu | [effect.rs:47-82](../src/effect.rs#L47-L82) |
| 8 | Test obciążenia tylko w dół | naprawione: faza wysyłania, ocena gorszego kierunku, martwe strumienie zgłaszane | [bandwidth.rs:305-316](../src/bandwidth.rs#L305-L316), [bandwidth.rs:367-390](../src/bandwidth.rs#L367-L390) |
| 9 | Błąd zapisu próbek dawał zielony werdykt | naprawione (`Seen::Unrecorded`) | [monitor.rs:203-204](../src/monitor.rs#L203-L204) |
| 10 | Brak bramy = "karta odłączona" (VPN) | **otwarte**, złagodzone: gdy TCP przechodzi, werdykt to "działa" | [monitor.rs:1593](../src/monitor.rs#L1593) |
| 11 | Ostatni widoczny hop obwiniany bez potwierdzenia | naprawione, z testem | [path.rs:441](../src/probe/path.rs#L441) |
| 12 | Kanały 5 GHz bez sąsiadów 80 MHz | naprawione; bloki policzone ręcznie dla 36-144, 149-161 i 6 GHz, zgodne | [airscan.rs:200-243](../src/probe/airscan.rs#L200-L243) |
| 13 | Nieudany Revert poza dziennikiem | naprawione (`revert_failed`) | [ui/opt.rs:591](../src/ui/opt.rs#L591) |

### Nowe ustalenia (każde potwierdzone w kodzie)

| # | Obietnica | Ustalenie | Dowód | Waga | Kierunek |
|---|---|---|---|---|---|
| B | szukanie problemów | **Zmiana cofnięta przed awarią nadal jest "prawdopodobną przyczyną".** `analyse` bierze ostatnie `apply` z 20 minut przed awarią i nie patrzy, czy potem było `revert`. Tryb gry stosuje zmianę na starcie i cofa ją po zamknięciu gry, więc awaria kwadrans po partii zostanie przypisana trybowi gry, wyżej niż każdy odczyt sygnału. Dane o cofnięciu już tu trafiają (`tweaks_between` obejmuje godzinę), a `effect.rs` robi to dobrze. | [cause.rs:209-223](../src/cause.rs#L209-L223), [game.rs:381](../src/game.rs#L381), [ui/report.rs:342](../src/ui/report.rs#L342), wzór: [effect.rs:53-57](../src/effect.rs#L53-L57) | wysoka | Pomijać `apply`, po którym przed `event.ts_start` było `revert` tej samej zmiany. Test z parą apply/revert. |
| H | szukanie problemów | **Długi pomiar obwinia sieć domową za router, który gubi pingi do siebie.** Sekunda bez odpowiedzi bramy to od razu strata w segmencie LAN, także gdy internet w tej samej sekundzie odpowiedział. Komentarz dwie linijki niżej mówi, że "routers drop pings addressed to themselves long before they drop traffic", ale ta ostrożność dotyczy tylko hopu dostawcy. Monitor i tabela z README mówią odwrotnie: router milczy, internet odpowiada, więc działa. Żaden test nie obejmuje bramy gubiącej część pingów przy działającym internecie. | [longrun.rs:146-148](../src/longrun.rs#L146-L148), [longrun.rs:143-145](../src/longrun.rs#L143-L145), testy [longrun.rs:313-412](../src/longrun.rs#L313-L412) | wysoka | Strata bramy liczy się tylko wtedy, gdy w tej samej sekundzie milczy też internet. Test: `gw: None, net: Some` przy `gw_med` obecnym. |
| G | szukanie problemów, pokazywanie | **Przyczyny z UPnP pokazują się jako surowe kody.** `router_wan_down`, `router_restarted`, `wan_new_ip` i `router_wan_up` nie mają tytułu ani porady. `cause_title` zwraca wtedy sam kod, a `cause_advice` pusty tekst. Użytkownik i konsultant dostawcy zobaczą w Historii i raporcie nagłówek "router_wan_down". Komentarz pola `code` mówi "Never shown raw". | [cause.rs:316-345](../src/cause.rs#L316-L345), [i18n.rs:3921](../src/i18n.rs#L3921), [i18n.rs:4141](../src/i18n.rs#L4141), [ui/report.rs:356](../src/ui/report.rs#L356), [cause.rs:46](../src/cause.rs#L46) | wysoka (psuje dowód dla dostawcy) | Dodać cztery tytuły i porady w obu językach. Test, który przechodzi po każdym kodzie z `cause.rs` i wymaga niepustego tytułu różnego od kodu. |
| C | sprawdzanie łącza | **Gdy pingi są filtrowane, DNS nie wchodzi do werdyktu.** `classify` sprawdza `dns_error` tylko w gałęzi, w której internet odpowiada na ping. Werdykt poręczony przez TCP zamienia się na `Ok` bez spojrzenia na DNS. README: "only calls it an outage if that fails too". | [monitor.rs:1571-1576](../src/monitor.rs#L1571-L1576), [monitor.rs:1610-1617](../src/monitor.rs#L1610-L1617) | średnia | W `past_filtered_pings` przy niepustym `dns_error` zwracać `DnsFail`. |
| D | optymalizacja | **Revert DNS może zamknąć aplikację.** Przy `dhcp: false` i pustej liście `servers[0]` wychodzi poza zakres. Flaga pochodzi z rejestru (`NameServer`), a lista z `net.dns_servers` odczytanego w tej chwili, więc mogą się rozjechać (np. rozłączone Wi-Fi). Revert działa w wątku interfejsu, więc panic zamyka okno. Łamie też regułę repo o braku panic. | [optimize/mod.rs:731-734](../src/optimize/mod.rs#L731-L734), [optimize/mod.rs:753](../src/optimize/mod.rs#L753), [ui/opt.rs:585](../src/ui/opt.rs#L585) | średnia | `servers.first()` z błędem albo zapisywać w snapshocie listę z `NameServer`. |
| E | szukanie problemów | **Restart samej sesji PPPoE czyta się jak restart routera.** `NewUptime` z IGD na wielu routerach liczy czas połączenia WAN. Tekst mówi uczciwie "router albo jego połączenie z dostawcą", ale sama obecność tej przyczyny wyłącza `router_wan_up`. | [cause.rs:322-346](../src/cause.rs#L322-L346), [i18n.rs:4289-4295](../src/i18n.rs#L4289-L4295) | niska | Przy `router_wan_down` albo `wan_new_ip` w tej samej awarii nazwać to zerwaniem sesji u dostawcy. |
| F | optymalizacja | **Tryb gry może zgubić zmianę z listy do cofnięcia.** Gdy zmiana już zadziałała, a `save_session` zwróci błąd, `?` kończy `prepare`, zanim zmiana trafi do pliku sesji. Ręczny Revert w Optymalizacji nadal działa. | [game.rs:339-343](../src/game.rs#L339-L343) | niska | Zapis sesji przed `apply` albo natychmiastowe cofnięcie przy błędzie zapisu. |
| I | bezpieczeństwo aktualizacji | **Wydanie bez `SHA256SUMS` instaluje się bez sumy kontrolnej.** Uzasadnienie w kodzie dotyczy starych wydań, ale brak pliku sprawdzany jest dla nowego wydania, które ta wersja pobiera. Ryzyko jest ograniczone, bo binarka i suma pochodzą z tego samego konta GitHub (opisane jako `ponytail`). | [update.rs:333-335](../src/update.rs#L333-L335) | niska | Wymagać `SHA256SUMS` od wydań nowszych niż ta, która pierwsza je publikowała. |

### Hipotezy (nie potwierdzone, z testem rozstrzygającym)

| # | Hipoteza | Co sprawdziłem | Test rozstrzygający | Waga, jeśli się potwierdzi |
|---|---|---|---|---|
| A | Antywirus z ochroną WWW albo portal hotelowy kończy połączenie TCP na 443 lokalnie, więc przy awarii dostawcy werdykt to "działa, sieć blokuje pingi". `tcp_probe` to samo `connect_timeout`, bez TLS ([diagnose.rs:1611-1619](../src/diagnose.rs#L1611-L1619)). | Na tym komputerze (Windows Defender) połączenia na nieistniejące adresy nie przechodzą, więc tu problemu nie ma. Nie znalazłem źródła, które by potwierdzało takie zachowanie ESET, Kasperskiego lub Avasta. | Na komputerze z danym programem: `Test-NetConnection 192.0.2.1 -Port 443`. `TcpTestSucceeded : True` potwierdza hipotezę. | wysoka |
| J | W Polsce kanały 149-165 bywają niedostępne w routerach, a rekomendacja może je wskazać, gdy 36-48 są zajęte ([airscan.rs:33](../src/probe/airscan.rs#L33)). | Kod: lista kandydatów nie zależy od kraju. | Sprawdzić listę kanałów 5 GHz w routerze sprzedawanym w PL. | niska |

### Mocne strony, które warto zachować

Poręczenie TCP tylko przez próbę rozpoczętą po zamilknięciu pingów ([monitor.rs:486-489](../src/monitor.rs#L486-L489)). Reguły routera opierają się na jego słowach, nigdy na braku odczytu ([cause.rs:304-308](../src/cause.rs#L304-L308)). Odcisk publicznego IP zamiast adresu ([igd.rs:39-43](../src/probe/igd.rs#L39-L43)). Nakładka bez aktualnego werdyktu pisze "nie mierzę" i nigdy nie chowa awarii ([overlay.rs:100-131](../src/overlay.rs#L100-L131)). Długi pomiar nie liczy straty łącza, które nigdy nie odpowiedziało ([longrun.rs:131-133](../src/longrun.rs#L131-L133)). Aktualizator sprawdza rozmiar, nagłówek `MZ` i SHA-256 oraz wraca do starej binarki, gdy podmiana się nie uda ([update.rs:205-221](../src/update.rs#L205-L221)). AI jest domyślnie wyłączone, działa na klucz użytkownika i maskuje SSID, BSSID oraz publiczne adresy ([ai.rs:1-14](../src/ai.rs#L1-L14)).

## 3. README a kod

- *"only calls it an outage if that fails too"*: DNS w tej ścieżce nie jest sprawdzany (C).
- Tabela werdyktów, wiersz *"adapter disconnected"*: kod stawia go także przy braku bramy na interfejsie trasy (VPN z pełnym tunelem), ustalenie 10.
- Tabela werdyktów, wiersz *"no | yes | working: the router ignores pings to itself"*: prawda dla monitora, nieprawda dla długiego pomiaru w Diagnostyce (H).
- *"evidence an ISP has to engage with"*: raport pokazuje przyczyny z UPnP jako surowe kody (G) i może wskazać cofniętą zmianę jako przyczynę (B).
- *"Revert restores the exact prior state"*: prawda dla typowego przypadku, z wyjątkiem panic (D).

## 4. Teksty PL i EN

Skrypt porównał 1060 par (wpisy `strings!`, `pick`, ramiona `Lang::En`/`Lang::Pl`). Zgłosił 4 różnice i wszystkie okazały się fałszywymi alarmami (odmiana liczebników przez `pl_form`, "ms" jako fragment słowa). Ręcznie przeczytałem wszystkie tytuły i porady przyczyn ([i18n.rs:3846-4144](../src/i18n.rs#L3846-L4144)) i mówią to samo w obu językach. Jedyna luka w tekstach to brakujące kody z ustalenia G, wspólna dla obu języków. `live_minmax` ma na sztywno "1.1.1.1", ale nic go nie wywołuje.

## 5. Luki funkcjonalne (od najczęstszych u zwykłego użytkownika)

1. Router ograniczający ICMP a długi pomiar: częsta konfiguracja routerów domowych, a skutkiem jest obwinianie własnej sieci (H).
2. Każdy, kto ma UPnP włączone (domyślne w większości routerów) i awarię: surowe kody w Historii i raporcie (G).
3. Tryb gry i świeżo cofnięte zmiany jako fałszywy winowajca (B).
4. VPN z pełnym tunelem (10).
5. Sieci blokujące ICMP z awarią DNS: rzadkie w domu, częstsze w podróży (C).
6. IPv6 i kilka kart naraz: poza zakresem pomiaru, uczciwie opisane w Limitations.

## 6. Najpierw

**Ustalenie G, zaraz potem B.** G jest najtańsze (osiem tekstów i jeden test pilnujący, żeby żaden kod przyczyny nie został bez tytułu), a dotyka każdego z włączonym UPnP dokładnie w raporcie dla dostawcy. B to jeden filtr i test, a psuje ten sam dowód. H jest tak samo ważne, ale zmienia regułę pomiaru, więc wymaga namysłu nad tym, jak liczyć sekundę, w której milczy tylko router.

## Czego ten audyt nie mógł sprawdzić

- **Zachowania przy prawdziwej awarii.** Wymagałoby to zerwania łącza, routera albo DNS na tym komputerze. Wnioski o awariach opieram na kodzie i testach jednostkowych, nie na obserwacji.
- **Zakładek Na żywo i Historia na ekranie.** Aplikacja ignoruje kliknięcia wysyłane w tle, a przejęcia myszy nie robiłem bez zgody. Zrzut objął nagłówek i Ustawienia; nagłówek po polsku mówi wprost "Połączenie sprawne" i nazywa sygnał słowem ("dobry · 72%").
- **Hipotezy A i J** zależą od oprogramowania i sprzętu, których tu nie ma. Do każdej podałem test.
