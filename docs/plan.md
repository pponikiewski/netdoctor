# netdoc — plan naprawczy

> Źródło: pełny przegląd repo z 2026-09-20. Każda teza jest poparta pomiarem
> albo testem, nie szacunkiem. Stan wyjściowy: `cargo clippy -- -D warnings`
> czysto, `cargo test` 178/178 w 1,4 s.
>
> Kolejność to (wpływ × taniość), nie kolejność odkrycia. Odhaczamy po kolei.

## Backlog

- [x] **1. Literówka `wevtutil.exe` zabija całą integrację z dziennikiem Windows**
- [x] **2. Kwerendy listy zdarzeń ciągną 33 KB kontekstu na wiersz**
- [x] **3. `refresh_tweaks` blokuje wątek UI na 576 ms**
- [x] **4. `notify_on_outage` to martwy przełącznik**
- [x] **5. Znaczniki awarii znikają przy interwale sondowania ≥ 2 s**
- [x] **6. `DwordTweak::read` gubi rozróżnienie „brak wartości" od „brak dostępu"**
- [x] **7. Snapshoty tweaków nie rozróżniają kart sieciowych**
- [x] **8. NetState zamarza na czas awarii; brak wykrywania APIPA/DHCP**
- [x] **9. Zamiatanie trwa 3,6 s, gdy cele nie odpowiadają**
- [x] **10. Druga, tylko-do-odczytu `Connection` dla UI**
- [ ] **11. Updater nie czyta `SHA256SUMS`, które sam publikuje**
- [ ] **12. Brak strażnika pojedynczej instancji**
- [ ] **13. Higiena repo (binarka w gicie, `__pycache__`, `legacy-python`, CI)**
- [ ] **14. Niewypełnione placeholdery w trzech dokumentach**

---

## 1. Literówka `wevtutil.exe` zabija całą integrację z dziennikiem Windows

**Objaw.** Funkcja opisana w nagłówku modułu jako „najmocniejszy dostępny na
Windowsie materiał dowodowy" nie zwróciła nigdy ani jednej linii, na żadnej
maszynie.

**Miejsce.** [eventlog.rs:123](../src/probe/eventlog.rs#L123) woła
`crate::optimize::run("wevtutil.exe", &args)`, a
[`system_tool`](../src/optimize/mod.rs#L289) dokleja rozszerzenie:

```rust
dir.join(format!("{program}.exe"))   // -> C:\Windows\System32\wevtutil.exe.exe
```

**Dowód.**

```text
run("wevtutil.exe") FAILED: Nie można odnaleźć określonego pliku. (os error 2)
run("wevtutil")     OK, 1098 bytes
read_channel(System, last hour) returned 0 events
```

Pozostałe trzy wywołania (`netsh` ×5, `ipconfig` ×3, `powercfg` ×1) są poprawne.

**Zasięg.** 13 kodów przyczyn nieosiągalnych w produkcji: `log_sleep`,
`log_resume`, `log_driver_fault`, `log_wlan_auth`, `log_dhcp`,
`log_duplicate_ip`, `log_link_down`, `log_clean_isp`, `log_wlan_inactivity`,
`log_wlan_ap_rejected`, `log_wlan_deauth` i dwa warianty `WlanDisconnect`. To
akurat te o `Confidence::Certain`, czyli najmocniejsze w całej aplikacji. README
poświęca im własny rozdział („What Windows already wrote down") z przykładem
reason code 4.

**Dlaczego awaria jest cicha, a nie kłamliwa.** Strażnik `!log.is_empty()` w
[cause.rs:347](../src/cause.rs#L347) blokuje regułę „dziennik milczy, więc to
dostawca". Celowa ostrożność, która się opłaciła: aplikacja nie twierdzi niczego
na podstawie dziennika, którego nie przeczytała.

**Dlaczego 178 testów tego nie złapało.** Testy `eventlog` parsują XML z
fixture'ów i nigdy nie wołają `read_channel`. Nazwa narzędzia to string, którego
nic nie weryfikuje.

**Naprawa.** Usunąć `.exe` z wywołania. Dodatkowo `system_tool` powinien obciąć
istniejący sufiks `.exe`, żeby oba zapisy działały.

**Kryterium akceptacji.**
- [x] Test, który dla każdego wywołania `run()` w repo sprawdza, że
      `system_tool(nazwa)` wskazuje na istniejący plik.
      (`optimize::tests::every_console_tool_we_name_resolves_to_a_real_file`;
      skaner źródeł plus ręczna lista dla `run(prog, args)` w `StackReset`.
      Mutacja kontrolna: podmiana nazwy na `wevtutyl` wywala test.)
- [x] Test integracyjny odpalający prawdziwe `wevtutil` z kwerendą zbudowaną
      przez produkcyjne `channel_args`
      (`probe::eventlog::tests::wevtutil_answers_the_query_read_channel_sends`).
      **Zmiana wobec pierwotnego brzmienia:** kryterium mówiło „`read_channel`
      zwraca więcej niż zero zdarzeń w ostatniej godzinie”. Pomiar pokazał, że
      to nie jest sprawdzalne: `parse_event` zostawia tylko zdarzenia, które
      `classify` zna, a System log tej maszyny miał w ostatniej godzinie
      dokładnie jedno zdarzenie (DistributedCOM, nieklasyfikowane). Zero
      przeparsowanych zdarzeń znaczy „nie było awarii sieci”, nie „narzędzie
      nie działa”. Test asercjonuje więc na surowym XML w oknie tygodnia
      i dodatkowo przepuszcza pełną ścieżkę `read_channel`.

---

## 2. Kwerendy listy zdarzeń ciągną 33 KB kontekstu na wiersz

**Objaw.** Zakładka Historia rysuje się z zauważalnym obciążeniem i blokuje
zapisy monitora.

**Pomiar** (200 awarii w dobie, kontekst realistycznej wielkości):

| Kwerenda | Czas | Dane |
|---|---|---|
| `events_since(24h)` przez `EVENT_COLUMNS` | **21,56 ms** | 13,4 MB kontekstu |
| ta sama kwerenda bez kolumn `context` | **199 µs** | — |
| stosunek | **108×** | |

Kontekst pojedynczej awarii waży **33 398 bajtów**, nie „kilka kilobajtów", jak
twierdzi komentarz przy [`LEAD_SWEEPS`](../src/monitor.rs#L231). 180 próbek
`LeadSample` w JSON plus `context_end`.

**Gdzie to boli.** `EVENT_COLUMNS` w [store.rs](../src/store.rs#L420) zawiera
`context` i `context_end`, więc każda kwerenda listy je ciągnie. Zakładka
Historia z rozwiniętym wierszem robi **co klatkę**, przy `request_repaint_after(500ms)`:

- [`events_since(24h)`](../src/ui/history.rs#L24) — bez cache
- [`recent_events(300)`](../src/ui/history.rs#L52) — bez cache
- [`tweaks_between`](../src/ui/history.rs#L151)
- [`cause::analyse`](../src/ui/history.rs#L157) → `Evidence::from_event` parsuje
  33 KB JSON i **klonuje** `lead_up` przed deserializacją
- [`parse_lead`](../src/ui/history.rs#L341) parsuje **ten sam** JSON drugi raz
  i klonuje `lead_up` drugi raz
- [`samples_between`](../src/ui/history.rs#L370) po całym zakresie awarii

Zakładka Na żywo ma cache, ale jej `collect_stats` też woła `events_since(24h)`
raz na sekundę ([live.rs:1064](../src/ui/live.rs#L1064)).

**Naprawa.** Rozdzielić kwerendy: chuda lista (bez `context`/`context_end`) dla
tabeli i kart, pełny kontekst tylko dla rozwiniętego wiersza, parsowany raz i
trzymany w polu `App` dopóki zaznaczenie się nie zmieni.

**Kryterium akceptacji.**
- [x] `events_since` i `recent_events` nie zwracają kolumn kontekstu.
      `EVENT_COLUMNS` to dziś sześć chudych kolumn, a `Event` nie ma już pól
      `context`/`context_end` — nie da się ich pociągnąć przez przypadek.
      Ciężka połowa to osobny `EventContext` i `Store::event_context(id)`.
- [x] `cause::analyse` i `parse_lead` dostają jeden, raz sparsowany `Evidence`.
      `analyse` przyjmuje `Option<&Evidence>` zamiast parsować z `Event`;
      `parse_lead` usunięty, wykres czyta `evidence.lead`. Parsowanie robi
      `ui::history::ensure_detail` raz na zmianę zaznaczenia, wynik siedzi
      w `App::outage_detail`.
- [x] Benchmark w repo: `store::tests::listing_events_does_not_carry_their_context`.
      **Próg to 5 ms, nie 1 ms.** Powód: to zwykły test, więc chodzi w buildzie
      debug, gdzie chuda kwerenda zajmuje 848 µs — margines do 1 ms jest tak
      cienki, że wolniejsza maszyna zapalałaby czerwone bez regresji. Regresja,
      przed którą test stoi, jest 10× nad progiem.

**Pomiar po naprawie** (build debug, baza w pamięci, 200 awarii po 33 KB):

| Kwerenda | Przed | Po |
|---|---|---|
| `events_since(24h)` | **8,04 ms** | **848 µs** |

Liczby z planu (21,56 ms → 199 µs) były z buildu release na pliku, więc nie
porównują się wprost; stosunek jest ten sam rząd wielkości.

---

## 3. `refresh_tweaks` blokuje wątek UI na 576 ms

**Pomiar** (21 tweaków, build release):

```text
tcp_congestion    166,6 ms
tcp_autotuning    158,7 ms
mtu               124,6 ms
wlan_power_plan   121,7 ms
radio_tx_power      0,8 ms
...
refresh_tweaks total (runs on the UI thread): 575,8 ms
```

571 z 576 ms to cztery uruchomienia `netsh`/`powercfg`.

**Gdzie.** [`App::new`](../src/ui/mod.rs#L294) woła `refresh_tweaks()` **przed
pierwszą klatką**, więc okno pojawia się o pół sekundy później niż musi. Drugie
wywołanie przy każdym kliknięciu zakładki Optymalizacja
([mod.rs:473](../src/ui/mod.rs#L473)), trzecie po każdym `apply`.

Dodatkowo [`apply_all_safe`](../src/ui/opt.rs#L538) woła `t.read(&net)` po raz
drugi dla każdego tweaka, choć `refresh_tweaks` właśnie policzył to samo, i całą
pętlę apply robi synchronicznie w wątku UI, bez paska postępu.

**Naprawa.** Przenieść na wątek roboczy i wysłać wynik istniejącym kanałem
`Job`. Infrastruktura już jest (`ScanProgress`/`ScanDone`).

**Kryterium akceptacji.**
- [x] Okno pojawia się bez czekania na odczyt tweaków. Zmierzone przez
      tymczasowy `eprintln!` wokół `App::new` (build debug, instrumentacja
      usunięta po pomiarze): **406 ms → 658 µs**.
- [x] Zakładka Optymalizacja pokazuje stan „odczytuję" zamiast zamarzać
      (`i18n::opt_reading`, `App::tweaks_loading`). Na czas odczytu Odśwież
      i Zastosuj wszystko są wyłączone, żeby nie działały na nieaktualnym
      odczycie.
- [x] `apply_all_safe` nie czyta stanu po raz drugi — bierze `tweak_states`
      z ostatniego zakończonego odczytu. Tweak bez odczytu jest pomijany,
      nie stosowany na ślepo.

**Pomiar własny przed naprawą** (`optimize::tests::read_cost_per_tweak`,
release, `--ignored`): `refresh_tweaks` = 369,2 ms, z czego `tcp_congestion`
117,5 · `mtu` 105,4 · `tcp_autotuning` 102,1 · `wlan_power_plan` 44,0. Pozostałe
17 tweaków razem: 0,1 ms. Plan mierzył 575,8 ms; rozkład ten sam, cała cena to
cztery podprocesy.

**Metryka, która nie działa.** Czas do `MainWindowHandle` z PowerShella nie
mierzy tego kryterium: eframe tworzy okno **przed** wywołaniem `App::new`, więc
uchwyt pojawia się, zanim odczyt tweaków w ogóle ruszy (48–916 ms, rozrzut od
cache'u dysku). Stąd instrumentacja `App::new` zamiast pomiaru z zewnątrz.

**Czego to nie naprawia.** Sama pętla `apply` dalej chodzi synchronicznie
w wątku UI i nadal nie ma paska postępu. Plan wspomina o tym w opisie, ale nie
w kryteriach.

---

## 4. `notify_on_outage` to martwy przełącznik

**Objaw.** Checkbox w Ustawieniach nie robi nic.

**Dowód.** Pole jest zdefiniowane ([settings.rs:50](../src/settings.rs#L50)),
domyślnie `true`, zapisywane do JSON i rysowane w
[settings_tab.rs:99](../src/ui/settings_tab.rs#L99). Wyszukanie po całym repo:
nie jest czytane nigdzie poza definicją i checkboxem.

**Naprawa.** Albo podpiąć pod toast w
[`drain_snapshots`](../src/ui/mod.rs#L395), albo usunąć pole i checkbox.
Decyzja należy do Ciebie; sugeruję podpięcie, bo README obiecuje baner przy
zmianie werdyktu.

**Kryterium akceptacji.**
- [x] Wyłączenie przełącznika faktycznie wycisza powiadomienie o awarii.
      Wybrałem podpięcie, nie usunięcie: README obiecuje baner przy zmianie
      werdyktu, a stara wersja w `legacy-python/netdoc/gui.py:731` bramkowała
      dokładnie te dwa toasty, więc to jest odtworzenie intencji, nie nowa
      funkcja.

**Jak.** Decyzja wyszła z `drain_snapshots` do wolnej funkcji
`ui::announcement(previous, snap, outage_started, notify)`, żeby dała się
przetestować bez kontekstu egui. Trzy testy: awaria i powrót mówią, wyłączony
przełącznik wycisza oba końce, niezmieniony werdykt milczy. Mutacja kontrolna:
po usunięciu bramki `!notify` test wyciszania jest czerwony.

**Co przełącznik wycisza, a czego nie.** Wycisza toasty. Nie dotyka zapisu:
`last_status` i `outage_started` są prowadzone tak samo, bo na nich stoi
zakładka Historia i kafelek „Bez przerw". Kto wyłączył wyskakujące okienka, nie
prosił o przestanie nagrywania.

---

## 5. Znaczniki awarii znikają przy interwale sondowania ≥ 2 s

**Objaw.** Przy zmienionym interwale wykres nie oznacza zgubionych pakietów.

**Przyczyna.** Ścieżka nieredukowana w [`reduce`](../src/ui/live.rs#L320)
sprawdza `span < 4.0`, gdzie `span` to odstęp między sąsiadami zgubionej próbki.
Stała 4.0 zakłada zamiatanie co sekundę. Ustawienia pozwalają na wszystko od
300 ms w górę.

**Dowód** (12 zamiatań, trzy środkowe zgubione):

```text
probe_interval =   1 s -> 3 outage marker(s) for 3 lost probes
probe_interval =   2 s -> 0 outage marker(s)
probe_interval =   3 s -> 0 outage marker(s)
probe_interval =   5 s -> 0 outage marker(s)
```

Ścieżka zredukowana (dłuższe okna) jest wolna od tego, bo liczy udział strat
w kubełku, nie odstępy czasowe.

**Naprawa.** Próg musi wynikać z `probe_interval_ms`, tak jak
[`mark_recording_gaps`](../src/ui/live.rs#L284) wyprowadza swój z mediany
odstępów w danych.

**Kryterium akceptacji.**
- [x] Test parametryzowany po interwale 1/2/3/5 s (plus 0,3 s, czyli dolna
      granica ustawień), wszystkie dają 3 znaczniki:
      `ui::live::tests::lost_probes_are_marked_at_every_configured_interval`.
      Przed naprawą czerwony dokładnie tam, gdzie mówił plan: „probe_interval
      = 2 s produced 0 marker(s) for 3 lost probes".

**Naprawa poszła dalej niż próg.** Plan proponował wyprowadzić stałą 4.0
z `probe_interval_ms`. Zrobiłem to inaczej: `mark_recording_gaps` zwraca teraz
listę znaczników, które sam wstawił, a `reduce` porównuje `None` z tą listą,
zamiast zgadywać z odstępu między sąsiadami. Powód: każdy próg wyprowadzony
z odstępów myli się na pierwszej zgubionej sondzie po przerwie w nagrywaniu —
jej lewym sąsiadem jest wstawiony znacznik, więc odstęp wychodzi szeroki
i strata zostaje wzięta za dalszy ciąg ciszy. Funkcja, która wstawiła przerwę,
wie to bez arytmetyki. Trzy testy: parametryzowany po interwale, przerwa
w nagrywaniu nie jest awarią, pierwsza sonda po przerwie jest awarią.

---

## 6. `DwordTweak::read` gubi rozróżnienie „brak wartości" od „brak dostępu"

**Objaw.** Tweak, którego nie dało się odczytać, pokazuje się jako „wart zmiany",
a Revert po jego zastosowaniu **kasuje** wartość zamiast ją przywrócić.

**Przyczyna.** [stack.rs:64](../src/optimize/stack.rs#L64):

```rust
Ok(None) | Err(_) => State::new(..., Some(false), json!({ "value": Value::Null }))
```

Nagłówek [winreg.rs](../src/winreg.rs#L1) mówi wprost, że sens całego modułu to
odróżnienie „wartości nie ma" od „brak dostępu", bo „ta różnica decyduje, czy
tweak zgłosi »nie ustawione« czy »wymaga administratora«". Jedyny konsument tej
różnicy wyrzuca ją jedną linią.

**Łańcuch skutków.** `Err` → `optimal: Some(false)` → widoczny przycisk Zastosuj
(a także kwalifikacja do `apply_all_safe`, jeśli tweak jest `Risk::Low`) →
`record_first` zapisuje `{"value": null}` jako stan pierwotny → Revert wywołuje
`delete_value` na wartości, która istniała.

**Naprawa.** `Err(e)` ma dawać własny wariant `State` z `optimal: None` i tekstem
błędu, dokładnie tak jak robi to już `AdapterPowerSaving::read`
([mod.rs:401](../src/optimize/mod.rs#L401)).

**Kryterium akceptacji.**
- [x] Nieudany odczyt daje `optimal == None` i nie oferuje Zastosuj.
      Mapowanie wyjęte do `stack::dword_state(name, wanted, read)`, trzy testy
      na trzy wyniki. Snapshot przy `Err` to `Value::Null`, nie
      `{"value": null}` — nie ma czego przywracać, więc nie udajemy, że jest.
- [x] Test: tweak z nieczytelną wartością nie trafia do `apply_all_safe`
      (`ui::opt::tests::apply_all_safe_skips_what_it_could_not_read`, przez
      wyciągniętą regułę `is_safe_candidate`).

**Naprawa musiała zejść niżej, do `winreg`.** `read_dword` woła `open()`, które
przy nieistniejącym **kluczu** zwracało `Err`. Gdyby zostawić to tak i tylko
przemapować `Err` na `optimal: None`, popsułyby się wszystkie tweaki
z `create: true` — ich klucz polityki z definicji nie istnieje, dopóki ktoś
polityki nie ustawi, więc przestałyby oferować Zastosuj. Dlatego `read_dword`
używa teraz `open_for_read`, które zwraca `Ok(None)` dla nieistniejącego klucza
i `Err` tylko wtedy, gdy klucz jest, ale się nie otwiera (w praktyce: brak
dostępu). Rozróżnienie zapada tam, gdzie jest informacja — przy kodzie błędu
z systemu — a nie w zgadywance na poziomie tweaka.

**Zmiana zachowania, którą trzeba znać.** Test `missing_key_is_an_error`
utrwalał stare zachowanie i został przepisany na
`a_missing_key_reads_as_absent_rather_than_as_a_failure`. Nowy test na ścieżkę
„nie da się odczytać" nie potrzebuje admina: zapisuje `REG_SZ` tam, gdzie
oczekiwany jest `REG_DWORD`, i sprawdza, że to jest błąd, a nie cztery pierwsze
bajty napisu.

**Czego nie ruszałem.** `mod.rs:801`, `802`, `867`, `870` robią
`.ok().flatten()`, czyli sklejają „brak" z „nie da się odczytać" tak samo.
Plan wskazywał `stack.rs:64` i tylko tam Revert kasował istniejącą wartość,
więc reszta zostaje na osobną pozycję.

---

## 7. Snapshoty tweaków nie rozróżniają kart sieciowych

**Objaw.** Na maszynie z dwiema kartami (laptop plus stacja dokująca, Wi-Fi plus
Ethernet, karta VPN) druga karta zostaje zmieniona bez zapisu stanu pierwotnego.

**Przyczyna.** [`record_first`](../src/optimize/mod.rs#L196) kluczuje po
`tweak.id()`, a tweaki związane z kartą rozwiązują klucz rejestru przez
`adapter_class_key(net)` z bieżącego `NetState`.

**Scenariusz.** Zastosuj `adapter_power` na karcie A (snapshot zapisany, klucz A).
Przełącz się na kartę B, zastosuj ponownie. `record_first` zwraca `false`, bo
wpis o tym id już istnieje, więc **zmiana na B jest wprowadzona i nigdy nie
zapisana**. `has_snapshot("adapter_power")` dalej pokazuje Revert, który
przywróci A i zostawi B zmienioną na zawsze.

To nie jest błąd w `revert` — ta funkcja słusznie czyta klucz ze snapshotu, nie
z bieżącego stanu. Błąd jest w przestrzeni kluczy snapshotów.

**Naprawa.** Klucz `(tweak_id, adapter_guid)` dla tweaków związanych z kartą.
Trait `Tweak` może dostać metodę `scope_key(&self, net) -> String` z domyślną
implementacją zwracającą samo `id()`.

**Kryterium akceptacji.**
- [x] Test: apply na GUID A, apply na GUID B, revert na B przywraca stan B
      (`optimize::tests::a_second_adapter_gets_its_own_recorded_state`).

**Jak.** Trait `Tweak` dostał `scope_key(&self, net) -> String` z domyślną
implementacją zwracającą `id()`, dokładnie jak proponował plan. Klucz per karta
to `"{id}@{guid}"`, deklarują go trzy miejsca: `AdapterPowerSaving`, `NagleOff`
i `AdvTweak` (czyli wszystkie `radio_*` i `nic_*`). `apply`, `revert`
i `has_snapshot` chodzą teraz po `scope_key`, nie po `id`.

**Zgodność wstecz, o której plan nie wspominał.** Snapshoty zapisane przez
wcześniejsze buildy leżą pod samym `id`. Gdyby zmienić klucz bez furtki, każdy,
kto już coś zastosował, straciłby przycisk Cofnij — i to bez żadnego
komunikatu. `snapshot_key_for` sprawdza więc najpierw klucz z zasięgiem, potem
goły `id`. Stary wpis nadal cofa się poprawnie, bo każdy z tych tweaków trzyma
ścieżkę klucza rejestru **wewnątrz** swojego snapshotu, więc revert pisze do
karty, z której zdjęto stan.

**Testy.** Pięć: dwie karty dostają osobne wpisy, drugi apply na tej samej
karcie nie nadpisuje pierwszego odczytu, stary wpis bez zasięgu nadal działa,
brak GUID-a nie tworzy trzeciej przestrzeni kluczy, oraz asercja na prawdziwych
tweakach (`adapter_power`, `nagle_off`, `radio_power_save`, `nic_green_ethernet`
per karta; `net_throttling`, `fast_dns`, `tcp_autotuning` bez zmian). Mutacja
kontrolna: po usunięciu `scope_key` z `AdvTweak` ostatni test jest czerwony.

---

## 8. NetState zamarza na czas awarii; brak wykrywania APIPA/DHCP

**Część pierwsza: zamrożenie.**

[`read_adapters`](../src/probe/netstate.rs#L206) przypisuje wynik tylko gdy
`st.gateway.is_some() && st.up`. Stąd wynika implikacja: `adapter_name` niepuste
⟹ `gateway.is_some()`. Zatem strażnik w monitorze
([monitor.rs:557](../src/monitor.rs#L557)):

```rust
if fresh.gateway.is_some() || !fresh.adapter_name.is_empty() { net = fresh; }
```

upraszcza się do `fresh.gateway.is_some()`. Skoro tak: **gdy brama znika, `net`
przestaje się aktualizować aż do jej powrotu.** UI pokazuje SSID, RSSI i kanał
sprzed awarii przez cały czas jej trwania, czyli dokładnie wtedy, gdy użytkownik
patrzy. `resolve_targets` dalej pinguje bramę, której może już nie być.

**Część druga: APIPA.**

169.254.0.0/16 nie jest rozpoznawane nigdzie w repo. `Ipv4Addr::is_private()`
tego nie obejmuje (tylko 10/8, 172.16/12, 192.168/16), a to jedyne dwa miejsca,
gdzie klasyfikujemy adresy: [path.rs:95](../src/probe/path.rs#L95) i
[diagnose.rs:795](../src/diagnose.rs#L795).

Skutek: nieudany DHCP, jedna z najczęstszych domowych awarii, jest nie do
odróżnienia od „nie ma karty sieciowej". `check_medium` mówi wtedy
„no connection", co jest złą radą dla działającej karty bez dzierżawy.

**Status weryfikacji.** To jedyna pozycja oparta wyłącznie na czytaniu kodu.
Wywód jest implikacją logiczną z dwóch warunków, więc jest pewny, ale nie
wyciągałem kabla ani nie wyłączałem radia. Warto potwierdzić ręcznie przed
naprawą.

**Naprawa.** `read_adapters` powinno zwracać najlepszą kartę nawet bez bramy
(osobne pole „ma bramę"), a klasyfikacja adresów dostać gałąź APIPA z własnym
`Finding` i własną radą. Przy okazji: wybór karty idzie dziś po kolejności
enumeracji, nie po metryce trasy — na zadokowanym laptopie z Wi-Fi i Ethernetem
naraz albo z kartą Hyper-V/VPN można opisać niewłaściwą kartę. Właściwe API to
`GetBestInterface`/`GetBestRoute2` dla 0.0.0.0.

**Kryterium akceptacji.**
- [x] Odłączenie kabla / wyłączenie Wi-Fi aktualizuje `NetState` zamiast go
      zamrażać. **Odtworzone na żywo**, `netsh wlan disconnect` przy włączonym
      `netstate::tests::watch_netstate` (za zgodą użytkownika).
- [x] Adres 169.254.x.x produkuje własny `Finding` z radą o DHCP
      (`diagnose::is_apipa` + gałąź w `check_medium`, trzy testy).
- [x] Wybór karty idzie po metryce trasy — właściwie po samej trasie:
      `GetBestInterface(1.1.1.1)`, a metryka interfejsu rozstrzyga remisy.
      Sprawdzone na żywo: `GetBestInterface -> Some(22)`, co zgadza się
      z jedyną trasą domyślną tej maszyny (`ifIndex 22`, `192.168.50.1`),
      i `read()` wybrał WiFi.

**Status weryfikacji zmieniony.** Plan pisał, że to jedyna pozycja oparta
wyłącznie na czytaniu kodu. Już nie jest. Przebieg przed naprawą:

```text
 4s adapter=WiFi    up=true  gw=192.168.50.1  ssid=wi-fi dom_2G  rssi=Some(-65)
 5s adapter=<none>  up=false gw=-             ssid=-             rssi=None
 ...
10s adapter=<none>  up=false gw=-             ssid=-             rssi=None
```

`read()` zwracał `NetState::default()`, więc strażnik w monitorze odrzucał
odczyt i UI trzymał stan sprzed awarii. Po naprawie ten sam eksperyment:

```text
 5s adapter=WiFi    up=false gw=192.168.50.1  ssid=-  rssi=None
```

**Czego pomiar nauczył ponad plan.** Brama i adres lokalny **nie znikają** od
razu: Windows trzyma dzierżawę i wpis trasy jeszcze kilka sekund po utracie
asocjacji. Prompt-owe fakty to `up` i puste pola Wi-Fi, nie `gateway.is_none()`.
Zapisane w komentarzu przy `read_adapters`, żeby nikt nie zbudował detekcji
zerwania na bramie. Drugie: w trakcie testu Windows sam podłączył się do innej
sieci (inny SSID, inna brama) — czyli aplikacja potrafiła opisywać zupełnie
inną sieć niż ta, o której myśli użytkownik.

**Czego nie zweryfikowałem na żywo.** Gałąź APIPA. Wywołanie jej naprawdę
wymaga zabrania maszynie DHCP albo wpisania statycznego 169.254.x.x, co
oznacza admina i zerwanie łącza na dłużej. Pokryta trzema testami
jednostkowymi, ścieżka od `local_ip` do `check_medium` jest prosta, ale to
nadal jest test kodu, nie maszyny.

---

## 9. Zamiatanie trwa 3,6 s, gdy cele nie odpowiadają

**Pomiar** (cztery adresy zarezerwowane, timeout domyślny 1000 ms):

```text
four dead targets, default 1000 ms timeout: sweep took 3.6001268s
```

Cztery cele pingowane sekwencyjnie na jednym uchwycie
([monitor.rs:576](../src/monitor.rs#L576)). W czasie awarii kadencja
próbkowania cicho spada z 1 s do 3,6 s.

**Skutek uboczny.** To uruchamia [`mark_recording_gaps`](../src/ui/live.rs#L284),
którego próg wynosi `max(3×mediana, mediana+2)` ≈ 3,0 s dla okna w większości
zdrowego. Test na realnej kadencji:

```text
8 failed probes at the real outage cadence:
8 synthetic "not recording" breaks inserted, 16 outage marker(s)
```

Osiem nieudanych sond zostaje oznaczonych jako „aplikacja nie nagrywała".
Wizualnie wychodzi podobnie, bo linia i tak się łamie, ale mechanizm, który miał
odróżniać „nie patrzyliśmy" od „sondy padły", działa odwrotnie dokładnie
w scenariuszu, dla którego powstał.

**Naprawa.** Sondowanie równoległe (osobny `Pinger` na cel albo pula), albo
skrócenie timeoutu po serii porażek. Uwaga: `Pinger` jest `Send`, ale świadomie
nie `Sync` — komentarz przy `unsafe impl Send` mówi wprost, żeby nie dodawać
`Sync` na jego podstawie.

**Kryterium akceptacji.**
- [x] Zamiatanie z czterema martwymi celami mieści się w skonfigurowanym
      interwale: **503 ms przy interwale 1 s** (przed: 3,59–3,99 s).

**Pomiar** (cztery adresy RFC 5737, build release):

| Timeout | Sekwencyjnie | Równolegle |
|---|---|---|
| 150 ms | 2,00 s | 0,50 s |
| 300 ms | 2,00 s | 0,50 s |
| 500 ms | 2,00 s | 0,50 s |
| 1000 ms | 3,99 s | 1,01 s |
| jak konfiguruje monitor (800 ms) | — | **0,50 s** |

**Jak.** `PingerPool`: jeden uchwyt ICMP na cel, każdy wątek bierze swój
`Pinger` przez `&mut`. To jest to, co trzyma całość poprawną: `Pinger` jest
`Send`, ale świadomie nie `Sync`, więc uchwyt jest **przenoszony** do wątku, a
nie dzielony — `IcmpSendEcho` nigdy nie widzi dwóch wywołań na jednym uchwycie.
Komentarz przy `unsafe impl Send` mówi wprost, żeby nie dodawać `Sync`; nie
dodałem.

**Dwie rzeczy, których pomiar nauczył ponad plan.**

1. **Podłoga ~500 ms.** Ping do nieosiągalnego adresu nie wraca szybciej,
   choćby timeout był 150 ms. Skracanie timeoutu poniżej pół sekundy nie kupuje
   nic — czyli druga propozycja z planu („skrócenie timeoutu po serii porażek")
   sama by tego nie rozwiązała.
2. **Sam równoległy sweep nie wystarczał do kryterium.** Przy domyślnych
   ustawieniach timeout i interwał to po sekundzie, więc sweep wychodził
   1,01 s — dalej ponad interwał. Stąd `Settings::sweep_timeout_ms()`: sufit
   80% interwału. Sonda, która wciąż czeka, gdy pora na następne zamiatanie,
   już odpowiedziała na jedyne zadane pytanie. Sufit dotyczy tylko kadencji
   monitora; jednorazowy skan i traceroute biorą ustawioną wartość wprost.

**Testy.** Dwa w normalnym zestawie: koszt zamiatania (próg 1,2 s, skalibrowany
na zmierzonych 0,50 s równolegle wobec 2,00 s sekwencyjnie) oraz test kolejności
— wyniki muszą wracać przypisane do właściwych celów, bo pula tnie cele na
kawałki i zszywa odpowiedzi po indeksie. Plus `sweep_cost_with_dead_targets`
jako `#[ignore]`.

---

## 10. Druga, tylko-do-odczytu `Connection` dla UI

**Objaw.** Komentarz w [store.rs:140](../src/store.rs#L140) mówi:
„WAL keeps the writer from blocking the UI thread's reads". Na poziomie SQLite
to prawda, w tym kodzie nie.

**Przyczyna.** Jest jedno `Mutex<Connection>`, więc odczyty z zapisami
serializują się w Rust, zanim SQLite je zobaczy. WAL nie daje tu nic. Te 21,5 ms
z punktu 2 nie tylko zacinają rysowanie, ale blokują `add_samples` monitora.

**Naprawa.** Osobna `Connection` tylko do odczytu dla wątku UI. Wtedy komentarz
staje się prawdziwy, a punkty 2 i 10 razem zdejmują UI z drogi monitora.

**Kryterium akceptacji.**
- [x] Wątek monitora nigdy nie czeka na blokadę trzymaną przez rysowanie.
      Zmierzone testem `a_reader_drawing_a_chart_does_not_hold_up_the_monitors_writes`:
      najgorszy zapis przy czytelniku mielącym godzinę próbek w pętli.

**Pomiar** (baza na dysku, 14 400 próbek, czytelnik w pętli):

| Konfiguracja | Najgorszy zapis |
|---|---|
| jedno `Mutex<Connection>` | **24–39 ms** |
| osobne połączenia | **0,36–0,91 ms** |

Próg w teście to 8 ms: rząd wielkości nad stanem naprawionym i trzykrotnie
poniżej zepsutego. Mutacja kontrolna (odczyty zawrócone na połączenie pisarza)
daje 38,8 ms i czerwony test.

**Jak.** `Store` ma dwa połączenia: `conn` do zapisów i `read` do odczytów.
Czytelnik dostaje `PRAGMA query_only=ON`, więc „UI nigdy tędy nie pisze" jest
faktem o połączeniu, a nie umową o tym, którą metodę wywołać. Dla bazy
w pamięci `read` jest `None` i odczyty wracają na jedyne połączenie — drugie
połączenie do `:memory:` byłoby drugą, pustą bazą, nie innym widokiem tej samej.

**Komentarz, który wreszcie jest prawdziwy.** WAL faktycznie pozwala czytać
w trakcie zapisu, ale tylko **między połączeniami**. Przy jednym `Mutex` obie
strony ustawiały się w kolejce w Ruście, zanim SQLite w ogóle je zobaczył.

---

## 11. Updater nie czyta `SHA256SUMS`, które sam publikuje

**Stan.** [Workflow](../.github/workflows/release.yml) wiesza `SHA256SUMS`
obok binarki. [`verify`](../src/update.rs) sprawdza tylko nagłówek `MZ`,
minimum 1 MiB i zgodność rozmiaru z API.

**Uwaga.** To nie jest przeoczenie. Wpis w
[docs/architecture.md](architecture.md) z 2026-09-20 mówi wprost, że weryfikacja
jest kształtowa, oparta na zaufaniu do HTTPS i GitHuba, oznaczona `ponytail:`,
a właściwą odpowiedzią jest podpisywanie wydań. To po prostu tańsza połowa tej
samej decyzji: kilkanaście linii, asset już istnieje.

**Naprawa.** Pobrać `SHA256SUMS`, policzyć skrót pobranego pliku, porównać.
Docelowo Authenticode, ale to koszt certyfikatu.

---

## 12. Brak strażnika pojedynczej instancji

**Objaw.** Dwa procesy piszą do jednej `history.db` po jednej próbce na sekundę
na cel. `Store::stats` liczy jitter jako średnią odległość między **kolejnymi**
próbkami, więc przeplot z dwóch procesów zafałszuje statystykę, a `loss_pct`
liczy wiersze.

Scenariusz realny: autostart uruchamia kopię z `--minimised`, użytkownik klika
skrót i uruchamia drugą.

**Status weryfikacji.** Nie odtwarzałem, żeby nie zaśmiecić prawdziwej
`history.db`. Wniosek z kodu: `Store::open_default` otwiera ten sam plik,
`Monitor::start` rusza bezwarunkowo, nic nie sprawdza obecności innej instancji.

**Naprawa.** Nazwany mutex (`CreateMutexW`) i wyniesienie istniejącego okna na
wierzch zamiast drugiego procesu.

---

## 13. Higiena repo

- [ ] `NetDoctor-1.1.0-windows-x64/netdoctor.exe` (5,8 MB) w gicie — połowa wagi
      `.git` (8,7 MB). 22 commity, więc `filter-repo` jest jeszcze tani.
- [ ] `__pycache__/run.cpython-313.pyc` w gicie.
- [ ] `legacy-python/` (17 plików) — zostawić tylko jeśli świadomie służy za
      referencję, inaczej to martwy kod, który ktoś kiedyś zacznie czytać jak
      żywy.
- [ ] README mówi „43 tests", jest 178.
- [ ] README nie wspomina, że `history.db` rośnie do ~367 MB (pomiar niżej).
- [ ] README nie wspomina, że test obciążenia ciągnie dane ze
      `speed.cloudflare.com` bez górnego limitu (4 strumienie w pętli).
      `--scan` bez `--quick` robi to domyślnie — na łączu mobilnym to realne
      pieniądze.
- [ ] Brak `ci.yml` na push i PR. Dziś workflow rusza tylko na tag `v*`. Ten sam
      zestaw kroków (clippy, test) na push do main łapałby regresję przed
      wydaniem.
- [ ] Brak atrybutów lintów przy 42 blokach `unsafe`.
      `#![deny(unsafe_op_in_unsafe_fn)]` byłoby proporcjonalne.

---

## 14. Niewypełnione placeholdery

Trzy razy to samo brakujące zdanie „co to jest i dla kogo":

- [ ] nagłówek [CLAUDE.md](../CLAUDE.md) — kotwica dla każdej decyzji agenta
- [ ] „Co to jest" w [docs/_index.md](_index.md)
- [ ] „Structure" w [docs/architecture.md](architecture.md)

---

## Do dodania (osobno od naprawiania)

W kolejności wartości:

1. **Diagnoza APIPA i nieudanego DHCP** — największa dziura merytoryczna, dane są
   w `GetAdaptersAddresses`, brakuje gałęzi. Domyka się z punktem 8.
2. **Eksport historii do CSV/JSON** — baza i generator raportu już są, godzina
   pracy, a daje użytkownikowi coś do wysłania dostawcy.
3. **Porównanie przed i po dla tweaków** — jest tabela `tweaks` i są pomiary. To
   jedyna rzecz, która udowadnia, że aplikacja cokolwiek dała.
4. **Progi na RSSI zamiast `wlanSignalQuality`** — `check_wifi` stawia progi
   (45%, 65%) na wartości mapowanej przez producenta, nieliniowej i
   niestandardowej, choć `rssi_dbm` jest już czytane. −67 dBm to branżowa linia
   dla VoIP.
5. **IPv6** — cały stos to `Ipv4Addr`/`AF_INET`, `resolve_target` odrzuca adresy
   v6. README to przyznaje w Ograniczeniach, więc to świadomy zakres, nie błąd,
   ale na DS-Lite i CGNAT aplikacja nie ma nic do powiedzenia.
6. **Domena regulacyjna przy rekomendacji kanału** — `NON_DFS_5` nie sprawdza
   regionu.
7. **Trzeci język** — przy obecnym makrze i18n oznacza dopisanie trzeciego
   argumentu do 500+ wpisów. Jeśli to w planach, lepiej przemyśleć teraz.

---

## Co jest zdrowe

Warto to zapisać, żeby przy naprawianiu nie zepsuć rzeczy, które działają.

- `cargo clippy -- -D warnings` czysto, 178/178 testów w 1,4 s, zero `unwrap()`
  poza testami.
- **Brak wycieku.** Trzyminutowy soak z ukrytym oknem: pamięć 77–78 MB i płasko,
  uchwyty 399 → 394, wątki stabilne na 10, CPU 0,82% jednego rdzenia. Rozbiór
  FFI WLAN zwalnia każdą alokację — sprawdzone wszystkie trzy ścieżki
  (`read_connection`, `read_channel`, `read_rssi`) plus `WlanFreeMemory(list)`.
- [`system_tool`](../src/optimize/mod.rs#L289) domyka podstawienie `netsh.exe`
  obok binarki dziedziczącej uprawnienia administratora. Rzadko ktoś o tym
  pamięta. (Ironia: to ta sama funkcja, która powoduje punkt 1.)
- [`ReplyBuf`](../src/probe/icmp.rs#L79) oparty o `Vec<u64>` zamiast
  `vec![0u8; n]` z powodu wyrównania do 8 bajtów. Prawdziwa dbałość o soundness.
- `record_first` jako niezmiennik Revertu, zapis snapshotów przez temp plus
  rename, `close_orphans` przy starcie, `clamp` w `Measurements::shares`,
  `ever_replied` w `HopStats`, wykluczanie kanałów DFS z rekomendacji,
  `dominant_scope` z totalnym porządkiem zamiast kolejności `HashMap`.
- Wzorzec `wide(path).as_ptr()` w [winreg.rs](../src/winreg.rs) jest **poprawny**
  — tymczasowy `Vec<u16>` żyje do końca instrukcji zawierającej wywołanie.
  Sprawdzone, bo to klasyczne miejsce na UB.

---

## Pomiary referencyjne

Wszystkie na tej maszynie, build release, benchmarki wycofane z drzewa po
pomiarze.

| Co | Wynik |
|---|---|
| `history.db` po 14 dniach, ustawienia domyślne (4 cele, 1 s) | **366,7 MB** |
| wstawienie 4 838 400 próbek | 31,0 s |
| `samples_between(60 s)` | 112 wierszy, 150 µs |
| `samples_between(300 s)` | 1072 wiersze, 445 µs |
| `samples_between(900 s)` | 3472 wiersze, 1,74 ms |
| `samples_between(3600 s)` | 14 272 wiersze, 7,18 ms |
| `stats("cloudflare", 300 s)` | 268 próbek, 262 µs |
| `observing_since` na pełnej bazie | **330 ms** (start, pod blokadą store) |
| `prune(14)` | 4,53 ms |
| kontekst jednej awarii | **33 398 B** |
| `events_since(24h)`, 200 awarii | **21,56 ms**, 13,4 MB |
| ta sama kwerenda bez kolumn kontekstu | 199 µs |
| `refresh_tweaks` (21 tweaków) | **575,8 ms** |
| zamiatanie, 4 martwe cele, timeout 1000 ms | **3,600 s** |
| `dns_lookup_ms` na nazwie NXDOMAIN | 25,7 ms |

Schemat `samples` trzyma `target` jako TEXT w każdym z 4,8 mln wierszy, plus
drugi raz w `idx_samples_target_ts`. Identyfikator całkowity albo rollup
minutowy dla starych danych to oczywista droga, jeśli 367 MB uznasz za za dużo.

---

## Czego nie zweryfikowałem

Uczciwie, żeby nie budować na piasku:

- ~~**Zamrożenie `NetState`** (punkt 8)~~ — odtworzone na żywo 2026-09-21,
  `netsh wlan disconnect` przy włączonym `watch_netstate`. Patrz punkt 8.
- **Dwie instancje** (punkt 12) — nie odtwarzałem, żeby nie zaśmiecić prawdziwej
  bazy.
- **Zacięcie `getaddrinfo` przy nieosiągalnym resolverze** — NXDOMAIN zmierzony
  i szybki (25 ms), wolnej ścieżki nie umiałem wywołać bez zmiany DNS w systemie.
  `dns_lookup_ms` nie ma timeoutu i jest wołane w wątku zamiatania co 10 sweepów
  ([monitor.rs:565](../src/monitor.rs#L565)), więc ryzyko istnieje, ale jego
  rozmiar jest nieznany.

## Hipotezy, które pomiar obalił

Zapisane, żeby nikt do nich nie wracał:

- **`--minimised` nie odcina dostępu do okna.** Podejrzewałem, że
  `with_visible(false)` bez ikony w zasobniku zostawia proces bez możliwości
  przywołania. Uruchomienie pokazało `MainWindowHandle` = 853360, ustawiony
  tytuł i wpis w pasku zadań. Nieprawda.
- **Nie ma wycieku pamięci.** Soak pokazał płaskie 77–78 MB przez trzy minuty.
  Wzrost 72,8 → 78,4 MB to rozgrzewka, nie dryf.
- **Brak wycieków w FFI WLAN.** Podejrzewałem `WlanQueryInterface` bez
  `WlanFreeMemory`. Każda z trzech ścieżek zwalnia pamięć.
