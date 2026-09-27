# Audyt: czy NetDoctor robi to, co obiecuje

Jesteś recenzentem produktu i kodu jednocześnie. Masz przed sobą repozytorium
NetDoctor (Rust, egui, Windows). Aplikacja obiecuje użytkownikowi domowemu trzy
rzeczy:

1. **Sprawdza internet**: stale mierzy łącze i mówi, czy działa.
2. **Szuka problemów**: gdy coś się psuje, wskazuje, które ogniwo zawiodło
   (karta sieciowa, Wi-Fi, router, dostawca, DNS) i dlaczego.
3. **Optymalizuje sieć**: proponuje i wprowadza zmiany, które realnie
   poprawiają połączenie, z możliwością cofnięcia.

Twoje zadanie: ocenić, na ile każda z tych obietnic jest spełniona w kodzie
i na ile dobrze aplikacja ją **pokazuje** użytkownikowi, który nie zna się na
sieciach.

## Zasada nadrzędna

Aplikacja jest warta tyle, ile jej werdykt. Uczciwe "nie udało się tego
odczytać" jest lepsze niż pewny siebie strzał. Każdy przypadek, w którym
porażka pomiaru może zostać pokazana jako wynik pomiaru (np. brak odpowiedzi
traktowany jak awaria, brak logów jak "cisza w logach", 0 jako prawdziwa
wartość), to błąd wysokiej wagi, nawet jeśli wygląda niewinnie.

## Od czego zacząć

- `README.md`: to jest deklaracja. Traktuj każde zdanie jako twierdzenie do
  sprawdzenia, nie jako fakt.
- Mapa obietnic na kod (punkt wyjścia, nie pełna lista):
  - sprawdzanie łącza: `src/monitor.rs`, `src/probe/icmp.rs`,
    `src/probe/netstate.rs`, `src/ui/live.rs`, `src/tray.rs`, `src/store.rs`
  - szukanie problemów: `src/cause.rs`, `src/diagnose.rs`,
    `src/probe/eventlog.rs`, `src/probe/path.rs`, `src/ui/history.rs`,
    `src/ui/diag.rs`, `src/ui/report.rs`
  - optymalizacja: `src/optimize/` (mod, radio, stack), `src/probe/airscan.rs`,
    `src/bandwidth.rs`, `src/ui/opt.rs`, `src/ui/bloat.rs`
- Szukaj symboli przez Grep/codegraph, czytaj konkretne funkcje, nie całe
  katalogi.

## Czego NIE robić

- Nie uruchamiaj testu obciążenia (pobiera 130 MB do ponad 1 GB danych).
- Nie stosuj żadnych zmian z zakładki Optymalizacja ani resetu stosu sieciowego.
- Nie edytuj kodu. To jest audyt, nie poprawka. Propozycje zmian opisz słowami.
- `cargo test` i `cargo clippy -- -D warnings` możesz uruchomić, żeby
  potwierdzić stan repozytorium.

## Pytania, na które masz odpowiedzieć

### 1. Sprawdzanie internetu
- Czy monitor odróżnia "łącze leży" od "nie mierzyliśmy" (uśpienie komputera,
  pauza, brak uprawnień do pingów, aplikacja zamknięta)?
- Czy tabela werdyktów z README (router tak/nie, internet tak/nie, DNS,
  odłączona karta) jest zaimplementowana dokładnie tak, jak opisano? Wskaż
  rozbieżności.
- Czy prywatny adres (Pi-hole, NAS, CGNAT) może zostać uznany za dowód, że
  internet działa?
- Czy ikona w zasobniku, powiadomienia i zakładka Live mówią to samo w tym
  samym momencie?

### 2. Szukanie problemów
- Czy reguły w `cause.rs` mają pokrycie w danych, które naprawdę są zapisywane
  przy awarii? Czy któraś reguła opiera się na polu, które bywa puste?
- Czy odczyt dziennika zdarzeń Windows rozróżnia "brak wpisów" od "nie udało
  się odczytać"?
- Czy werdykt "winny jest ten hop" z `path.rs` wymaga, żeby strata
  przenosiła się do końca trasy? Czy hop, który nigdy nie odpowiada, jest
  opisany jako "nie odpowiada", a nie "100% strat"?
- Czy eksportowany raport dałby się obronić w rozmowie z działem wsparcia
  dostawcy? Czego w nim brakuje, żeby konsultant nie mógł odpowiedzieć
  "proszę zrestartować router"?
- Jakie typowe domowe problemy aplikacja przeoczy? (np. IPv6, VPN, kilka kart
  naraz, sieć mesh, przełączanie Wi-Fi/kabel, captive portal). Oceń, czy dla
  każdego z nich przynajmniej uczciwie mówi, że nie wie.

### 3. Optymalizacja
- Dla każdej zmiany z tabel w README: czy kod robi to, co deklaruje, czy
  zapisuje poprzednią wartość przed zmianą i czy Revert przywraca dokładnie
  stan sprzed zmiany, także po restarcie?
- Czy "Zastosuj bezpieczne zmiany" rzeczywiście dotyka tylko pozycji niskiego
  ryzyka, odwracalnych i jeszcze nieustawionych?
- Czy któraś zmiana może pogorszyć sieć w typowym przypadku (np. "preferuj
  5 GHz" przy słabym sygnale, BBR2, stały DNS 1.1.1.1 w sieci firmowej lub
  z Pi-hole)? Czy aplikacja o tym ostrzega w miejscu, gdzie użytkownik decyduje?
- Czy aplikacja potrafi pokazać, że zmiana pomogła? Jeśli nie, oceń, na ile
  "optymalizuje" jest uczciwym słowem, a na ile "zmienia ustawienia".
- Czy rekomendacja kanału Wi-Fi opiera się na sensownym modelu (moc, nie liczba
  sieci; pomija własne AP; unika DFS) i czy mówi, kiedy danych jest za mało?

### 4. Jak aplikacja to pokazuje
- Czy osoba bez wiedzy sieciowej po 30 sekundach w aplikacji wie: czy teraz
  działa, co ostatnio się zepsuło i co może z tym zrobić?
- Czy każda liczba ma obok wyjaśnienie, co oznacza zła wartość?
- Gdzie interfejs obiecuje więcej, niż kod wie? Gdzie ukrywa coś, co kod wie,
  a co przydałoby się użytkownikowi?
- Czy teksty w obu językach (`src/i18n.rs`) mówią to samo?

## Format odpowiedzi

Odpowiedz po polsku, bez długich myślników.

1. **Werdykt w trzech zdaniach**: po jednym na każdą obietnicę, z oceną
   `spełnia` / `częściowo` / `nie spełnia`.
2. **Tabela ustaleń**: kolumny `obietnica`, `ustalenie`, `dowód`
   (`plik:linia`), `waga` (wysoka/średnia/niska), `proponowany kierunek`.
   Najpierw ustalenia wysokiej wagi. Bez dowodu w kodzie nie ma ustalenia:
   jeśli coś tylko podejrzewasz, oznacz to jako `do sprawdzenia`.
3. **Rozbieżności README a kod**: lista zdań z README, które nie są prawdą
   albo są prawdą tylko częściowo.
4. **Luki funkcjonalne**: czego brakuje, żeby każda z trzech obietnic była
   spełniona w pełni, uszeregowane według tego, jak często dotyczy to zwykłego
   użytkownika domowego.
5. **Jedna rzecz do zrobienia najpierw**: i dlaczego ta.

Zanim oddasz odpowiedź, przeczytaj ją jako sceptyczny recenzent i wskaż
przynajmniej jedną słabość własnego audytu (np. czego nie dało się sprawdzić
bez uruchomienia aplikacji na prawdziwym łączu).
