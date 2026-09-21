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
- 2026-09-21 — wydanie v1.2.0: czternastopunktowy audyt repo zamknięty w całości. Dziennik Windows wreszcie czytany (literówka w nazwie narzędzia zabijała 13 najmocniejszych kodów przyczyn), zamiatanie równoległe zamiast sekwencyjnego (3,6 s → 0,50 s przy czterech martwych celach), osobne połączenie do odczytu w store (najgorszy zapis 24–39 ms → 0,36–0,91 ms), odczyt tweaków zdjęty z wątku UI (start 406 ms → 658 µs), weryfikacja pobrania przez SHA-256, strażnik pojedynczej instancji, rozpoznanie APIPA, NetState opisujący kartę w trakcie awarii. Repo ma `ci.yml` na push i PR oraz `deny(unsafe_op_in_unsafe_fn)`.
- 2026-09-20 — zakładka Na żywo tłumaczy każdą liczbę, którą pokazuje (legenda, kafelki, progi, ośie); repo ma przypiętą konfigurację rustfmt i przechodzi `cargo fmt --check`.
- 2026-09-20 — aplikacja jest do pobrania z GitHuba (`pponikiewski/netdoctor`, wydanie v1.1.0) i potrafi się sama zaktualizować: workflow wydaniowy zielony na całej długości, `netdoctor.exe` i `SHA256SUMS` wiszą przy tagu, pobranie zweryfikowane bez uwierzytelniania.
- 2026-09-20 — kafelek „Bez przerw" mówi prawdę na świeżej instalacji i po uśpieniu maszyny: liczy nieprzerwany czas obserwacji, a nie brak wpisów w dzienniku awarii.
