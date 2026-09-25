# NetDoctor

Network diagnostics and latency optimiser for Windows, built around one
question: **where exactly does the connection break when "the internet drops"?**

A single 5 MB executable. No runtime, no installer, no dependencies.

## Why the question matters

From inside Windows, every outage looks the same: things stop working. But a
sleeping Wi-Fi card and a dead uplink need completely different fixes, and
guessing wrong wastes hours.

NetDoctor pings the router and several internet endpoints at the same time, and
reads the blame off the *pattern* of failures:

| Router answers | Internet answers | Verdict |
| --- | --- | --- |
| yes | yes | working |
| yes | no | WAN / ISP problem |
| no | no | between the PC and the router (Wi-Fi, adapter, router) |
| adapter disconnected | — | driver, power saving, or signal strength |
| yes | yes, but names do not resolve | DNS failure |
| no | yes | working: the router ignores pings to itself (a setting, not a fault) |
| no pings answered, but a TCP connection gets through | — | working: this network blocks pings, so latency and loss cannot be measured |

"Internet answers" means a public address answered: a Pi-hole, a NAS or a
provider's CGNAT box answering proves nothing past the provider. A verdict
that something is down needs its readings to agree, so one filtered address
or one lost DNS reply is not an outage. When no public address answers a ping,
the app tries a TCP connection to port 443 every two seconds, and only calls it
an outage if that fails too. DNS is tested by asking the adapter's
resolvers directly, past the Windows DNS cache, which would otherwise answer
for a resolver that is gone.

Every outage is written to SQLite along with the connection state at that
instant — signal, RSSI, channel, access point, link rate, and whether the router
was still answering — and the three minutes of sweeps that led up to it. That is
what makes a 3 a.m. dropout explainable the next morning, and what turns "my
internet is bad" into evidence an ISP has to engage with.

## From where it broke to why

A verdict of "the router stopped answering" is a location, not a reason, and it
fits a laptop carried out of range, a Wi-Fi card put to sleep by Windows, a
handover to another access point, and a router that genuinely crashed. Those
have four different fixes.

Selecting an outage in the history opens the reasoning behind it: the probable
cause, the numbers that cause was read off, the signal and router latency
plotted from before the break through to the recovery, the state it failed on
against the state it came back into, and any tweak applied shortly beforehand —
which is the first thing worth suspecting and the easiest to undo. Where a
cause has a matching entry in Optimise, one button goes straight to it.

The rules live in [`src/cause.rs`](src/cause.rs) and are plain enough to argue
with: every verdict shows its evidence, so a wrong one is visibly wrong rather
than merely unhelpful.

## What Windows already wrote down

Every probe in the app so far *infers*: it pings, watches a trend, and argues
from it. Meanwhile the operating system was recording what actually happened —
that the access point deauthenticated the card and with which 802.11 reason
code, that the adapter driver faulted, that the machine suspended, that the
DHCP lease could not be renewed — and nothing read it back.

Selecting an outage now reads it back. Two channels are queried around the
outage's own time span, `System` and `Microsoft-Windows-WLAN-AutoConfig/Operational`,
and the lines that mean something become verdicts of their own, ahead of every
inferred one. A log line is not a deduction from latency: it is the OS naming
the event. Reason 4 on a disconnect — *disassociated for inactivity* — says the
access point dropped a card Windows had quietly powered down, which is the
textbook cause of a link that dies while the machine sits idle, and it points
straight at the tweak that fixes it.

The rendered message text is never matched on, because `wevtutil` translates
it; only structured fields are read. Silence is evidence too: a WAN outage with
log entries around it and not one of them a fault on this machine is precisely
the case a provider has to answer. It is claimed only when the log had
something to say at all, since "quiet" and "unavailable" look identical from
here. See [`src/probe/eventlog.rs`](src/probe/eventlog.rs).

## Which hop it starts at

"It is the ISP" is a claim, and the provider's first move is to blame the
Wi-Fi. What settles it is a per-hop measurement. The path to a fixed anchor is
walked every few minutes and every hop on it is then pinged directly on a slow
cadence, continuously — the picture `mtr` draws, kept up rather than run once
after the fact, because the outage worth measuring is never happening while you
are typing the command. Each hop is labelled with whose it is: the router, the
private stretch behind it that is still the household's own kit, the provider's
edge, the networks beyond.

The reading that matters is not "which hop shows loss". A router showing 60%
while everything behind it shows none is rate-limiting the replies it generates
itself — a configuration choice, not a fault, and forwarded traffic never
touches that path. A hop is blamed only when the loss *carries* to the end, and
a hop that has never once answered a probe addressed to it is reported as not
answering rather than as losing everything. The verdict is therefore rarer than
the red figures in the table, which is the point. It also lands in the exported
report, where it is the paragraph a support line cannot answer with "restart
the router". See [`src/probe/path.rs`](src/probe/path.rs).

## Getting it

Download `netdoctor.exe` from the
[latest release](https://github.com/pponikiewski/netdoctor/releases/latest).
It is a single file with nothing to install; `SHA256SUMS` next to it is there
if you want to check what you downloaded.

Put it somewhere you can write to — your user folder is fine, `Program Files`
is not. That is what lets the app replace itself when it updates, without
asking for administrator rights every time.

## Updating

On start the app asks GitHub once whether there is a newer release. If there
is, a banner offers to fetch it; **Update and restart** downloads the new
binary, swaps it in, and the app restarts into it. The copy it replaced is
deleted on the next start, once Windows has released its lock on it.

Settings → Updates has the same thing on a button, the release notes, and the
switch that turns the automatic check off. With it off the app never contacts
GitHub on its own.

If the update fails because the folder is read-only, move the executable
somewhere you own and try again, or download the new version by hand.

## Running it

```text
netdoctor.exe              # the app
netdoctor.exe --scan       # run the diagnostic scan on the console and exit
netdoctor.exe --scan --quick  # the same scan without the load test
netdoctor.exe --minimised  # start minimised (used by autostart)
netdoctor.exe --help
```

`--scan` includes the load test unless you add `--quick`, so a scripted scan
downloads and uploads a few hundred megabytes every time it runs. See the Speed test entry
under Tabs for what that costs. `--quick` is the flag to reach for on a metered
connection or in anything scheduled.

NetDoctor is a windowed program, so `cmd` hands the prompt back before a
command-line run finishes and its output lands after the prompt. Use
`start /wait netdoctor.exe --scan` to wait for it and get its exit code in
`%ERRORLEVEL%`, or redirect it (`netdoctor.exe --scan > scan.txt`).

| Exit code | Meaning |
| --- | --- |
| 0 | the line is sound, or only a setting on this PC is worth changing |
| 1 | a fault was found now, or outages were recorded in the last 24 hours |
| 2 | nothing could be decided: pings could not be sent, an unknown flag, or the database would not open |

Diagnostics need no elevation. Applying changes does; the app offers to relaunch
itself when you ask it to apply one.

## Language

English and Polish. On first run the app follows the Windows UI language; the
Settings tab has a picker that switches everything immediately, including the
exported report. The choice is remembered in `settings.json`.

Both languages are compiled into the binary as `&'static str`, so a lookup is a
branch and nothing is loaded at runtime. Adding a third means adding a column to
the table in [`src/i18n.rs`](src/i18n.rs) — a missing translation is a compile
error, not a string that silently falls back to English.

Technical vocabulary stays in English inside Polish sentences (bufferbloat,
jitter, MTU, TCP autotuning, Nagle). That is how the terms are actually used,
and it keeps them searchable.

Note that `powercfg` and `netsh` translate *their* output too, so the code that
reads them matches a set of known labels and falls back to a structural rule —
position for power indices, stem matching for auto-tuning levels — rather than
one English label that would silently fail everywhere else.

## Tabs

- **Live**: opens with the answer in plain words: whether the internet
  works and since when (as far as the outage log and the watch so far can
  back it), a picture of the chain *this computer → router → internet* with
  each link marked working, slow, broken or unknown, and what to do about it.
  Every control sits on that panel: the one next step (open Diagnose, or save
  a report for the provider when the break is on their side), pausing, and
  the report. A link that was not measured is drawn as unknown, never as
  fine. Under it the headline figures, then the latency plot
  over a range you pick, from a minute to an hour,
  read from the database rather than from memory so it covers more than this
  run. Pointing at it reads out every probe at that instant, with no delay
  before the figures appear. Clicking a name in the legend takes that line
  away, which is the only way to follow one of four crossing lines. *Smooth*
  swaps each slice's range for its average: the first is what happened, the
  second is the shape of an hour, and neither is readable as the other.
  A red line through the chart means every internet target went silent at
  once; a reply lost by one target alone is a dot on the chart's floor, and a
  spike shared by every target a triangle on its top edge, so a noisy five
  minutes no longer hides the lines behind a fence of marks. Time the app
  spent closed leaves a gap and no accusation. Nothing on this tab expects you to already
  know what it means: each legend entry carries its target's current reading
  and explains which stretch of the path it measures, the threshold lines are
  labelled with their values, and the key under the plot keeps its prose
  behind a single question mark instead of running along the row. The
  headline figures (latency, jitter, loss, DNS, time since the last outage)
  each say on hover what they are and what a bad value there points at. The
  outage card counts breaks only, with periods of poor quality beside them,
  and opens the outage history when clicked.
  Folded away under *Technical details*: the path hop by hop with a verdict on
  which one the trouble starts at, and traceroute.
- **Diagnose**: nine checks: adapter and medium (on a cable, also whether
  it is corrupting frames and what speed it negotiated), Wi-Fi quality and band,
  adapter power management, DNS, the link to the router, internet latency and
  loss, MTU, TCP settings, and the recorded outage history; also whether IPv6
  works, is simply absent, or is set up and broken (the case that makes first
  loads slow), how your DNS compares with a public resolver asked the same
  question, and what Windows itself logged about the connection while the scan
  ran (a Wi-Fi drop logged on the card counts against the link; events that
  may belong to a VPN or virtual adapter are shown but never blamed). It also
  says when the traffic goes through a VPN, in which case every number includes
  the trip to the VPN server and the first thing to do is to scan without it,
  and when Windows sends programs through a proxy. It opens with the
  verdict, then draws the chain *this PC → router → provider → internet* with
  the milliseconds and loss each link added and the link at fault marked, then
  a table of every reading (samples sent, loss, min / avg / max, jitter per
  leg, DNS, TCP connect, the line's usual ping, the load test) so the verdict
  can be checked rather than taken on trust. A leg that was not measured says
  so. Each finding explains itself, and some link straight to the fix.
  For drops that come a few times an hour, *Measure for* adds two or five
  minutes of watching: every link pinged once a second on one clock, each bad
  second charged to the link where it started, drawn on a timeline and listed
  with the time to the second. A single lost second is recorded but blames
  nothing; drops found this way outrank the outage history in the verdict, and
  a quiet run does not clear that history. It can be stopped early and keeps
  what it recorded. A second scan shows what the one before it found and how
  the ping to the router and to the internet moved since, which is how to see
  whether a fix did anything. The report can be saved from the tab.
  Optionally, with your own [OpenRouter](https://openrouter.ai/keys) key in
  the settings, a button asks a language model (DeepSeek V4 Flash by default)
  to explain the result in plain words. Nothing is sent until you press it, the
  exact text sent can be read first, and your network's name, the access
  point's MAC and public addresses are masked. The answer is laid out in four
  parts (the problem, what it rests on, what to do, what the scan could not
  tell), and its steps are only ones you can take yourself or tell your
  provider. It is labelled as an interpretation; the verdict from the
  measurements stays the answer.
- **Speed test**: download and upload speed in Mbps and the idle ping,
  filled in live while the test runs, plus bufferbloat: idle latency
  versus latency with the link
  saturated, first downloading and then uploading, since on an asymmetric
  line the upload queue (video calls, cloud backups) is usually the worse
  one. Grade A–F from the worse direction, and specific advice. A good grade
  names the speeds it was measured at: if your plan is clearly faster, the
  test did not fill the line. This is usually the answer to
  "good ping, still lagging". A chart shows every ping of the test against
  time, so a spike the averages hide is visible, and a running test can be
  stopped without losing the previous result. It will not run beside a scan
  that loads the line too. **It costs data**: saturating the link is the
  measurement, so it moves data to and from `speed.cloudflare.com` on four
  streams for about 13 seconds each way, and a faster line therefore pays
  more. Roughly 130 MB of download on 100 Mbit/s, roughly 1.3 GB on gigabit,
  plus the upload. The result screen prints what it actually moved. On a
  metered or mobile connection, skip it.
- **Optimise** — every tweak grouped by what it is about (power and sleep,
  radio and range, names and addresses, throughput and latency, what else
  uses the link, last resort), each row saying in words whether it is set,
  worth changing, or not available on this machine — a Wi-Fi setting on a
  cable and a property the driver does not expose both say so rather than
  looking like a failure. A DNS server you run yourself (a Pi-hole, AdGuard
  Home, a company resolver) is left alone and never offered a replacement.
  Each tweak is a card with a switch: on applies it, off puts back what was
  there before, and one that is high risk or cannot be undone asks first.
  Clicking a card opens what it does and why; once a change has been
  applied, that also shows the line in the day before it against the time
  since (median ping, jitter, loss), or says there are too few readings; the
  figures are measured, not proof that the change did it. Above the cards,
  **Wi-Fi channel on your router** listens to the networks nearby and says
  which channel to set on the router, or that the one it is on is fine.
- **Outage history** — when, how long, whose fault, and on selecting an entry,
  why: cause, evidence, what Windows logged around it, the lead-up plotted, and
  a route to the fix. A line that stays *slow* (loss, jitter or latency past
  the thresholds in Settings) for three readings in a row is recorded here too,
  as "unstable"; it does not trigger a notification. An outage that starts slow
  and then goes down is recorded as the worst state it reached, once that
  state held for as many readings as opening an outage takes: a router that
  misses one ping to itself does not turn the provider's outage into yours.
  If the router answers UPnP (most do unless it is switched off), NetDoctor
  asks it every 30 seconds how its internet connection is doing, read-only.
  An outage then also says whether the router itself reported that
  connection as down, whether the router or its connection restarted during
  it, whether it came back with a new public address, or whether the router
  thought it was up throughout and the break was further out. A router that
  does not answer adds nothing, and silence is never read as "connected".
  **Save report** here writes a PDF for a support ticket over the range
  you pick (24 hours, 7 or 30 days, or everything kept): how much of it was
  actually watched, totals per kind of outage, and for each outage its cause
  and the evidence behind it, the Windows log around it, the path hop by hop
  as it was when it began, and the minute before it. Periods of poor quality
  follow as a list, one line each. Outages recorded before the app started
  storing the path with them have none, and say so. Each report is its own
  file, dated in its name, in Documents\NetDoctor unless you pick another
  folder in Settings.
  The list and the selected outage sit side by side. Its lead-up is two
  plots on one time axis: round trip to the router and to the internet in
  ms, with unanswered pings marked, and the Wi-Fi signal in dBm, with the
  outage shaded on both. **Clear history** deletes every finished outage
  after asking, and names the report as the way to keep a copy; an outage
  still in progress and the measurements behind the live chart are kept.
- **Settings**: your line (fibre, cable, DSL, radio, LTE / 5G, satellite, or
  don't know) and your plan's speeds, which the app cannot read for itself: the
  line sets the ping to expect until the app has its own history and fits the
  advice to it, and the speed test reports its result as a share of the plan.
  Also probe cadence (300 ms to 30 s; slower and a sleeping machine could not
  be told from an outage), thresholds, extra ping targets (your game server,
  for instance), the folder reports are saved in, autostart, updates. Split into pages, with the save
  bar pinned under every page: it says when something is not saved yet, and
  Ctrl+S saves.

## Running in the background

An outage is only recorded if the monitor is running when it happens. So:

- **Settings → Start with Windows** adds NetDoctor to the current user's `Run`
  key. No elevation needed, because monitoring does not need it.
- **Start minimised**, or `--minimised`.
- **An icon in the notification area** shows the line's state: green when it
  works, yellow when it is slow, red when it is down, grey when nothing is
  being measured (paused, no reading yet, the last reading is more than a few
  intervals old, Windows would not let the app send pings, or the readings
  could not be saved and a healthy line cannot be judged: none of these
  is recorded as an outage). Hover for the verdict, click to
  open the window, right-click for Quit.
- **The close button hides the window to that icon** and measuring goes on.
  Quit from the icon's menu to stop it.
- **Windows notifies you** when an outage is serious enough to be recorded
  (three failed readings in a row, and not merely slow), and again when the
  line comes back, with how long it was down. Turn it off in Settings.

## Game mode

NetDoctor notices League of Legends, Counter-Strike 2, VALORANT or
Hearthstone running (by process name, every few seconds) and then:

- **measures twice as often**, so a spike is caught while it happens;
- **shows an overlay** in a corner of the game's screen (top left unless
  Settings picks another; CS2 and VALORANT keep their minimap there): the
  router's and the
  internet's reply time, and whose side a spike is on: *your side* (Wi-Fi or
  router) when the router spiked with the internet, *past your router* (the
  line or the provider) when only the internet did. The internet reply is
  Cloudflare's or Google's, not the game server's: a lag on the game's own
  server does not show. A spike is at least 50 ms over the calm part of the
  last two minutes (the 20th percentile), held for two of the last three
  readings, so a lag that lasts a minute is still called one. Those numbers
  were chosen on real recorded readings: a first guess of +30 ms on any
  single reading would have warned 27 times an hour on a line that was sound
  97% of the time; these warn about 6 times an hour. While all is
  calm it shows only the two numbers and a green bar; a second line appears
  only for a spike or an outage. When a spike past the router comes while
  this PC is moving more than 2 Mbit/s (a match needs far less), it names
  both, because a download fills the line's queue and looks just like the
  provider. Which
  program, it cannot tell. The overlay is a separate click-through window, nothing is injected
  into the game, so anti-cheat has nothing to object to. It shows over
  borderless windowed; over exclusive fullscreen Windows may hide it. It shows
  only over these games, never over a browser or anything else, and can be
  turned off in Settings. Settings also set
  its visibility (20-100%), its size (small, medium, large; the window fits
  its text), and what it shows: ping and verdict, router and internet, or the
  internet alone. The compact choices drop the verdict, never an outage: while
  something is down, that is what the overlay says. These overlay settings
  take effect on the click, while the overlay is watched; the rest of
  Settings waits for Save.

It does not show the game server's ping: these games talk over UDP, CS2
through Valve's relays, and a ping measured from outside the game would be a
different number. The game shows its own.

**Prepare the connection** (tray menu, while a game runs, as administrator)
stops Windows Update, BITS and Delivery Optimization if they are running and
turns Wi-Fi power saving off if it is on. Only what it changed is written to
`game_session.json`, and exactly that is undone once neither the game nor
its client (the League client, the Riot client) has been running for 90
seconds, so the lobby between matches does not end it; from the tray menu;
or on the next start if the app was killed mid-game. While the session lasts,
a service Windows starts again on its own is stopped again. It can
only clear this computer's own traffic: a television streaming in the next
room is the router's to manage, and the speed test tells you whether it needs
to.

## Changes it can make

Each one records the previous value to
`%LOCALAPPDATA%\NetDoctor\tweak_snapshots.json` before touching anything, so
Revert restores the exact prior state — including after a reboot, which is when
it matters, since several of these only take effect after one.

| Change | Risk | Why |
| --- | --- | --- |
| Adapter power management | low | the most common cause of drops on a laptop |
| Wi-Fi power plan | low | a second, independent radio throttle |
| Fast DNS (1.1.1.1) | medium | removes the router as a single point of failure |
| TCP auto-tuning = normal | low | undoes the damage done by "ping boost" guides |
| Disable Nagle | medium | a few ms off games that talk over TCP; League of Legends, CS2 and VALORANT use UDP and gain nothing; needs a restart |
| Multimedia packet throttle | medium | Windows caps traffic during media playback |
| MTU correction | medium | oversized MTU means pages that never finish loading |
| Network stack reset | medium | rescue action when the link died; **not undoable** |

### Inside the radio

These are the entries on the adapter's Advanced tab in Device Manager, and they
are where the range actually lives. All of them take effect after a restart.

| Change | Risk | Why |
| --- | --- | --- |
| Transmit power to maximum | medium | laptops ship it lower; the router hears you worse than you hear it |
| Roaming aggressiveness to highest | medium | stops the card clinging to a mesh node it has walked away from |
| No power save on the radio | low | the stutter at the start of every call |
| No MIMO power save | low | a parked second antenna costs about 3 dB where it matters |
| 20 MHz channels on 2.4 GHz | medium | a 40 MHz link takes two of the three clean channels and collides with everything |
| Prefer 5 GHz | medium | far more throughput, far worse through walls — wrong if the signal is already weak |
| No interrupt moderation (wired) | medium | batching holds packets back for a fraction of a millisecond |
| No Green Ethernet / EEE (wired) | low | every renegotiation is a short disconnect |

The name of each property is the vendor's choice, and so is the number behind
each option, so nothing here is hardcoded: each tweak carries the names the
property goes by across vendors, takes the first the driver actually declares,
and then picks the option by matching the driver's own wording out of
`Ndi\Params\<name>\Enum`. "Highest transmit power" therefore stays "highest"
on a card that numbers it 1..5 and on one that numbers it 0..100.

### Deeper in the stack

| Change | Risk | Why |
| --- | --- | --- |
| No negative DNS caching | low | why names stay broken for five minutes after the link comes back |
| No Delivery Optimization uploads | low | Windows Update seeding to strangers over your uplink |
| No auto-connect to open hotspots | low | Windows leaving a working network for a hotspot that wants a login |
| BBR2 congestion control | high | CUBIC reads Wi-Fi interference as congestion and throws away throughput |
| Prefer IPv4 over IPv6 | high | an ISP handing out IPv6 that does not route makes every page wait out a timeout |

"Apply all safe changes" only touches low-risk, reversible items that are not
already correct, so nothing in the medium or high rows is ever applied without
being asked for by name.

## Which channel to ask the router for

Windows cannot change the router's channel — that setting lives in the router,
and no API on this side reaches it. What the client can do is measure. Every
access point in range beacons its channel and its signal strength, so Optimise
scans for them and sums the interference each candidate channel would suffer,
as power rather than as a count: one neighbour at -45 dBm hurts more than six
at -85, and counting rows says the opposite.

The result is a sentence to type into the router's admin page — "use 2.4 GHz
channel 11, it is 9 dB quieter than 6" — with the evidence next to it. Our own
access points are excluded from the interference, since a mesh is not competing
with itself. On 5 GHz only radar-free channels are suggested: a DFS channel can
be perfectly quiet and still cut the network for a minute when the router thinks
it heard radar, which is exactly the outage this app exists to explain.

## Why the Windows APIs, not the command-line tools

The original prototype (kept in `legacy-python/`) shelled out to `ping.exe`,
`netsh`, `ipconfig`, `powercfg` and `reg.exe`, and parsed their output. That
approach has three problems this version does not:

- **Cost.** `ping.exe` needs about 40 ms of process startup per sample.
  `IcmpSendEcho` needs none.
- **Resolution.** `ping.exe` reports whole milliseconds, so on a LAN every
  sample collapses to "0 ms" or "1 ms" and jitter becomes fiction. Timing the
  API call gives two decimal places — the router here measures 3.47 ms, not "3".
- **Silent breakage.** A parser keyed on English output finds nothing on a
  Polish install, and *finding nothing looks exactly like finding no problem*.
  A monitor that reports a healthy connection during an outage is worse than no
  monitor. Structs cannot be mistranslated.

The same reasoning applies to `WlanQueryInterface` over `netsh wlan` (it also
yields RSSI in dBm, which `netsh` never prints) and to the registry API over
`reg.exe` (an absent value is distinguishable from access denied, so a tweak can
say "not set" rather than "failed").

## Tests

```text
cargo test
```

387 tests, covering the failure-blame logic, the statistics, the registry layer,
settings migration, and the ICMP status-code mapping. Several run against the
live machine — a `ping_once` to loopback must succeed, a reserved address must
fail without hanging, and a full diagnostic scan must produce presentable
findings.

Seven of them are marked `#[ignore]` and do not run by default. Two are timing
benchmarks that only mean something in a release build on an otherwise quiet
machine; two print what the live adapter is doing and are there to be read, not
to pass or fail. Run them on purpose:

```text
cargo test --release -- --ignored --nocapture
```

Three check the notification-area icon and need a desktop session. One of
those cuts the Wi-Fi for about ten seconds to produce a real outage, and checks
that it is announced, that its end is announced promptly, and that the length
recorded matches the cut. It writes to a scratch database, and needs to know
what to reconnect to:

```text
set NETDOCTOR_TEST_WIFI_IFACE=WiFi
set NETDOCTOR_TEST_WIFI_PROFILE=<your network's profile name>
cargo test a_real_outage -- --ignored --nocapture
```

## Data

`%LOCALAPPDATA%\NetDoctor\`

- `history.db` — samples, outage events, tweak log, the router's UPnP
  readings (WAL mode). The router's public address is stored only as a
  fingerprint, enough to tell a new address from the same one
- `settings.json` — your settings; unknown keys from older versions are ignored.
  If you set an OpenRouter key, it is stored here in plain text
  (`OPENROUTER_API_KEY` in the environment works too and keeps it out of the file)
- `tweak_snapshots.json` — prior state for every applied change

Measurements older than the configured retention (14 days by default) are
pruned automatically. Recorded outages are kept for a year, so a report can
reach back further than the measurements do; past the retention they lose the
sweeps that led up to them, which are most of their size, and keep the time,
the kind, the state they failed on and the path. An outage still in progress
is never pruned. The log of changes the app made is kept.

`history.db` is the one file here that gets big. At the defaults — four targets,
one sweep a second — a full 14 days of retention measures **367 MB**. That is
the steady state, not a leak: pruning keeps it there. If it is more than you
want to spend, the retention setting is the dial, and dropping to seven days
roughly halves it.

## Building

```text
cargo build --release
```

Produces `target/release/netdoctor.exe`, about 5.3 MB, with no external
dependencies. Requires Rust 1.82+ and the MSVC toolchain.

`.cargo/config.toml` links the C runtime statically. Without it the binary
imports `vcruntime140.dll` and will not start on a machine that lacks the
Visual C++ Redistributable — which is the wrong thing to discover on a machine
whose network is already broken. The `api-ms-win-crt-*` imports that remain are
the UCRT, part of Windows itself since Windows 10.

To hand the app to someone, ship that executable on its own. It needs no
installer and writes nothing outside `%LOCALAPPDATA%\NetDoctor\`.

### Cutting a release

```text
# bump `version` in Cargo.toml first
git tag v1.2.0
git push origin v1.2.0
```

`.github/workflows/release.yml` builds on Windows, runs clippy and the tests,
and attaches `netdoctor.exe` and `SHA256SUMS` to a GitHub release. It refuses
to publish a tag that disagrees with the version in `Cargo.toml`: the updater
compares the tag against the version baked into the running binary, so a
mismatch ships a build that either re-offers itself forever or never updates.

Running the workflow by hand from the Actions tab does everything except
publish, and leaves the build as an artifact — useful for checking the release
path before cutting a tag.

## Limitations

- Windows only. The optimisation layer is inherently Windows-specific; the
  diagnostics would port, the tweaks would not.
- The MTU probe relies on DF-flagged ICMP, which some paths rate-limit or block.
  Re-run it before acting on the result.
- Memory use is around 70 MB — higher than the Tk prototype, because egui keeps
  a GPU surface. The trade was made for a single dependency-free binary, not for
  a smaller footprint.
- Measurements are IPv4. IPv6 is checked only for whether a connection gets
  through, not measured for latency or loss.
