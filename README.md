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
was still answering. That is what makes a 3 a.m. dropout explainable the next
morning, and what turns "my internet is bad" into evidence an ISP has to engage
with.

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
- **Optimise** — every tweak with its current state and risk level.
- **Outage history** — when, how long, and whose fault.
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

"Apply all safe changes" only touches low-risk, reversible items that are not
already correct.

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
