> **Zamrożona referencja, nie kod do uruchamiania.**
>
> To jest poprzednia, pythonowa wersja NetDoctora, zachowana w repo z jednego
> powodu: kiedy nowa wersja w Rust robi coś nieoczywistego, tutaj widać, jaka
> była pierwotna intencja. Nikt tego nie utrzymuje i nikt nie powinien tego
> instalować. Jeśli szukasz działającej aplikacji, jest w `../src`.

# NetDoctor

Diagnostyka i optymalizacja sieci pod Windows. Zbudowana wokół jednego pytania:
**gdzie dokładnie urywa się połączenie, kiedy „wywala neta"**.

Bez zewnętrznych bibliotek — tylko Python 3.10+ i narzędzia systemowe.

## Uruchomienie

```text
NetDoctor.bat                      # zwykły tryb (diagnoza działa w całości)
NetDoctor (administrator).bat      # potrzebny, żeby stosować zmiany
```

albo `python run.py`.

## Jak to działa

Monitor pinguje jednocześnie router i kilka adresów w internecie, raz na sekundę.
Wzór awarii mówi, kto zawinił:

| Router odpowiada | Internet odpowiada | Werdykt |
| --- | --- | --- |
| tak | tak | działa |
| tak | nie | problem po stronie WAN / operatora |
| nie | nie | problem między komputerem a routerem (Wi-Fi, karta, router) |
| karta rozłączona | — | sterownik, oszczędzanie energii albo zasięg |
| tak | tak, ale nazwy się nie rozwiązują | awaria DNS |

Ta różnica jest istotna: zerwanie po stronie operatora wygląda w Windows tak samo
jak zasypiająca karta Wi-Fi, a naprawia się zupełnie inaczej.

Każde zerwanie ląduje w SQLite razem ze stanem połączenia w tamtej chwili — siłą
sygnału, kanałem, BSSID, prędkością. Dzięki temu zerwanie o 3 w nocy da się
wyjaśnić rano.

## Zakładki

- **Na żywo** — wykres opóźnień z osią czasu, kafelki (ping, jitter, strata
  pakietów, DNS, czas bez zerwań), traceroute, eksport raportu. Kliknięcie
  pozycji w legendzie ukrywa serię.
- **Diagnoza** — jedenaście testów: medium, jakość i pasmo Wi-Fi, zatłoczenie
  kanału, oszczędzanie energii karty, DNS, link do routera, ping do internetu,
  MTU, ustawienia TCP, wiek sterownika, historia zerwań. Każdy wynik ma
  wyjaśnienie, a część — przycisk naprawy.
- **Test obciążenia** — bufferbloat: ping na spoczynku kontra ping przy
  wysyconym łączu. Ocena A–F i konkretna porada. To zwykle odpowiedź na
  „mam dobry ping, a i tak laguje".
- **Optymalizacja** — lista zmian z aktualnym stanem i poziomem ryzyka.
- **Historia zerwań** — kiedy, jak długo, po czyjej stronie.
- **Ustawienia** — częstotliwość pomiarów, progi ocen, własne cele pingowania
  (np. serwer gry), powiadomienia i autostart.

## Praca w tle

Zerwanie zostanie zapisane tylko wtedy, gdy monitor akurat działa. Dlatego:

- **Ustawienia → Startuj razem z Windows** dopisuje NetDoctor do klucza `Run`
  bieżącego użytkownika (bez uprawnień administratora).
- **Uruchamiaj zminimalizowany** — albo `python run.py --minimised`.
- Przy zerwaniu i przy powrocie łącza pojawia się powiadomienie w rogu ekranu
  z informacją, po czyjej stronie był problem. Kliknięcie otwiera okno.

## Zmiany ustawień

Każda zapisuje poprzednią wartość do `%LOCALAPPDATA%\NetDoctor\tweak_snapshots.json`
przed dotknięciem czegokolwiek, więc „Cofnij" działa też po restarcie komputera.

| Zmiana | Ryzyko | Po co |
| --- | --- | --- |
| Oszczędzanie energii karty | niskie | najczęstsza przyczyna zerwań na laptopie |
| Plan zasilania Wi-Fi | niskie | drugi, niezależny mechanizm usypiania radia |
| Szybkie DNS (1.1.1.1) | niskie | usuwa router jako pojedynczy punkt awarii |
| TCP auto-tuning = normal | niskie | naprawia skutki „poradników na ping" |
| Wyłączenie Nagle'a | średnie | kilka–kilkanaście ms mniej w grach; wymaga restartu |
| Limit pakietów multimediów | średnie | Windows dławi ruch przy odtwarzaniu multimediów |
| Korekta MTU | średnie | za duże MTU = strony się nie ładują mimo działającego pingu |
| Reset stosu sieciowego | średnie | akcja ratunkowa, gdy net padł i nie wraca; **bez cofnięcia** |

„Zastosuj wszystkie bezpieczne" rusza tylko pozycje o niskim ryzyku, które dają
się cofnąć i nie są jeszcze ustawione.

## Dane

`%LOCALAPPDATA%\NetDoctor\history.db` — próbki, zdarzenia, dziennik zmian.
Próbki starsze niż ustawiona liczba dni (domyślnie 14) są kasowane automatycznie.
`settings.json` obok trzyma Twoje ustawienia, `tweak_snapshots.json` — stany
sprzed każdej zastosowanej zmiany.

## Ograniczenia

- Ping idzie przez systemowe `ping.exe`, więc rozdzielczość to pełne milisekundy.
- Kilka odczytów (PnPCapabilities, klucze interfejsu) wymaga administratora do
  zapisu; odczyt działa bez.
- Zmiany rejestru (Nagle, limit pakietów, PnPCapabilities) działają po restarcie.

## Testy

```text
python -m unittest discover tests
```

21 testów na parsery i logikę klasyfikacji awarii — czyli na te części, które
przy zmianie języka lub wersji Windows psują się po cichu.
