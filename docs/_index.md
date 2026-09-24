---
type: project
project: netdoc
status: active
updated: 2026-09-24
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
- 2026-09-24 — Historia awarii w nowym układzie: lista obok szczegółów, przebieg awarii na dwóch wykresach (czas odpowiedzi i sygnał) na wspólnej osi czasu, oraz czyszczenie historii z potwierdzeniem.
- 2026-09-24 — pierwszy krok porządkowania wyglądu: Ustawienia podzielone na strony, z przypiętym paskiem zapisu i widocznym stanem niezapisanych zmian, zamiast jednej długiej kolumny z przyciskiem Zapisz na samym dole.
- 2026-09-24 — Diagnoza pokazuje dowody, nie tylko werdykt: rysunek ogniw z milisekundami i stratami każdego, tabela wszystkich pomiarów z jawnym „nie zmierzono”, oraz opcjonalne wyjaśnienie AI przez OpenRouter na kluczu użytkownika, z maskowaniem danych identyfikujących sieć.
- 2026-09-23 — audyt funkcji zamknięty poza #10, plus odczyt routera przez UPnP IGD: nieudana zmiana nie jest przyczyną awarii, błąd zapisu do bazy daje szary stan zamiast zielonego, jeden próg jittera, Pi-hole/AdGuard bez rady „zmień DNS”, pomiar skutku zmiany (doba przed i po), test obciążenia w obie strony z zastrzeżeniem o nasyceniu, trasa nie obwinia ostatniego widocznego hopa, kanały 5 GHz liczone blokami 80 MHz, a analiza awarii wie, czy router sam zgłaszał WAN jako rozłączony, czy się zrestartował i czy wrócił z nowym adresem publicznym.
- 2026-09-23 — wydanie v1.3.0: tryb gry (nakładka z pingiem i stroną skoku, „Przygotuj łącze” cofane samo), ekran główny zwykłymi słowami dla osób nietechnicznych, próba TCP gdy sieć blokuje pingi, liczniki błędów kabla, Cofnij DNS wraca do DHCP, awaria dostawcy nie jest przepisywana na LAN po jednym pingu.
- 2026-09-23 — faza 2 przeglądu wiarygodności werdyktu: werdykt odpowiada temu, co zmierzono. Lokalne urządzenia nie udają internetu, skan i monitor nie polegają na jednym adresie, router ignorujący pingi nie jest awarią LAN, awaria ma w historii najpoważniejszy osiągnięty stan, DNS jest pytany z pominięciem pamięci podręcznej Windows i wymaga dwóch porażek, a reguły przyczyn mają pewność zgodną z dowodem (w tym poprawione id DHCP według manifestu dostawcy).
- 2026-09-23 — faza 1 przeglądu wiarygodności werdyktu: błąd pomiaru nie jest już werdyktem. Nieotwarty ICMP nie zapisuje awarii ani nie powiadamia (skan mówi „nie ustalono”), nieczytelny stan Wi-Fi nie znaczy „rozłączona”, zasobnik szarzeje przy nieświeżym pomiarze, interwał ponad 30 s jest odrzucany. Każdy punkt z testem odtwarzającym i mutacją kontrolną.
- 2026-09-23 — ikona w zasobniku (kolor stanu, menu, krzyżyk chowa) i powiadomienia Windows o awarii i powrocie, sprawdzone na prawdziwym odcięciu Wi-Fi. Przy okazji naprawione: `--minimised` nigdy nie ukrywało okna, awarie w historii trwały o ok. minutę za długo.
- 2026-09-23 — siedem błędów z przeglądu 2026-09-22 naprawionych: aplikacja nie znika już po restarcie z aktualizacji, DNS nie blokuje pomiaru, sen i pauza nie zawyżają awarii, Modern Standby rozpoznawany, Apply bez odczytanego stanu zablokowane, pauzy zadań się nie gryzą, `--scan` pisze na konsolę. Restart i konsola sprawdzone na prawdziwych procesach, ze starą wersją jako kontrolą.
- 2026-09-21 — wydanie v1.2.0: czternastopunktowy audyt repo zamknięty w całości. Dziennik Windows wreszcie czytany (literówka w nazwie narzędzia zabijała 13 najmocniejszych kodów przyczyn), zamiatanie równoległe zamiast sekwencyjnego (3,6 s → 0,50 s przy czterech martwych celach), osobne połączenie do odczytu w store (najgorszy zapis 24–39 ms → 0,36–0,91 ms), odczyt tweaków zdjęty z wątku UI (start 406 ms → 658 µs), weryfikacja pobrania przez SHA-256, strażnik pojedynczej instancji, rozpoznanie APIPA, NetState opisujący kartę w trakcie awarii. Repo ma `ci.yml` na push i PR oraz `deny(unsafe_op_in_unsafe_fn)`.
- 2026-09-20 — zakładka Na żywo tłumaczy każdą liczbę, którą pokazuje (legenda, kafelki, progi, ośie); repo ma przypiętą konfigurację rustfmt i przechodzi `cargo fmt --check`.
- 2026-09-20 — aplikacja jest do pobrania z GitHuba (`pponikiewski/netdoctor`, wydanie v1.1.0) i potrafi się sama zaktualizować: workflow wydaniowy zielony na całej długości, `netdoctor.exe` i `SHA256SUMS` wiszą przy tagu, pobranie zweryfikowane bez uwierzytelniania.
- 2026-09-20 — kafelek „Bez przerw" mówi prawdę na świeżej instalacji i po uśpieniu maszyny: liczy nieprzerwany czas obserwacji, a nie brak wpisów w dzienniku awarii.
