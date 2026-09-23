# Spec: ikona w zasobniku + systemowe powiadomienia o awariach

## Context
Aplikacja chodzi w tle z autostartu (`--minimised`, okno `with_visible(false)`). Powiadomienia o awarii to dziś dymki **w oknie** (`ui/mod.rs` `announcement` → `toast`), więc w tym trybie są niewidoczne; do tego odpalają przy każdej zmianie statusu, a historia dopiero po 3 złych pomiarach. Ukrytego okna nie da się przywołać inaczej niż ponownym uruchomieniem exe. Cel: stan widoczny zawsze (ikona), awarie zgłaszane przez Windows, tą samą miarą co historia.

## 1. Assumptions
- Win32 bezpośrednio przez crate `windows` (już jest `Win32_UI_Shell`, `WindowsAndMessaging`), bez nowego crate'a.
- Powiadomienie = balon `NIF_INFO` z `Shell_NotifyIconW`; Windows 10/11 pokazuje go jako toast. Bez WinRT, bez AppUserModelID.
- Ikony 16×16 generowane w kodzie (`CreateIcon`, kółko w kolorze stanu), bez plików zasobów.
- Tray na własnym wątku z ukrytym oknem i własną pętlą komunikatów, niezależny od winit. Czyta `monitor::Shared.last` (już `Arc`), więc działa, gdy okno eframe jest ukryte i nie rysuje.
- Otwieranie okna z ikony: reużycie `single::raise_existing_window()` (Win32 `SW_RESTORE`, działa na ukrytym oknie; sprawdzone w testach restartu).
- eframe obsługuje WM_CLOSE przy ukrytym oknie (zmierzone wcześniej: `taskkill` → wyjście 0,2 s).
- Decyzje użytkownika: X chowa do zasobnika; powiadomienia tylko o twardej awarii i powrocie.
- ? Powiadomienie „padło” wysyłane przy pierwszym pomiarze, gdy awaria jest otwarta (`Outages` → `Step::Open` lub już otwarta) **i** status ≠ `Degraded`; jedno na awarię. „Wróciło” tylko, jeśli poszło „padło”, i tylko przy zamknięciu z obserwowanym powrotem (nie przy śnie/luce).

## 2. Success criteria
- [x] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` przechodzą (237 zaliczonych, 4 `#[ignore]`)
- [x] Test: mapowanie `Status` → kolor ikony (`tray::the_icon_colour_follows_the_verdict`); dodatkowo szary przy pauzie i przed pierwszym pomiarem (`a_paused_or_unstarted_monitor_is_grey_not_green`)
- [x] Test: logika powiadomień, 6 testów w `monitor` (`a_hard_outage_is_announced_once...`, `a_blip_shorter...`, `a_slow_line...`, `an_outage_that_turns_hard...`, `the_setting_silences_both_ends`, `an_outage_lost_in_a_hole...`)
- [x] Na żywo (release, `--minimised`): okno tray istnieje i należy do procesu, `NIM_MODIFY` na ikonie przechodzi (`tray_check.py`)
- [x] Na żywo: `WM_LBUTTONUP` do okna tray → główne okno widoczne
- [x] Na żywo: WM_CLOSE do głównego okna → proces żyje, okno niewidoczne; drugi cykl pokaż/schowaj też działa
- [x] Na żywo: „Zakończ” → wyjście w 0,29 s, kod 0, mutex zwolniony, ikona usunięta
- [x] Na żywo: restart po aktualizacji PASS (`restart_check.py`, stara kopia zamykana przez „Zakończ”)
- [x] `TaskbarCreated` → ikona usunięta z zewnątrz wraca
- [x] README: punkty w „Running in the background” + liczba testów

## Wyniki poza planem (znalezione przy weryfikacji na żywo)
- `--minimised` **nigdy nie ukrywało okna**: eframe 0.29 po pierwszej klatce woła `set_visible(true)` bezwarunkowo (`epi_integration::post_rendering`). Potwierdzone na buildzie sprzed zmian. Naprawa: `ViewportCommand::Visible(false)` w pierwszej klatce (wykonywane po tym pokazaniu).
- Szukanie okna aplikacji po „ma tytuł” trafiało w okna pomocnicze tego samego procesu (`Default IME`, `__wglDummyWindowFodder`). Dotyczyło też istniejącego `single::raise_existing_window`. Naprawa: `single::TITLE_PREFIX` + `is_app_window`.
- Ukryte okno eframe nie dostaje klatek, więc WM_CLOSE do niego wisiał. „Zakończ” pokazuje je najpierw zminimalizowane bez aktywacji (`SW_SHOWMINNOACTIVE`); sprawdzone eksperymentalnie obok wariantu z przezroczystym oknem.
- winit ukrywa okno tylko przy zmianie własnej flagi, a zasobnik pokazuje je przez Win32. Chowanie wysyła więc `Visible(true)` i `Visible(false)`.
- „Uruchom jako administrator” (pytanie otwarte 1): przechodzi przez normalne wyjście i `--after-update`.

## Domknięte po review (bez słabych stron)
- [x] Powiadomienia na prawdziwej awarii: `tray::live_tests::a_real_outage_is_announced_and_so_is_its_end` (Wi-Fi odcięte `netsh wlan disconnect`, osobna baza). Wynik: „padło” po 4,0 s, „wróciło” 6,6 s po powrocie łącza, zapisana awaria 9,4 s przy odcięciu 6,4 s.
- [x] Windows przyjmuje i pokazuje balon (`NIN_BALLOONSHOW`): `a_notice_reaches_windows_as_a_notification`. Uwaga: stare balony z zasobnika nie trafiają do `wpndatabase.db`, więc brak wpisu tam niczego nie dowodzi.
- [x] Błąd znaleziony na żywo: po twardej awarii straty z niej samej trzymały łącze w stanie „wolno” do 60 s, więc każda awaria trwała w historii o ok. minutę za długo, a „wróciło” spóźniało się o minutę. Okno jakości liczone od końca ostatniej twardej awarii, minimum 10 pomiarów do oceny strat. Testy: `the_outage_s_own_lost_pings_do_not_keep_the_line_degraded`, `too_few_readings_are_not_judged_for_loss`.
- [x] Start `--minimised` bez mignięcia: okno powstaje poza ekranem (`ui::OFF_SCREEN`), jest chowane i wraca na środek w pierwszej klatce. Pomiar co 2 ms od startu: 0 próbek widocznych na ekranie w 3 uruchomieniach (wcześniej 22–38 ms).
- [x] Ikona po panice: hook paniki usuwa ją (`tray::remove_after_crash`); test `the_icon_goes_away_on_a_crash_on_close_and_can_come_back` (także drugi zasobnik w tym samym procesie, wcześniej `RegisterClassW` odmawiał).
- Pozostaje z założenia: przy „Zakończ” ze schowanego okna przez ok. 0,3 s widać przycisk na pasku zadań (okno pokazane zminimalizowane, żeby eframe dostał klatkę na zamknięcie).

## 3. Scope
- **nowy** `src/tray.rs`: wątek, okno ukryte, ikony, tooltip (headline + note), menu (Otwórz / Zakończ), balony, `TaskbarCreated`, `NIM_DELETE` przy wyjściu
- `src/monitor.rs`: kanał `Notice { Down{status, note}, Up{secs} }` z `run_loop`; logika w `Outages` lub obok niej (testowalna)
- `src/ui/mod.rs`: start tray w `App::new`; przechwycenie `close_requested` → `CancelClose` + `Visible(false)`, chyba że flaga wyjścia; usunięcie `announcement`/toastów statusu (+ ich 3 testy zastąpione nowymi)
- `src/ui/update_ui.rs`: restart ustawia flagę wyjścia przed `Close`
- `src/main.rs`: `mod tray;`
- `src/i18n.rs`: napisy menu, tooltipu, balonów
- `Cargo.toml`: cechy `Win32_System_LibraryLoader` (GetModuleHandleW), ew. `Win32_Graphics_Gdi`
- `README.md`, `docs/plan.md`

## 4. Simplest approach
Jeden wątek `netdoctor-tray` z message-only oknem (`HWND_MESSAGE` nie dostaje `TaskbarCreated`, więc zwykłe ukryte okno), `Shell_NotifyIconW` do ikony i balonów, `SetTimer` 1 s do odświeżenia koloru/tooltipu z `Shared.last` i odbioru `Notice` z kanału. Flaga wyjścia `AtomicBool` wspólna dla tray i UI; „Zakończ” = flaga + WM_CLOSE do głównego okna. Odrzucone: crate `tray-icon` (nowa zależność z własną pętlą zdarzeń, konflikt z winit), toasty WinRT (wymagają AUMID i skrótu w Menu Start), powiadomienia z wątku UI (nie działa przy ukrytym oknie, co jest właśnie problemem).

## 5. Open questions
1. (rozstrzygnięte: tak, w zakresie) „Uruchom ponownie jako administrator” kończy przez `process::exit(0)`: ma ten sam wyścig o mutex co naprawiony restart aktualizacji, a z ikoną zostawi też „ducha” w zasobniku (brak `NIM_DELETE`). Włączyć poprawkę (flaga `--after-update` + normalne wyjście) w zakres?

## Następne (z audytu 2026-09-23, `prompts/audyt-wynik-2026-09-23.md`)

Zrobione w tej sesji: cofanie DNS wraca do DHCP, awaria przepisywana dopiero po serii odczytów, test TCP gdy pingi milkną, liczniki błędów kabla w Diagnozie.

- [ ] Nieudane zastosowanie zmiany zapisywać jako `apply_failed`; `after_tweak` bierze tylko udane (audyt #4)
- [ ] Jeden zestaw progów jakości dla Live i Diagnozy (audyt #5)
- [ ] "Szybki DNS" nie jest "do zmiany" przy własnym prywatnym resolverze (Pi-hole) (audyt #6)
- [ ] Pomiar skutku zmiany: ping/jitter/straty 24 h przed i po, w wierszu zmiany (audyt #7)
- [ ] Bufferbloat przy wysyłaniu + ostrzeżenie, gdy test nie nasycił łącza (audyt #8)
- [ ] Błąd zapisu próbek do bazy jako "nie mierzę", nie zielony werdykt (audyt #9)
- [ ] UPnP IGD: czas działania routera, stan WAN, publiczny IP w kontekście awarii (restart routera vs rozłączenie przez dostawcę)
- [ ] IPv6: czy jest adres i trasa, czy połączenie przez v6 dochodzi (wolne pierwsze ładowanie)
- [ ] BSS Load z beaconów: zajętość kanału i liczba stacji wg punktu dostępowego
- [ ] Sprawdzić na żywo: VPN bez bramy daje `AdapterDown` (audyt #10)
