# Przegląd czytelności i UX (2026-09-23)

Przegląd na żywo buildu debug z gałęzi `feat/tray` (commit `2dbdf27`), obu języków, w oknie
dużym (1120×760) i minimalnym (900×620), plus prawdziwy raport z tej maszyny. Zrzuty leżą w
`target/ux-screens/` (poza repo). Kodu nie zmieniałem.

Miara: domowy użytkownik ma w kilka sekund wiedzieć, czy działa, co padło i co zrobić, a
aplikacja nie może przy tym mówić więcej, niż wie.

## 5 najważniejszych zmian

| # | Zmiana | Dlaczego pierwsza | Koszt |
| --- | --- | --- | --- |
| 1 | Pauza widoczna w nagłówku i kafelkach (szary stan „Pomiar wstrzymany”, kafelki wyszarzone) | Dziś po wstrzymaniu okno dalej mówi „Połączenie sprawne” na zielono. To jedyne miejsce, gdzie UI twierdzi więcej, niż wie | mały |
| 2 | Jedno źródło prawdy dla nagłówka i kafelków na Na żywo | Nagłówek „sprawne” na zielono obok czerwonego „Utrata pakietów 2.8%”. Użytkownik nie wie, której części wierzyć | średni |
| 3 | Właściciel prywatnych skoków za routerem: „adres prywatny (twój sprzęt albo sieć dostawcy)” zamiast „twoja sieć” | Raport dla dostawcy przypisuje użytkownikowi 4 skoki, które najpewniej należą do dostawcy. To argument dla dostawcy, nie dla użytkownika | mały |
| 4 | Naprawić trzy rzeczy, które wyglądają na błąd: znaki `\` w tekście o kanale, nachodzący pasek w Optymalizacji (PL), „0 min przestoju” w raporcie | Każda z nich podkopuje zaufanie do reszty ekranu | mały |
| 5 | Diagnostyka: pełny skan bez testu obciążenia jako domyślny, test obciążenia jako świadomy wybór z ostrzeżeniem o danych | Główny przycisk dziś pobiera do gigabajta bez ostrzeżenia na tym ekranie | mały |

## Nagłówek (wszystkie zakładki)

**Pauza nie zmienia werdyktu.** `ui/mod.rs:146` (`verdict_line`) sprawdza tylko `blind`.
Po 12 s wstrzymania: zielone „Połączenie sprawne”, kafelki z liczbami, jedyny ślad to napis
„Wznów monitor” na przycisku (`41-paused-12s.png`). Zasobnik w tej sytuacji szarzeje, okno nie.
Propozycja: `verdict_line` bierze też `monitor.is_paused()` / blokady zadań i nieświeżość jak
`tray::Seen`, pokazuje szare „Pomiar wstrzymany” / „Brak świeżego pomiaru”. Waga: blokuje
zrozumienie. Koszt: mały.

**Wiersz faktów łamie się w środku pozycji.** Przy 900×620 „802.11ax” zostaje w pierwszej
linii, a „(HE)” spada do drugiej; w dużym oknie spada „karta WiFi” (`21-min-pl-live.png`,
`39-large-live.png`). Propozycja: łamać tylko między faktami, a „802.11ax (HE)” i nazwę karty
przenieść do podpowiedzi, bo nie pomagają zdecydować, co zrobić. Waga: kosmetyka, ale widoczna
na każdym ekranie. Koszt: mały.

**Przycisk administratora zawsze w prawym górnym rogu.** Jest najbardziej wyróżnionym
elementem nagłówka, choć potrzebny tylko w Optymalizacji. Propozycja: w nagłówku zostawić
krótką etykietę „tryb zwykły”, przycisk przenieść do Optymalizacji, gdzie zmiany go wymagają.
Waga: myli (sugeruje, że coś jest nie tak). Koszt: mały.

**Tytuł okna nie zmienia języka.** Po przełączeniu na English tytuł zostaje
„diagnostyka sieci i optymalizacja opóźnień” (`main.rs:163`, ustawiany raz przy starcie).
Waga: kosmetyka. Koszt: mały (`ViewportCommand::Title` przy zmianie języka).

## Na żywo

**Nagłówek i kafelki mierzą co innego.** Kafelki czytają samo `cloudflare` z 5 minut
(`live.rs:1003`), a werdykt w nagłówku bierze najmniejszą stratę ze wszystkich publicznych celów
z najwyżej 60 s (`monitor::quality_verdict`). Efekt na zrzucie `44-report-toast.png`: „Połączenie
sprawne” (zielone) i „Utrata pakietów 2.8%” (czerwone). Propozycja: kafelek straty i jittera
liczony tą samą regułą i z tego samego okna co werdykt, z podpisem okna („ostatnia minuta”);
5 minut zostaje na wykresie. Waga: blokuje zrozumienie. Koszt: średni.

**„Resolver ISP” to 1.1.1.1.** Ta maszyna ma DNS 1.1.1.1 i 8.8.8.8, więc cel `dns_isp` pinguje
Cloudflare i nazywa go resolverem dostawcy (`i18n.rs:96`, legenda i raport). Propozycja: etykieta
„Twój serwer DNS (1.1.1.1)”, a gdy adres jest taki sam jak inny cel, nie pingować go drugi raz.
Waga: myli. Koszt: mały.

**Wykres jest gęsty od pionowych kresek.** Przy stracie ~2% czerwone i żółte linie przecinają
cały wykres (`39-large-live.png`), a pod nim „skoki: 64 — 47 wspólne, 17 pojedyncze” i
„14 powyżej 75 ms, poza skalą”. Użytkownik nie ma z tym czego zrobić. Propozycja: znaczniki
zgubionych pakietów jako krótkie kreski przy osi X zamiast linii przez całą wysokość; licznik
skoków przenieść do podpowiedzi pod „?”. Waga: myli. Koszt: średni.

**Kafelek DNS jest teraz częściej żółty.** Po zmianie z fazy 2.4 czas DNS to prawdziwe zapytanie
do resolvera (13 do 65 ms na tej maszynie), a nie odpowiedź z pamięci podręcznej (0,7 ms). Progi
`DNS_GOOD_MS` / `DNS_OK_MS` ustawiono pod stare wartości. Propozycja: przejrzeć progi na danych
z kilku dni. Waga: myli. Koszt: mały.

**„Od ostatniej awarii 5 min” na czerwono.** Czerwień tu znaczy „niedawno”, a gdzie indziej
„padło”. Propozycja: kolor neutralny, czerwień tylko gdy awaria trwa. Waga: myli. Koszt: mały.

**Minimalne okno ucina kafelki.** Przy 900×620 kafelki są poniżej krawędzi i trzeba przewijać,
żeby zobaczyć stratę i DNS (`21-min-pl-live.png`). Propozycja: kafelki nad wykresem albo niższy
wykres przy małej wysokości. Waga: myli. Koszt: średni.

## Diagnostyka

**Domyślny skan pobiera dużo danych bez ostrzeżenia.** „Dołącz test obciążeniowy” jest
zaznaczony od startu (`mod.rs:337`, `deep_scan: true`), a ostrzeżenie o danych jest tylko na
zakładce Test obciążeniowy. Propozycja: domyślnie odznaczone; przy zaznaczeniu ta sama żółta linia
o danych co w teście obciążenia. Waga: blokuje zrozumienie (koszt dla użytkownika z limitem).
Koszt: mały.

**Pusty stan to jeden gęsty akapit** z „rozcina łańcuch”, „przeżyje pobieranie”, „realny ruch
TCP” (`22-min-pl-diag.png`). Propozycja: jedno zdanie, co skan powie („Sprawdzi, gdzie powstają
opóźnienia i straty: w komputerze, u routera czy u dostawcy”), reszta pod „Jak to działa”.
Waga: kosmetyka. Koszt: mały.

Werdyktu skanu nie oglądałem na żywo (patrz niżej).

## Test obciążeniowy

**Dwa sprzeczne opisy jeden pod drugim.** „Pobiera kilkadziesiąt MB … trwa około 25 sekund”
(`bloat_blurb`, `i18n.rs:958`) i zaraz pod tym „przez około 14 sekund … grubo ponad gigabajt”
(`bloat_cost_warning`). Potwierdzone na `23-min-pl-bloat.png` i `33-min-en-bloat.png`. Drugi opis
zgadza się z README (4 strumienie, 13 s). Propozycja: usunąć zdanie o MB z `bloat_blurb`,
zostawić ostrzeżenie. Waga: blokuje zrozumienie. Koszt: mały.

## Optymalizacja

**Pasek narzędzi nachodzi na siebie po polsku.** Przy 900×620 napis „tylko do odczytu (uruchom
ponownie jako administrator, żeby zastosować)”, pole „pokaż niedostępne” i liczniki leżą na tych
samych pikselach (`24-min-pl-opt.png`). Po angielsku się mieści (`34-min-en-opt.png`), a w dużym
oknie po polsku też częściowo nachodzi (`05-history.png`). Propozycja: napis o trybie tylko do
odczytu jako osobna linia nad paskiem, liczniki w drugim wierszu. Waga: blokuje zrozumienie.
Koszt: mały.

**Znaki `\` w środku zdania.** „dla \ najmniej zatłoczonego eteru … nie milkną \ same z siebie”
w obu językach. Przyczyna: `\\` zamiast kontynuacji linii w `air_dfs_move` (`i18n.rs:1555-1559`).
Propozycja: poprawić i dodać test na `ALL_STRINGS`, który odrzuca `\` z białym znakiem po nim.
Waga: kosmetyka, ale wygląda na zepsutą aplikację. Koszt: mały.

**Skan eteru zajmuje cały ekran przed listą zmian.** Sekcja „Co jeszcze jest w eterze” jest
rozwinięta domyślnie i spycha właściwe ustawienia poniżej krawędzi. Propozycja: zwinięta
domyślnie, z jednolinijkowym podsumowaniem („Kanał 108 jest radarowy, lepszy: 36”). Waga: myli.
Koszt: mały.

**Ostrzeżenie o kanale radarowym jest długie i techniczne** („wyda mu się, że usłyszał radar”,
„DFS”). Propozycja: „Twój router jest na kanale, który musi czasem na chwilę opuścić. To może
dawać krótkie zrywy bez widocznej przyczyny. Ustaw w routerze kanał 36.” Waga: kosmetyka.
Koszt: mały.

## Historia awarii

**Nagłówek zawsze czerwony.** `history.rs:80` barwi na czerwono każde podsumowanie z awariami,
także gdy to głównie „pogorszona jakość” (`15-history.png`: „12 awarii … głównie jako pogorszenie
jakości”). Propozycja: kolor według najpoważniejszego rodzaju (żółty dla samych pogorszeń) i
rozdzielenie w tekście: „2 przerwy i 10 okresów słabej jakości”. Waga: myli. Koszt: mały.

**Tekst nagłówka jest niezręczny:** „12 awarii … głównie jako pogorszenie jakości”, po angielsku
„12 outage(s)”. Propozycja jak wyżej, z odmianą liczebnika. Waga: kosmetyka. Koszt: mały.

**Szczegóły są w języku z chwili awarii.** W angielskim UI kolumna Detail pokazuje
„8.3% utraconych pakietów…” (`35-min-en-history.png`), bo notatka zapisuje się jako gotowy tekst.
Propozycja: zapisywać kod i liczby, tłumaczyć przy wyświetlaniu; stare wiersze zostawić. To zmiana
tego, co trafia do bazy, więc wymaga decyzji. Waga: kosmetyka. Koszt: średni.

**Zaznaczona awaria spoza widocznej części listy.** Panel szczegółów pokazuje awarię z 02:04, a
jej wiersz jest przewinięty poza widok, więc nie wiadomo, skąd się wziął (`25-min-pl-history.png`).
Propozycja: przewinąć listę do zaznaczonego wiersza. Waga: kosmetyka. Koszt: mały.

**„Żadna przyczyna się nie wyróżnia” dominuje.** 7 z 12 wpisów w raporcie i szczegółach. Jako
pierwsza, wyróżniona linia mówi „nic”. Propozycja: dla pogorszeń jakości podawać to, co zmierzono
(„3.3% strat przez 57 s, router bez strat, więc straty były za routerem”), a nie brak przyczyny.
Waga: myli. Koszt: średni.

## Ustawienia

**Przycisk „Zapisz” jest pod krawędzią.** Karty zajmują cały widok, a zapis jest na samym dole
(`settings_tab.rs:33`). Język działa od razu, reszta dopiero po zapisie, więc łatwo zmienić próg
i wyjść bez zapisu. Propozycja: pasek „Niezapisane zmiany · Zapisz · Cofnij” przyklejony u góry,
widoczny tylko gdy szkic różni się od ustawień. Waga: blokuje zrozumienie. Koszt: mały.

**„Zgłaszaj awarie w aplikacji”** (`i18n.rs:213`). Powiadomienia idą przez Windows. Propozycja:
„Powiadomienia Windows o awarii i powrocie”. Waga: myli. Koszt: mały (to nie jest tekst samego
powiadomienia, tylko etykieta przełącznika).

**Progi to żargon w milisekundach** („Jitter akceptowalny do (ms)”). Propozycja: grupa „Progi”
zwinięta pod „Zaawansowane”, bo domyślne wartości są dobre dla większości. Waga: kosmetyka.
Koszt: mały.

## Raport tekstowy

Czytany jako pracownik pomocy technicznej dostawcy, na prawdziwym raporcie z tej maszyny.

- **„łącznie 0 min przestoju”** dla dwóch awarii dostawcy trwających 3 i 8 s. `i18n::span`
  zaokrągla do minut. To błąd z fazy 3 (mój). Propozycja: poniżej minuty podawać sekundy.
  Waga: blokuje zrozumienie. Koszt: mały.
- **„przestój” dla pogorszenia jakości** („pogorszona jakość: 9 raz(y), łącznie 2 min przestoju”).
  Wolne łącze to nie przestój. Propozycja: „łącznie 2 min słabej jakości”. Waga: myli. Koszt: mały.
- **Właściciel skoków:** 192.168.222.1, 172.20.2.1, 10.30.64.1, 10.8.105.1 opisane jako „twoja
  sieć” (`path_owner_local`, `i18n.rs:587`). Cztery prywatne skoki po 4 do 9 ms za routerem to
  najpewniej wewnętrzna sieć dostawcy, a nie cztery routery w domu. Propozycja: „adres prywatny”
  bez przypisywania właściciela; jako „twoja sieć” tylko skoki, które odpowiadają z tej samej
  podsieci co brama. Waga: blokuje zrozumienie (to raport na dostawcę). Koszt: mały.
- **„0% loss” po angielsku w polskim raporcie** (`report.rs:408`, to samo w tabeli ścieżki w UI).
  Waga: kosmetyka. Koszt: mały.
- **Zdanie bez sensu:** „karta wyłączyła się, gdy sygnał wynosił jeszcze nieznany”
  (`ev_adapter_powered_down`, `i18n.rs:2937`). Propozycja: bez sygnału „karta wyłączyła się; siły
  sygnału w tej chwili nie zapisano”. Waga: myli. Koszt: mały.
- **Powtórzenie w dowodzie:** „powód 11 (802.11, kod 11)”. Waga: kosmetyka. Koszt: mały.
- **Zmiany ustawień** podane surowymi identyfikatorami (`fast_dns`, `tcp_congestion`), słowami
  `apply` / `revert` i częściowo po angielsku (starsze wpisy). Propozycja: nazwy z `tweak_name`,
  „zastosowano” / „cofnięto”. Waga: kosmetyka. Koszt: mały.
- **Dobrze:** sekcja awarii ma teraz czas, przyczynę z pewnością, linie dziennika Windows i minutę
  przed awarią. Wpis #7 (rozłączenie Wi-Fi, powód 11, dziennik 2 s przed) jest dokładnie tym, czego
  potrzebuje pomoc techniczna.

## Zasobnik

Oceniony z kodu (`tray.rs`), nie na żywo. Kolory i tooltip rozróżniają pauzę, brak pomiaru, brak
świeżego odczytu i werdykt, a tekst tooltipa mieści się w 127 znakach. W zasobniku pauza jest
szara, w oknie zielona: to ta sama niespójność co w ustaleniu nr 1.

## Czego nie sprawdziłem na żywo

- **Werdykt skanu, stan skanowania i wynik testu obciążenia:** nie uruchamiałem ich, bo zajmują
  łącze i pobierają dane (prompt zabrania bez pytania).
- **Świeży start z pustą bazą:** wymagałby usunięcia `history.db`.
- **Brak ICMP i nieświeży pomiar w oknie:** nie da się ich bezpiecznie wywołać. Oceniłem je z kodu
  i testów.
- **Tooltip, menu i dymki zasobnika:** najechanie kursorem i prawy klik na ikonę nie są pewne przez
  syntetyczne zdarzenia; oceniłem z kodu.
- **Komunikat po zapisie raportu:** zrzut po 8 s był już po jego zniknięciu (6 s). Plik się zapisał.
- **Szczegóły Optymalizacji i wykresu przebiegu w Historii:** widziałem tylko górę tych ekranów, bez
  przewijania.

## Czego nie ruszać

- Pusty stan testu obciążenia: kafelki z „—” i „?” zamiast zer. Uczciwe i czytelne.
- Legenda wykresu jako klikalne chipy z bieżącą wartością: to najczytelniejsza część Na żywo.
- Kolory ryzyka i licznik „13 ustawionych · 1 do poprawy · 7 niedostępnych” w Optymalizacji.
- Stopka nagłówka „tryb zwykły, zmiany wymagają uprawnień administratora”: jasno mówi, dlaczego
  Zastosuj jest wyłączone.
- Sekcja awarii w raporcie jako całość (poza poprawkami wyżej).

## Słabości tego przeglądu

- Oceniam z perspektywy autora zmian z faz 1 do 3, więc część ustaleń (DNS, raport) dotyczy moich
  własnych decyzji i mogę być wobec nich łagodniejszy albo surowszy, niż trzeba.
- Nie widziałem aplikacji na ekranie o innym skalowaniu DPI niż 100%. Przy 125% i 150%, typowych
  na laptopach, ucięcia będą większe.
- Wszystko oglądałem przy jednym stanie łącza (lekkie straty, bez twardej awarii). Ekrany w trakcie
  prawdziwej awarii oceniłem tylko z kodu i z zapisanej historii.
- Nie mierzyłem kontrastu narzędziem; ocena `FG_DIM` na `BG2` jest na oko.
