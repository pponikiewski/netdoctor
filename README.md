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

## Running it

```text
netdoctor.exe              # the app
netdoctor.exe --scan       # run the diagnostic scan on the console and exit
netdoctor.exe --minimised  # start minimised (used by autostart)
netdoctor.exe --help
```

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

- **Live** — latency plot with a time axis and gaps where packets were lost,
  headline figures (latency, jitter, loss, DNS, time since the last outage),
  traceroute, report export. Hovering a legend entry shows that probe's current
  state.
- **Diagnose** — nine checks: adapter and medium, Wi-Fi quality and band,
  adapter power management, DNS, the link to the router, internet latency and
  loss, MTU, TCP settings, and the recorded outage history. Each finding
  explains itself, and some link straight to the fix.
- **Load test** — bufferbloat: idle latency versus latency with the link
  saturated. Grade A–F and specific advice. This is usually the answer to
  "good ping, still lagging".
- **Optimise** — every tweak grouped by what it is about (power and sleep,
  radio and range, names and addresses, throughput and latency, what else
  uses the link, last resort), each row saying in words whether it is set,
  worth changing, or not available on this machine — a Wi-Fi setting on a
  cable and a property the driver does not expose both say so rather than
  looking like a failure. Plus a scan of the surrounding Wi-Fi that works
  out which channel to ask the router for.
- **Outage history** — when, how long, whose fault, and on selecting an entry,
  why: cause, evidence, the lead-up plotted, and a route to the fix.
- **Settings** — probe cadence, thresholds, extra ping targets (your game
  server, for instance), autostart.

## Running in the background

An outage is only recorded if the monitor is running when it happens. So:

- **Settings → Start with Windows** adds NetDoctor to the current user's `Run`
  key. No elevation needed, because monitoring does not need it.
- **Start minimised**, or `--minimised`.
- When the verdict changes, a banner names what broke and on whose side.

## Changes it can make

Each one records the previous value to
`%LOCALAPPDATA%\NetDoctor\tweak_snapshots.json` before touching anything, so
Revert restores the exact prior state — including after a reboot, which is when
it matters, since several of these only take effect after one.

| Change | Risk | Why |
| --- | --- | --- |
| Adapter power management | low | the most common cause of drops on a laptop |
| Wi-Fi power plan | low | a second, independent radio throttle |
| Fast DNS (1.1.1.1) | low | removes the router as a single point of failure |
| TCP auto-tuning = normal | low | undoes the damage done by "ping boost" guides |
| Disable Nagle | medium | a few ms off twitch games; needs a restart |
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

43 tests, covering the failure-blame logic, the statistics, the registry layer,
settings migration, and the ICMP status-code mapping. Several run against the
live machine — a `ping_once` to loopback must succeed, a reserved address must
fail without hanging, and a full diagnostic scan must produce presentable
findings.

## Data

`%LOCALAPPDATA%\NetDoctor\`

- `history.db` — samples, outage events, tweak log (WAL mode)
- `settings.json` — your settings; unknown keys from older versions are ignored
- `tweak_snapshots.json` — prior state for every applied change
- `netdoctor-report.txt` — written by "Save report"

Samples older than the configured retention (14 days by default) are pruned
automatically.

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

## Limitations

- Windows only. The optimisation layer is inherently Windows-specific; the
  diagnostics would port, the tweaks would not.
- The MTU probe relies on DF-flagged ICMP, which some paths rate-limit or block.
  Re-run it before acting on the result.
- Memory use is around 70 MB — higher than the Tk prototype, because egui keeps
  a GPU surface. The trade was made for a single dependency-free binary, not for
  a smaller footprint.
- IPv4 only.
