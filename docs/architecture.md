# netdoc — Architecture

## Structure

Jedna binarka, trzy wątki, które się nie przeplatają: monitor mierzy, UI rysuje,
zadania długie (skan, odczyt tweaków, test obciążenia, aktualizacja) idą na wątki
robocze i wracają kanałem `Job`. Wszystkie trzy spotykają się wyłącznie na
`Store`, i to jest jedyne miejsce, gdzie trzeba myśleć o współbieżności.

**Pomiar i zapis**

| Moduł | Za co odpowiada |
|---|---|
| `monitor.rs` | wątek zamiatania: `PingerPool` pinguje wszystkie cele naraz, wykrywa początek i koniec awarii, pisze próbki i zdarzenia |
| `store.rs` | SQLite (WAL): próbki, awarie z kontekstem, dziennik tweaków. Osobne połączenie do zapisu i drugie `query_only` do odczytu, żeby UI nie stał w kolejce przed monitorem |
| `probe/icmp.rs` | uchwyt ICMP (`Pinger`, `Send` ale nie `Sync`) i pula uchwytów |
| `probe/netstate.rs` | która karta jest tą właściwą (`GetBestInterface`), jej adres, brama i stan Wi-Fi przez FFI do WLAN |
| `probe/path.rs` | traceroute i to, na którym skoku zaczyna się strata |
| `probe/eventlog.rs` | co Windows sam zapisał o zerwaniu, przez `wevtutil` |
| `probe/airscan.rs` | skan otoczenia Wi-Fi i rekomendacja kanału |

**Wnioskowanie**

| Moduł | Za co odpowiada |
|---|---|
| `cause.rs` | z zapisanej awarii na nazwaną przyczynę i naprawę, z poziomem pewności |
| `diagnose.rs` | skan jednorazowy: dziewięć kontroli, werdykt na górze, `Finding` pod nim |
| `bandwidth.rs` | test obciążenia i ocena bufferbloatu |

**Zmienianie systemu**

| Moduł | Za co odpowiada |
|---|---|
| `optimize/mod.rs` | trait `Tweak`, snapshoty stanu pierwotnego (klucz `scope_key`, czyli per karta tam, gdzie to ma znaczenie), `system_tool` |
| `optimize/stack.rs` | stos TCP, resolver i funkcje Windowsa, które same używają łącza |
| `optimize/radio.rs` | to, co siedzi na zakładce Zaawansowane sterownika karty |
| `winreg.rs` | rejestr, z rozróżnieniem „wartości nie ma" od „nie da się odczytać" |
| `autostart.rs` | wpis w `HKCU\...\Run` |

**Reszta**

| Moduł | Za co odpowiada |
|---|---|
| `ui/` | egui: `mod.rs` to `App` i pętla klatki, reszta to po jednej zakładce na plik |
| `settings.rs` | ustawienia w JSON, migracja starych kluczy, `data_dir()` |
| `single.rs` | nazwany mutex i wyniesienie okna pierwszej instancji na wierzch |
| `tray.rs` | ikona w zasobniku na własnym wątku Win32: kolor stanu, menu, powiadomienia Windows o awariach |
| `update.rs` | wydania z GitHuba, weryfikacja przez `SHA256SUMS`, podmiana działającej binarki |
| `i18n.rs` | dwa języki, jeden wpis na komunikat |

**Zasada, która trzyma to razem.** Wszystko, co dotyka bazy, sieci albo blokady
dzielonej z monitorem, jest liczone w rytmie pomiaru i trzymane w pamięci
podręcznej — nigdy w rytmie klatki. Trzy osobne pozycje audytu wzięły się
z łamania dokładnie tej zasady.

## Decisions
<!-- Append: date — decision — why -->

- 2026-09-20 — `rustfmt.toml` z `use_small_heuristics = "Max"`, a całe repo przeformatowane jednym commitem (`c5780a6`) wpisanym do `.git-blame-ignore-revs` — kod był formatowany ręcznie i nie odpowiadał żadnej konfiguracji rustfmt, więc reguła „cargo fmt przed każdym commitem" z CLAUDE.md produkowała diff po kilkunastu nietkniętych modułach. Konfiguracja „Max" rusza 853 linie zamiast 3271 przy domyślnej, bo odpowiada temu, jak ten kod był pisany. Od teraz `cargo fmt --check` przechodzi czysto i reguła jest bezpieczna do wykonania.

- 2026-09-20 — dane zakładki Na żywo liczone raz na zamiatanie, nie raz na klatkę: zakres celu wędruje z `Settings::targets()` do pola `ChartSeries` w `ChartCache`, a kafelki mają własny `CardCache` o żywotności jednej sekundy — `Settings::targets()` rozwiązuje przez DNS każdy host dodany przez użytkownika, a kafelki robiły trzy zapytania do SQLite; jedno i drugie działo się w wątku rysowania, przy kursorze nad wykresem nawet 60 razy na sekundę. Zasada: wszystko, co dotyka bazy, sieci albo blokady współdzielonej z wątkiem monitora, ma trafiać do pamięci podręcznej odświeżanej w rytmie pomiaru.

- 2026-09-20 — czasy dymków ustawiane na `Context`, nie na `Ui` — `Response` czyta `tooltip_delay`, `tooltip_grace_time` i `show_tooltips_only_when_still` wyłącznie z `ctx.style()`, więc te same pola ustawione na `Ui` są po cichu ignorowane. Ustawienie żyje w `apply_theme`.

- 2026-09-20 — dystrybucja przez GitHub Releases, a podmiana binarki przez przemianowanie działającego pliku (`netdoctor.exe` → `.old`, nowy na jego miejsce, restart, kasowanie `.old` przy następnym starcie) — Windows nie pozwala nadpisać ani skasować uruchomionego obrazu, ale pozwala go przemianować, i to jedyny sposób na samoaktualizację bez osobnego programu instalującego. Konsekwencja: aplikacja musi leżeć w katalogu zapisywalnym dla użytkownika, więc `Program Files` jest odradzane w README, a `install()` sprawdza zapisywalność przed pobraniem, nie po. Wybrany goły `.exe` zamiast MSI/NSIS właśnie dlatego, że instalator wymusiłby UAC przy każdej aktualizacji.

- 2026-09-20 — `ureq` awansowany z zależności używanej tylko przez test obciążeniowy na transport aktualizacji, a odpowiedź API GitHuba parsowana przez `serde_json::from_str`, nie przez `into_json` — `into_json` siedzi za domyślną cechą `ureq`, której `Cargo.toml` nie wymienia z nazwy, więc opieranie się na niej to cicha zależność od tego, że nikt nie doda `default-features = false`. Pobieranie idzie przez `AgentBuilder` z `timeout_read`, bo `timeout` na żądaniu ogranicza tylko dotarcie do odpowiedzi, a nie odczyt kilku megabajtów ciała.

- 2026-09-21 — weryfikacja pobranego pliku jest kształtowa **i** sumą kontrolną: nagłówek `MZ`, minimum 1 MiB, zgodność rozmiaru z API, a na końcu porównanie SHA-256 z `SHA256SUMS`, które workflow wiesza obok binarki. Wcześniej ten plik był publikowany i nieczytany. Model zaufania się przez to nie zmienił — binarka i suma przychodzą z tego samego miejsca, więc kto potrafi podmienić jedno, podmieni i drugie; zamknięte są przypadki losowe (ucięty strumień, podmieniony asset, uszkodzone wydanie), a nie przejęte konto. Podpisywanie wydań pozostaje właściwą odpowiedzią i kosztuje certyfikat. Hash liczony przez `sha2`, nie przez BCrypt z crate'a `windows`: pięćdziesiąt linii unsafe FFI za skrót to zły kurs w repo, które ma 42 bloki `unsafe` i plan, żeby ich pilnować ostrzej.

- 2026-09-20 — toolchain w workflow przypięty (`dtolnay/rust-toolchain@1.95.0`) zamiast `stable` — pierwszy przebieg wydania padł, bo clippy z nowszego kanału odrzucił kod, który maszyna deweloperska akceptuje. Wydanie, które potrafi paść w dniu, w którym nikt nie dotknął projektu, nie jest procesem wydawniczym. Podbicie tej wersji ma być świadomą zmianą. Workflow odmawia też publikacji tagu niezgodnego z `version` w `Cargo.toml`, bo updater porównuje właśnie tag z wersją wkompilowaną w binarkę.

- 2026-09-20 — ciągłość obserwacji jest stanem wątku monitora, a baza odpowiada na to pytanie tylko raz, przy starcie — `Store::observing_since` skanuje dobę unikalnych znaczników sweepu przez `LAG` i zwraca początek ostatniego nieprzerwanego odcinka, a dalej pętla wykrywa własne luki (pauza, uśpienie maszyny), bo jest jedynym pisarzem próbek. Wynik jedzie w `Snapshot.observed_from`, więc UI nie dotyka SQLite przy rysowaniu. Próg `OBSERVATION_GAP_S = 60 s` leży w `store.rs` i jest stały: przy interwale sondowania dłuższym niż minuta każdy normalny sweep wyglądałby jak przerwa, więc gdyby ustawienia kiedyś na taki interwał pozwoliły, próg musi być jego wielokrotnością. Zasada ogólna: brak zapisanych zdarzeń nigdy nie jest dowodem na ich nieobecność, jeśli nie wiadomo, jak długo trwał zapis.

- 2026-09-23 — nic, co blokuje na DNS, nie działa w wątku zamiatania. Test DNS (`run_dns`, co 10 s) i rozwiązywanie hostów dodanych przez użytkownika (`run_hosts`, co 5 min, co 30 s, gdy któregoś nie da się rozwiązać) mają własne wątki i publikują wynik w `Shared`; zamiatanie tylko go czyta. `getaddrinfo` przy martwym resolverze blokuje do ~12 s, czyli wcześniej pomiar zwalniał dokładnie w awarii DNS, a host podany nazwą znikał z celów. Zapytanie wiszące dłużej niż `DNS_STALL = 5 s` jest raportowane jako awaria, nie jako ostatni dobry wynik. Host, którego chwilowo nie da się rozwiązać, zachowuje ostatni adres. `Settings::targets()` nadal blokuje i jest tylko dla UI; monitor używa `targets_with` z pamięcią podręczną.

- 2026-09-23 — `Drop` monitora czeka wyłącznie na wątek zamiatania (on zamyka otwartą awarię). Wątki ścieżki, DNS i hostów są odłączone: dotykają tylko `Shared`, a czekanie na nie mogło trwać do 24 × timeout pinga. Czas zamknięcia ma znaczenie, bo nowa kopia po aktualizacji startuje dopiero, gdy stara zwolni mutex pojedynczej instancji (`--after-update` + `single::acquire_within`, do 60 s). Zmierzone: zamknięcie 0,2–0,4 s.

- 2026-09-23 — pauza monitora to dwie rzeczy: pauza użytkownika (`set_paused`) i licznik blokad zadań (`hold`/`release`: skan głęboki, test obciążenia, skan Wi-Fi). Jedna flaga pozwalała pierwszemu kończącemu się zadaniu odpauzować monitor pod drugim albo wbrew użytkownikowi.

- 2026-09-23 — reguły otwierania i zamykania awarii wydzielone do `monitor::Outages`, bez I/O, żeby dało się je testować. Dziura w obserwacji dłuższa niż `OBSERVATION_GAP_S` (sen, długa pauza) zamyka otwartą awarię na ostatnim pomiarze przed nią i czyści `lead`, bo pomiary sprzed snu nie są wstępem do niczego po nim. Krótsza pauza jest mostkowana: awaria wciąż trwająca po niej to ta sama awaria, a sekundy bez pomiaru trafiają do `context_end` jako `unwatched_s`; awaria, która minęła w trakcie pauzy, kończy się na ostatnim pomiarze przed pauzą. Monitor zawsze zamyka przez `Store::close_event_at` z jawnym czasem; „teraz" jest błędne po każdej pauzie.

- 2026-09-23 — ikona w zasobniku na własnym wątku (`tray.rs`) z ukrytym oknem Win32 i własną pętlą komunikatów, bez crate'a `tray-icon`; czyta `monitor::Shared` bezpośrednio, a powiadomienia dostaje kanałem `Monitor::notices`. Powód: okno eframe jest przez większość czasu ukryte, a ukryte okno eframe nie dostaje klatek, więc nic, co żyje w pętli UI, nie może być tym, co widać, gdy okna nie widać. Powiadomienia to balony `Shell_NotifyIconW` (`NIF_INFO`), nie toasty WinRT, bo te wymagają AUMID i skrótu w Menu Start. Reguła „kiedy powiadomić” jest w `monitor::Announcer`, obok `Outages`, żeby powiadomienia i historia liczyły awarię tą samą miarą.

- 2026-09-23 — trzy zachowania eframe 0.29/winit, na których opiera się chowanie okna, każde potwierdzone na żywym procesie: (1) eframe pokazuje okno po pierwszej klatce bez względu na `with_visible(false)`, a polecenia widoku wykonuje po tym pokazaniu; start zminimalizowany ustawia okno poza ekranem i chowa je w pierwszej klatce. (2) Ukryte okno nie dostaje klatek, więc WM_CLOSE do niego wisi; „Zakończ” pokazuje je najpierw `SW_SHOWMINNOACTIVE`. (3) winit wywołuje `ShowWindow` tylko przy zmianie własnej flagi, a zasobnik pokazuje okno przez Win32; chowanie wysyła `Visible(true)` i `Visible(false)`. Okno aplikacji rozpoznawane po prefiksie tytułu (`single::TITLE_PREFIX`), bo proces ma też okna IME i sterownika GL z tytułami. Przy podbiciu eframe każde z tych trzech trzeba sprawdzić na nowo (`tray_check`).

- 2026-09-23 — straty i jitter oceniane od końca ostatniej twardej awarii, nie z pełnych 60 s, i dopiero przy 10 pomiarach. Pełne okno liczyło zgubione pingi samej awarii jako „wolne łącze” i przedłużało każdą awarię o ok. minutę; znalezione testem z prawdziwym odcięciem Wi-Fi.

- 2026-09-23 — nieudany pomiar to osobny stan, a nie werdykt. Przebieg, w którym nie dało się wysłać ani jednego pinga, nie ma `Status`: `Snapshot.blind` niesie przyczynę, pętla traktuje go jak pauzę (bez próbki, bez kroku `Outages`, bez powiadomienia), a nagłówek i zasobnik pokazują go na szaro. Wybrane zamiast nowego wariantu `Status`, bo ten trafiałby do historii; koszt: każdy, kto pokazuje werdykt, musi najpierw sprawdzić `blind` (dziś `ui::verdict_line` i `tray::Seen`). W skanie to samo robią `Wire.blind` i `Segment::Unmeasured`. Zasobnik ocenia też wiek migawki (`Seen::Stale`). Interwał sondowania ma teraz sufit `MAX_PROBE_INTERVAL_MS`, połowę `OBSERVATION_GAP_S`, co zamyka zastrzeżenie z wpisu o ciągłości obserwacji.

- 2026-09-23 — test DNS pyta resolvery karty bezpośrednio (zapytanie A po UDP, bez nowych zależności: `netstate::resolvers_answer`), a nie `getaddrinfo`. Klient DNS Windows odpowiada z pamięci podręcznej przez cały TTL wpisu i trzyma porażki w pamięci negatywnej, więc `getaddrinfo` mierzył pamięć, a nie resolver. Koszt: sieć, która blokuje UDP 53 przy działającym DoH, pokaże awarię DNS, której użytkownik nie odczuwa; bez znanego resolvera zostaje `getaddrinfo`. Awaria DNS wymaga `DNS_FAILS_TO_REPORT = 2` porażek z rzędu, a po porażce powtórka jest po `DNS_RETRY = 2 s`.

- 2026-09-23 — werdykt „odcinek padł” wymaga zgodności pomiarów. „Internet odpowiada” znaczy: odpowiedział adres publiczny (`diagnose::is_public`; prywatne, APIPA i CGNAT się nie liczą, CGNAT to sieć dostawcy jak w `find_edge`). Skan ma dwie kotwice (1.1.1.1 i 8.8.8.8, ping i TCP), a monitor ocenia straty, jitter i opóźnienie z najlepszego publicznego celu, bo to, co robi łącze, widać na wszystkich celach naraz. Rodzaj awarii w historii to najpoważniejszy stan, jaki osiągnęła (`Recorded` + `Store::set_event_kind`), nie stan z chwili otwarcia.

- 2026-09-23 — skutek zmiany ustawień liczony z istniejących tabel, bez nowego schematu: `effect::of` bierze ostatnie udane `apply` z dziennika zmian i porównuje próbki z doby przed z czasem po (do cofnięcia albo doby), każdą miarę z lepszej kotwicy, jak nagłówek. Liczone w wątku, który czyta stany zmian (`refresh_tweaks`), a nie per klatka. Świadomie bez werdyktu „pomogło”: pora dnia i inne zmiany w tym samym oknie przesuwają liczby, więc aplikacja pokazuje pomiar i zastrzeżenie. Nieudany zapis próbek to `Seen::Unrecorded`, osobny od `Blind`, bo pingi poszły i awaria z nich wykryta nadal jest awarią; zamienia tylko werdykt Ok.

- 2026-09-23 — router pytany o własne łącze przez UPnP IGD (bez nowych zależności: SSDP na `UdpSocket`, SOAP przez ureq, XML czytany wyszukiwaniem tekstu). Nowa tabela `router` (ts, status, uptime_s, ip_tag), przycinana razem z próbkami. Odczyty są dowodem dla analizy awarii, nie wchodzą do werdyktu na żywo. Adres publiczny zapisywany tylko jako odcisk FNV-1a. Opis i adres sterujący przyjmowane tylko z hosta bramy, bo router odpytywany jest co 30 s pod adresem, który sam podał. Licznik `NewUptime` na wielu routerach (miniupnpd) startuje od nowa także przy ponownym połączeniu WAN, więc reguła mówi „router albo jego połączenie się zrestartowało”, nie „router się zrestartował”.

- 2026-09-24 — pierwsze zewnętrzne API w działającej aplikacji: opcjonalne wyjaśnienie AI przez OpenRouter (`ai.rs`, zapytanie OpenAI-compatible przez istniejące ureq + serde_json, bez nowych zależności). Wybrane zamiast TypeSafe/Jev, bo Jev zwraca typowane wybory, a potrzebny był tekst wyjaśnienia; wybór segmentu robi już deterministycznie `judge`. Zasady: wyłączone bez klucza użytkownika (`Settings::ai_key` albo `OPENROUTER_API_KEY`), wysyłane tylko po kliknięciu, SSID, BSSID i publiczne adresy poza kotwicami maskowane (`ai::redact`, na stanie sieci z chwili skanu `App::scan_net`), odpowiedź oznaczona jako interpretacja, werdykt z pomiarów nadrzędny, odpowiedź po ponownym skanie odrzucana (`scan_gen`). Klucz jawnym tekstem w `settings.json` (oznaczone `ponytail:`, ścieżka: DPAPI). `Scan` niesie teraz `Measurements`, a `diagnose::chain` zamienia je na trzy ogniwa ze stanem `Measured / Silent / Filtered / Unknown / NotMeasured`; UI i raport dla AI czytają ten sam model, więc nie mogą się rozjechać w tym, co uznają za zmierzone.

## Open questions
<!-- Track unresolved technical decisions -->
