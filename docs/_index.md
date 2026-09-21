---
type: project
project: netdoc
status: active
updated: 2026-09-20
---

# 🧠 netdoc

> Mózg projektu. Pliki aktualizowane przez Claude Code (`/sync`).

## Co to jest

NetDoctor to aplikacja desktopowa na Windowsa dla osoby, której „wywala neta"
w domu i która nie wie, czy winna jest karta Wi-Fi, router, czy dostawca. Pinguje
bramę i kilka celów w internecie naraz i odczytuje winnego ze **wzorca** porażek,
zamiast pokazywać jedną liczbę; każdą awarię zapisuje do SQLite razem ze stanem
łącza w tamtej chwili i trzema minutami sondowania, które do niej doprowadziły.

To drugie jest właściwym produktem: dzięki temu zerwanie o trzeciej w nocy da się
wyjaśnić następnego ranka, a „mam słaby internet" zamienić w materiał, z którym
dostawca musi się zmierzyć. Stąd bierze się zasada, która rozstrzyga spory w tym
repo: uczciwe „nie udało się odczytać" jest zawsze warte więcej niż pewna siebie
zgadywanka.

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
