# netdoc — Session Notes
<!-- One line per commit, appended via /sync: date | commit subject | what/why -->

- 2026-09-20 | feat: enhance outage analysis and logging with new metrics and improved handling | Rozbudowa analizy awarii i zapisu zdarzeń w cause/diagnose/store, żeby historia niosła więcej niż sam fakt przerwy.
- 2026-09-20 | feat(live): make the chart and the cards readable without prior knowledge | Zakładka Na żywo tłumaczy teraz każdą liczbę: klikalne chipy legendy z odczytem, wyjaśnienia celów i kafelków pod kursorem, podpisane progi, uporządkowana legenda pod wykresem; przy okazji usunięte zapytania DNS i SQLite wykonywane przy każdej klatce.
- 2026-09-20 | style: format the codebase with rustfmt, pinned by rustfmt.toml | Kod był formatowany ręcznie i nie pasował do żadnej konfiguracji rustfmt, więc `cargo fmt` wymagany przez CLAUDE.md rozsypywał diff po kilkunastu modułach; jednorazowe przeformatowanie z przypiętym `use_small_heuristics = "Max"`.
- 2026-09-20 | chore: keep the formatting commit out of git blame | `.git-blame-ignore-revs` z hashem commita formatującego, żeby blame dalej wskazywał autora treści, a nie przebiegu formatowania.
