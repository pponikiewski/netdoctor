# netdoc — Architecture

## Structure
<!-- uzupełniane w miarę rozwoju projektu -->

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

## Open questions
<!-- Track unresolved technical decisions -->
