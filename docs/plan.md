# Spec: tryb gry

## Context
Skoki pingu i przycinanie w LoL / CS2 / Valorancie (problem B) + ciągły podgląd pingu z informacją, czyja to wina (C). Wybrany kierunek z brainstormingu: tryb mieszany. Gra wykrywana sama → nakładka i gęstsze pomiary; zmiany w systemie tylko po kliknięciu, cofane same po wyjściu z gry. Hearthstone poza zakresem (gra turowa, ping bez znaczenia).

## 1. Assumptions
- Gry = procesy `League of Legends.exe`, `cs2.exe`, `VALORANT-Win64-Shipping.exe`; porównanie bez wielkości liter.
- Okno egui nie dostaje klatek, gdy jest schowane (tak samo jak przy ikonie w zasobniku) → nakładka = natywne okno Win32 we własnym wątku, wzorowane na `src/tray.rs`.
- Nakładka przepuszcza kliknięcia (WS_EX_LAYERED|TRANSPARENT|TOPMOST|TOOLWINDOW|NOACTIVATE), nic nie wstrzykuje → bezpieczna dla Vanguard i VAC. Działa nad grą w oknie bez ramki. Nad prawdziwym pełnym ekranem może nie być widoczna (ograniczenie Windows, opisane w README).
- Ping do serwera gry nie jest mierzony (UDP, przekaźniki Valve). Nakładka pokazuje router + internet (istniejące cele monitora) i wskazanie winnego.
- "Przygotuj łącze" tylko jako administrator (decyzja). Bez uprawnień pozycja w menu wyszarzona z dopiskiem.
- Zmiany na czas sesji: `wlan_power_plan` (istniejący tweak, działa od razu) + zatrzymanie usług `wuauserv`, `BITS`, `DoSvc`, jeśli działają (przez `sc.exe` i `optimize::run`). Cofane są tylko te zmiany, które zrobiła sesja: tweak, który już był ustawiony, i usługa, która już stała, są pomijane.
- ? Lista programów zajmujących łącze → **odpada w v1** (per-proces wymaga ETW albo admina dla każdego połączenia TCP, a gry używają UDP). Zamiennik: łączny ruch tego komputera z liczników karty (`GetIfEntry2`, InOctets/OutOctets). Przy > 2 Mbit/s w trakcie gry nakładka mówi "coś na tym komputerze pobiera X Mbit/s", bez wskazywania programu. Oznaczone `ponytail:`.
- ? Pozycja i wygląd nakładki: nie da się tego rozstrzygnąć rozmową. v1: lewy górny róg, 2 linie, potem poprawki po jednym meczu.

## 2. Success criteria
- [x] `cargo clippy --all-targets -- -D warnings` przechodzi
- [x] `cargo test` przechodzi; liczba testów w README zaktualizowana
- [x] test: `is_game("league of legends.exe")` true, `is_game("Hearthstone.exe")` false
- [x] test `blame`: skok routera + internetu → `Local`; tylko internet → `Beyond`; stabilnie → `Clean`; < 10 próbek → `Unknown`
- [x] test: plan sesji pomija tweak już ustawiony i usługę już zatrzymaną; znacznik sesji (JSON) zapisuje się i odczytuje bez strat
- [x] test: monitor w trybie gry używa `min(interwał, 500 ms)`, poza nim interwału z ustawień
- [x] test: przy wzroście liczników o 3 MB w 10 s ruch = 2,4 Mbit/s; przy liczniku, który się cofnął, `None`
- [x] test na żywo (`#[ignore]`): `running_game` z listą `["notepad.exe"]` znajduje uruchomiony Notatnik (zmienione na `explorer.exe`: uruchamianie Notatnika otwiera okno na ekranie)
- [x] ręcznie: gra uruchomiona → nakładka w ciągu ≤ 5 s; gra zamknięta → znika w ciągu ≤ 5 s; nakładka nie przejmuje kliknięć (sprawdzone symulacją `cs2.exe`: pokazana ≤ 3 s, schowana po 3,1 s, WS_EX_TRANSPARENT + NOACTIVATE + TOPMOST)
- [ ] ręcznie (admin): "Przygotuj łącze" → `sc query wuauserv` = STOPPED; zamknięcie gry → RUNNING; tweak wraca do stanu sprzed (do sprawdzenia przez użytkownika)
- [ ] ręcznie: zabicie aplikacji w trakcie sesji → przy następnym starcie (bez gry) usługi i tweak wracają, znacznik usunięty (do sprawdzenia przez użytkownika)
- [ ] ręcznie: bez admina pozycja w menu wyszarzona z dopiskiem (do sprawdzenia przez użytkownika)
- [x] opis Nagle'a w Optymalizacji i w README nie obiecuje zysku w grach UDP (LoL, CS2, Valorant)

## 3. Scope
- `Cargo.toml`: feature `Win32_System_Diagnostics_ToolHelp`
- `src/game.rs` (nowy): lista gier, `running_game`, `blame`, sesja (przygotuj / cofnij / znacznik / sprzątanie po awarii), wątek obserwatora
- `src/overlay.rs` (nowy): okno Win32 nakładki, timer 500 ms, tekst GDI
- `src/main.rs`: `mod`, start obserwatora i nakładki, sprzątanie po awarii przy starcie
- `src/monitor.rs`: `Shared.gaming: AtomicBool`, skrócenie interwału w trybie gry
- `src/probe/netstate.rs`: `LinkCounters` + bajty wysłane i odebrane
- `src/tray.rs`: pozycje menu "Przygotuj łącze do gry" / "Zakończ tryb gry"
- `src/settings.rs`: `game_overlay: bool` (domyślnie true)
- `src/ui/settings_tab.rs`: przełącznik nakładki
- `src/i18n.rs`: teksty EN/PL + poprawka `tw_nagle_why`
- `README.md`: sekcja "Tryb gry", wiersz Nagle w tabeli, liczba testów
- `docs/plan.md`: ten spec

## 4. Simplest approach
Jeden wątek `game` co 3 s sprawdza procesy (ToolHelp) i ustawia `Shared.gaming`. Monitor czyta flagę co pomiar i zagęszcza pomiary. Nakładka to drugi wątek Win32 z tym samym wzorcem co ikona w zasobniku: czyta `Shared.last`, historię `lead` i liczniki karty, a pokazuje się tylko przy `gaming && game_overlay`. "Przygotuj łącze" z menu ikony wywołuje `game::prepare`: istniejące `optimize::apply` dla `wlan_power_plan` i `sc stop` dla usług. Lista tego, co faktycznie zmieniono, trafia do `game_session.json`. Wyjście z gry, "Zakończ" albo start aplikacji z pozostawionym znacznikiem → `game::restore`. Odrzucone: nakładka jako drugie okno egui (nie dostaje klatek, gdy główne jest schowane); QoS i oznaczanie pakietów (domowe routery je ignorują); lista programów per-proces (ETW, osobne uprawnienia, brak UDP); proces pomocniczy z UAC (wybrano "tylko admin").

## 5. Open questions
1. Czy zamiennik listy programów (łączny ruch komputera bez nazwy programu) wystarczy na v1?
2. Zgoda na szkic nakładki w lewym górnym rogu i poprawki wyglądu po jednym meczu?

## Następne (z audytu 2026-09-23, `prompts/audyt-wynik-2026-09-23.md`)

Zrobione w tej sesji: cofanie DNS wraca do DHCP, awaria przepisywana dopiero po serii odczytów, test TCP gdy pingi milkną, liczniki błędów kabla w Diagnozie.

- [x] Nieudane zastosowanie zmiany zapisywać jako `apply_failed`; `after_tweak` bierze tylko udane (audyt #4). Także `revert_failed` w zakładce Optymalizacja; test w `cause.rs`. Tryb gry (`game.rs`) też zapisuje `revert_failed` (audyt #13)
- [x] Jeden zestaw progów jakości dla Live i Diagnozy (audyt #5). Jitter: Live nie podwaja już `jitter_ok_ms`, test w `monitor.rs`. Uwaga: ping ma ten sam próg, ale Live bierze najszybszy cel, a Diagnoza średnią jednej kotwicy (inna statystyka, nie inny próg)
- [x] "Szybki DNS" nie jest "do zmiany" przy własnym prywatnym resolverze (Pi-hole) (audyt #6). `NetState::dns_is_own_resolver`: prywatny DNS inny niż brama; Optymalizacja, Diagnoza i przyczyny bez naprawy `fast_dns`; testy w `netstate.rs` i `cause.rs`
- [x] Pomiar skutku zmiany: ping/jitter/straty 24 h przed i po, w wierszu zmiany (audyt #7). `effect.rs` + `Store::stats_between`, panel zmiany w Optymalizacji, okno „po” kończy cofnięcie; 3 testy w `effect.rs`. Nie oglądane w działającej aplikacji
- [x] Bufferbloat przy wysyłaniu + ostrzeżenie, gdy test nie nasycił łącza (audyt #8). Druga faza POST do `speed.cloudflare.com/__up`, ocena z gorszego kierunku, przy A/B dopisek z prędkościami pomiaru; testy w `bandwidth.rs`. Endpoint sprawdzony jednym chunked POST (HTTP 200); pełnego testu w aplikacji nie uruchamiano (koszt transferu)
- [x] Błąd zapisu próbek do bazy jako "nie mierzę", nie zielony werdykt (audyt #9). Nowy `Seen::Unrecorded` (szara ikona, nagłówek „nie może zapisywać”), tylko zamiast werdyktu Ok; test w `tray.rs`. Sprawdzone testami, nie oglądane w działającej aplikacji
- [x] Ostatni widoczny hop obwiniany tylko, gdy jest celem trasy (audyt #11, `Path.dest`, test w `path.rs`)
- [x] Model kanałów 5/6 GHz liczy cały blok 80 MHz sąsiada za pół (audyt #12, test w `airscan.rs`)
- [x] UPnP IGD: czas działania routera, stan WAN, publiczny IP w kontekście awarii (restart routera vs rozłączenie przez dostawcę). `probe/igd.rs`, wątek `run_router` co 30 s, tabela `router`, reguły `router_wan_down` / `router_restarted` / `wan_new_ip` / `router_wan_up`. Odczyt sprawdzony na żywo (test `#[ignore]` `the_live_router_answers`: Connected, uptime ~19 dni); reguły tylko na danych syntetycznych, bez prawdziwej awarii
- [ ] IPv6: czy jest adres i trasa, czy połączenie przez v6 dochodzi (wolne pierwsze ładowanie)
- [ ] BSS Load z beaconów: zajętość kanału i liczba stacji wg punktu dostępowego
- [ ] Sprawdzić na żywo: VPN bez bramy daje `AdapterDown` (audyt #10)
