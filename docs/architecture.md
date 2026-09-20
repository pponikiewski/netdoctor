# netdoc — Architecture

## Structure
<!-- uzupełniane w miarę rozwoju projektu -->

## Decisions
<!-- Append: date — decision — why -->

- 2026-09-20 — `rustfmt.toml` z `use_small_heuristics = "Max"`, a całe repo przeformatowane jednym commitem (`c5780a6`) wpisanym do `.git-blame-ignore-revs` — kod był formatowany ręcznie i nie odpowiadał żadnej konfiguracji rustfmt, więc reguła „cargo fmt przed każdym commitem" z CLAUDE.md produkowała diff po kilkunastu nietkniętych modułach. Konfiguracja „Max" rusza 853 linie zamiast 3271 przy domyślnej, bo odpowiada temu, jak ten kod był pisany. Od teraz `cargo fmt --check` przechodzi czysto i reguła jest bezpieczna do wykonania.

- 2026-09-20 — dane zakładki Na żywo liczone raz na zamiatanie, nie raz na klatkę: zakres celu wędruje z `Settings::targets()` do pola `ChartSeries` w `ChartCache`, a kafelki mają własny `CardCache` o żywotności jednej sekundy — `Settings::targets()` rozwiązuje przez DNS każdy host dodany przez użytkownika, a kafelki robiły trzy zapytania do SQLite; jedno i drugie działo się w wątku rysowania, przy kursorze nad wykresem nawet 60 razy na sekundę. Zasada: wszystko, co dotyka bazy, sieci albo blokady współdzielonej z wątkiem monitora, ma trafiać do pamięci podręcznej odświeżanej w rytmie pomiaru.

- 2026-09-20 — czasy dymków ustawiane na `Context`, nie na `Ui` — `Response` czyta `tooltip_delay`, `tooltip_grace_time` i `show_tooltips_only_when_still` wyłącznie z `ctx.style()`, więc te same pola ustawione na `Ui` są po cichu ignorowane. Ustawienie żyje w `apply_theme`.

## Open questions
<!-- Track unresolved technical decisions -->
