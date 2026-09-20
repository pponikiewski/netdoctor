---
type: project
project: netdoc
status: active
updated: 2026-09-20
---

# 🧠 netdoc

> Mózg projektu. Pliki aktualizowane przez Claude Code (`/sync`).

## Co to jest
<!-- 2-3 zdania: co robi aplikacja i dla kogo. Uzupełnij raz, ręcznie. -->

## Nawigacja
- [[architecture]] — struktura i decyzje architektoniczne
- [[notes]] — dziennik commitów (jedna linia na commit)
- [[plan]] — aktualny spec z kryteriami (powstaje przy pierwszym /plan)
- [[inbox]] — pomysły z telefonu, czytane przy brainstormingu
- README (wizytówka repo) → w rootcie projektu, aktualizowany przez /sync

## Status
<!-- aktualizuj przy kamieniach milowych -->
- 2026-09-20 — zakładka Na żywo tłumaczy każdą liczbę, którą pokazuje (legenda, kafelki, progi, ośie); repo ma przypiętą konfigurację rustfmt i przechodzi `cargo fmt --check`.
- 2026-09-20 — aplikacja jest do pobrania z GitHuba (`pponikiewski/netdoctor`, wydanie v1.1.0) i potrafi się sama zaktualizować: workflow wydaniowy zielony na całej długości, `netdoctor.exe` i `SHA256SUMS` wiszą przy tagu, pobranie zweryfikowane bez uwierzytelniania.
- 2026-09-20 — kafelek „Bez przerw" mówi prawdę na świeżej instalacji i po uśpieniu maszyny: liczy nieprzerwany czas obserwacji, a nie brak wpisów w dzienniku awarii.
