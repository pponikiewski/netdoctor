# Audyt funkcji NetDoctor: wynik (2026-09-23, gałąź feat/tray, f175116)

Stan repozytorium: `cargo clippy -- -D warnings` czysto, `cargo test` 274 zaliczone, 8 pominiętych.
Audyt robiony wyłącznie z kodu. Aplikacji nie uruchamiałem na prawdziwym łączu.

## 1. Werdykt

- **Sprawdza internet: częściowo.** Monitor jest staranny (pauza, brak pomiaru i przeterminowany odczyt nie udają awarii, tylko publiczne adresy świadczą o internecie), ale opiera się wyłącznie na pingach. Sieć, która blokuje ICMP na zewnątrz, jest przez cały czas raportowana jako awaria dostawcy.
- **Szuka problemów: częściowo.** Architektura dowodów jest mocna (logi Windows przed wnioskami, trasa zapisana z awarią, uczciwe "brak dowodów"). Ale dwa błędy psują właśnie ten dowód, który ma trafić do dostawcy: jedna zgubiona odpowiedź routera przepisuje awarię dostawcy na "sieć lokalna", a nieudana zmiana ustawień jest wskazywana jako prawdopodobna przyczyna.
- **Optymalizuje sieć: częściowo.** Zmiany są opisane, oznaczone ryzykiem i w większości odwracalne. Aplikacja nigdy jednak nie mierzy, czy zmiana pomogła, a cofnięcie zmiany DNS nie przywraca stanu sprzed zmiany u typowego użytkownika z DHCP.

## 2. Ustalenia

| # | Obietnica | Ustalenie | Dowód | Waga | Kierunek |
|---|---|---|---|---|---|
| 1 | optymalizacja | **Revert "Szybkiego DNS" ustawia DNS na stałe zamiast przywrócić DHCP.** Snapshot zapisuje tylko listę aktualnych serwerów, a `FirstDnsServerAddress` zwraca te z DHCP tak samo jak ręczne. U kogoś, kto ma DNS z routera (192.168.1.1), Revert wpisze `source=static address=192.168.1.1`. Na innej sieci (praca, kawiarnia) nazwy przestaną się rozwiązywać, a nic nie wskaże, że to przez NetDoctor. Gałąź `source=dhcp` działa tylko przy pustej liście. | [optimize/mod.rs:666-679](src/optimize/mod.rs#L666-L679), [optimize/mod.rs:714-742](src/optimize/mod.rs#L714-L742), [netstate.rs:239-245](src/probe/netstate.rs#L239-L245) | wysoka | W snapshot zapisać źródło (rejestr `Tcpip\Parameters\Interfaces\{guid}`: `NameServer` pusty = DHCP). Revert przy DHCP: `source=dhcp`. Test na snapshot z DHCP. |
| 2 | szukanie problemów | **Jedna nieudana odpowiedź routera zmienia awarię dostawcy w awarię sieci lokalnej, na stałe.** `escalate` działa na każdym pojedynczym złym odczycie, bez serii. Ranking `scope_rank` mierzy "lokalność", nie wagę: lan (3) > isp (2). Router pod obciążeniem albo ograniczający ICMP zgubi jeden ping w trakcie 2-godzinnej awarii dostawcy i cała awaria trafia do historii jako "lan". Wypada wtedy z licznika `isp_pattern`, a raport obwinia domową sieć. | [monitor.rs:984-991](src/monitor.rs#L984-L991), [monitor.rs:1276-1279](src/monitor.rs#L1276-L1279), [cause.rs:495](src/cause.rs#L495) | wysoka | Eskalować dopiero po `outage_after_fails` kolejnych odczytach nowego stanu. Rozważyć zapisywanie przebiegu stanów zamiast jednej etykiety. |
| 3 | sprawdzanie łącza | **Monitor rozpoznaje internet wyłącznie po ICMP.** Gdy sieć blokuje pingi na zewnątrz (część hoteli, sieci firmowych, niektórzy operatorzy komórkowi), a router odpowiada, werdykt to stale `IspDown`: zapis awarii, powiadomienie, przyczyna `isp_sustained` z pewnością "prawdopodobne" po 2 minutach. Diagnoza ma już próbę TCP na 443 i rozpoznaje "ICMP filtrowane", monitor z niej nie korzysta. | [monitor.rs:1362-1382](src/monitor.rs#L1362-L1382), [cause.rs:487-490](src/cause.rs#L487-L490), [diagnose.rs:1203-1210](src/diagnose.rs#L1203-L1210) | wysoka | Przed ogłoszeniem `IspDown` jedna próba TCP do kotwicy. Jeśli przejdzie, to stan "internet działa, pingi blokowane", nie awaria. |
| 4 | szukanie problemów | **Nieudana zmiana ustawień jest zapisywana jak udana, a potem obwiniana za awarię.** Przy błędzie `log_tweak(id, "apply", "", błąd)`, identycznie jak przy sukcesie. `after_tweak` nie filtruje po wyniku i ma pewność "prawdopodobne", wyżej niż każdy odczyt sygnału. Zmiana, która się nie wykonała, zostaje uznana za najbardziej prawdopodobną przyczynę. | [ui/opt.rs:513-514](src/ui/opt.rs#L513-L514), [ui/opt.rs:596-597](src/ui/opt.rs#L596-L597), [cause.rs:200-215](src/cause.rs#L200-L215) | wysoka | Akcja `apply_failed` dla błędów, a `after_tweak` bierze pod uwagę tylko `apply`. |
| 5 | pokazywanie | **Diagnoza i Live oceniają jakość innymi progami.** Live: jitter powyżej 2 × `jitter_ok_ms` (30 ms), ping z najszybszego celu. Diagnoza: jitter powyżej 15 ms, ping ze średniej jednej kotwicy. Przy jitterze 20 ms nagłówek jest zielony, a Diagnoza ostrzega. To ta sama klasa błędu, którą naprawił ba106f0 dla kart w Live. | [monitor.rs:1418-1421](src/monitor.rs#L1418-L1421), [diagnose.rs:1162](src/diagnose.rs#L1162), [diagnose.rs:1168](src/diagnose.rs#L1168) | średnia | Jedna funkcja progów wspólna dla obu miejsc albo nazwać wprost, że Diagnoza jest surowsza. |
| 6 | optymalizacja | **"Szybki DNS" jest oznaczony jako warty zmiany dla każdego niepublicznego resolvera, łącznie z Pi-hole i AdGuard.** Przyczyna DNS zawsze podpina tę zmianę jako naprawę. Użytkownik z Pi-hole dostaje radę, która wyłącza mu blokowanie reklam. | [optimize/mod.rs:677](src/optimize/mod.rs#L677), [cause.rs:500-511](src/cause.rs#L500-L511) | średnia | Prywatny resolver inny niż brama: stan "nie dotyczy / Twój własny DNS", bez podpinania naprawy. |
| 7 | optymalizacja | **Brak pomiaru skutku.** Żadne miejsce nie porównuje jakości łącza przed i po zmianie. Aplikacja "zmienia ustawienia", a nie "optymalizuje" w sensie, który mogłaby udowodnić. Dane do tego już są (próbki w SQLite, dziennik zmian z czasem). | brak w `optimize/`, `ui/opt.rs`, `store.rs` (grep: effect/before/after) | średnia | Przy każdej zmianie: mediana pingu, jitter i straty z 24 h przed i po, pokazane w wierszu zmiany, z "za mało danych" gdy trzeba. |
| 8 | optymalizacja | **Test obciążenia mierzy tylko pobieranie.** Na łączach asymetrycznych bufferbloat przy wysyłaniu (wideorozmowy, upload do chmury) jest zwykle gorszy. Test nie sprawdza też, czy łącze zostało naprawdę nasycone: serwer ograniczony do 100 Mbit/s na łączu gigabitowym da ocenę A. | [bandwidth.rs:115](src/bandwidth.rs#L115), [bandwidth.rs:166-221](src/bandwidth.rs#L166-L221) | średnia | Druga faza z wysyłaniem. Przy mbps wyraźnie poniżej prędkości łącza: ocena z zastrzeżeniem. |
| 9 | sprawdzanie łącza | **Błąd zapisu próbek do bazy jest ignorowany, a brak danych daje zielony werdykt.** `let _ = store.add_samples(...)`. Gdy zapis się nie uda, `line_quality` ma mniej niż 10 próbek i zwraca `None`, więc `quality_verdict` kończy na `Ok`. | [monitor.rs:1208](src/monitor.rs#L1208), [monitor.rs:1415-1424](src/monitor.rs#L1415-L1424) | średnia | Błąd zapisu jako stan "nie mierzę" (jak `Blind`), nie jako zdrowe łącze. |
| 10 | sprawdzanie łącza | **Brak bramy = "karta odłączona".** `gw == None` daje `AdapterDown`. Przy VPN z pełnym tunelem (np. WireGuard) interfejs trasy domyślnej zwykle nie ma bramy, więc awaria serwera VPN zostanie opisana jako problem karty. | [monitor.rs:1392](src/monitor.rs#L1392) | średnia, do sprawdzenia | Sprawdzić na WireGuard/OpenVPN. Jeśli potwierdzone: osobny stan "brak bramy na interfejsie trasy", bez obwiniania karty. |
| 11 | szukanie problemów | **Ostatni widoczny hop trasy jest obwiniany bez potwierdzenia.** Gdy trasa kończy się na `MAX_HOPS = 12` przed celem albo cel milczy, ostatni widoczny router ograniczający ICMP zostaje wskazany jako źródło strat. | [path.rs:81](src/probe/path.rs#L81), [path.rs:261](src/probe/path.rs#L261) | niska, do sprawdzenia | "Na wiarę" tylko wtedy, gdy ostatni hop to adres docelowy. |
| 12 | optymalizacja | **Model kanałów 5 GHz ignoruje sąsiadów na 80 MHz.** Liczy tylko ten sam kanał i kanał oddalony o 4. Sąsiad na 36/80 MHz zajmuje 36-48, a 44 może wyjść jako "najcichszy". | [airscan.rs:214-218](src/probe/airscan.rs#L214-L218) | niska | Uwzględnić szerokość kanału sąsiada, jeśli skan ją podaje. W przeciwnym razie liczyć cały blok 80 MHz. |
| 13 | optymalizacja | Nieudany Revert nie trafia do dziennika zmian (tylko komunikat). | [ui/opt.rs:536](src/ui/opt.rs#L536) | niska | Zapisać jako `revert_failed`. |

Mocne strony, które warto zachować: `Seen` wspólny dla okna i ikony ([monitor.rs:157-199](src/monitor.rs#L157-L199)), DNS wymagający powtórzonego błędu ([monitor.rs:440-493](src/monitor.rs#L440-L493)), "pierwszy snapshot wygrywa" i odmowa zmiany przy nieznanym stanie wyjściowym ([optimize/mod.rs:205-295](src/optimize/mod.rs#L205-L295)), `System32` dla narzędzi uruchamianych z uprawnieniami ([optimize/mod.rs:339-367](src/optimize/mod.rs#L339-L367)), "cicha" ścieżka tylko przy niepustym logu ([cause.rs:371](src/cause.rs#L371)), test obciążenia bez obciążenia daje `Unknown`, nie ocenę ([bandwidth.rs:199-205](src/bandwidth.rs#L199-L205)).

## 3. README a kod

- *"Revert restores the exact prior state"*: nieprawda dla DNS przydzielanego przez DHCP (ustalenie 1).
- *"An outage that starts slow and then goes down is recorded as the worst state it reached"*: "najgorszy" to w praktyce "najbardziej lokalny" i wystarcza jeden odczyt, więc awaria dostawcy może stać się awarią LAN (ustalenie 2).
- *"one filtered address ... is not an outage"*: prawda dla jednego adresu. Gdy filtrowane są wszystkie pingi na zewnątrz, aplikacja ogłasza awarię dostawcy (ustalenie 3).
- *"each row saying in words whether it is set, worth changing, or not available"*: DNS mówi "warto zmienić" także przy świadomie wybranym własnym resolverze (ustalenie 6).
- *Load test: "This is usually the answer to 'good ping, still lagging'"*: tylko dla kierunku pobierania (ustalenie 8).

## 4. Luki funkcjonalne (od najczęstszych u zwykłego użytkownika)

1. Brak pomiaru skutku zmian: każdy, kto kliknie Optymalizację, nie dowie się, czy coś pomogło.
2. DNS z DHCP po Revert: każdy laptop z domyślną konfiguracją, który użyje tej zmiany i ją cofnie.
3. Awaria dostawcy przepisana na LAN: każda dłuższa awaria dostawcy przy routerze, który czasem gubi pingi do siebie.
4. Upload bufferbloat: każdy, kto ma problem z wideorozmowami na łączu asymetrycznym.
5. Sieci blokujące ICMP: rzadziej w domu, częściej w podróży i na LTE.
6. VPN: rosnąca grupa, ale wymaga potwierdzenia.
7. IPv6: świadomie poza zakresem (sekcja Limitations), uczciwie opisane.

## 5. Najpierw

**Ustalenie 1 (Revert DNS).** To jedyne miejsce, w którym aplikacja sama psuje użytkownikowi sieć, i to w sposób, którego nikt nie połączy z NetDoctorem, bo objaw pojawia się dopiero na innej sieci. Poprawka jest mała: jedno pole w snapshocie i jedna gałąź w `revert`. Zaraz potem ustalenie 2, bo psuje dokładnie ten dowód, dla którego aplikacja istnieje.

## Słabe strony tego audytu

- Nie uruchamiałem aplikacji. Pytanie "czy laik po 30 sekundach wie, co się dzieje" zostało bez odpowiedzi. Zakładki Live i Historia oceniłem tylko po kodzie, bez zobaczenia ekranu.
- Ustalenie 10 (VPN bez bramy) opiera się na mojej wiedzy o WireGuard na Windows, nie na teście.
- Nie sprawdziłem, czy teksty PL i EN w `i18n.rs` mówią to samo (4000 linii). Nie czytałem w całości `ui/report.rs`, `ui/history.rs` ani wszystkich 16 zmian z `optimize/radio.rs` i `stack.rs`. Sprawdziłem mechanizm snapshot/revert wspólny dla wszystkich, a pojedynczo tylko DNS. Podobny problem ze stanem wyjściowym może dotyczyć MTU.
