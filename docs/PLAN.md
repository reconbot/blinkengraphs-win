# blinkengraphs: plan

The vocabulary is defined in [`CONTEXT.md`](../CONTEXT.md). The decisions are recorded in [`docs/adr/`](adr/). The research behind them is in [`docs/research/`](research/).

## Goal

A Rust app for Windows 11 (x64 + ARM64) that shows the Latency, CPU, Memory, Network and Disk Sections as Meters you can always see, and a Panel that explains a slowdown or a network failure with a Verdict.

Budget: under 1% CPU and about 30 MB RAM when idle with the Panel closed. These are targets, to be measured in M1.

## Decisions (from the design session, 2026-11-21)

| Area | Decision |
|---|---|
| Audience | The author first, ready for fleet use later. Settings are changed in the app's menus and saved as TOML |
| Privileges | Never requires admin; Elevated Extras switch on only when the app is already elevated ([ADR 0002](adr/0002-no-admin-by-default.md)) |
| Surfaces | Tray Icons, one per Section, each can be hidden. The Taskbar Strip is in v1 and, when active, replaces the Meter icons with one Menu Icon. It falls back to Tray Icons ([ADR 0001](adr/0001-tray-icons-supported-strip-unsupported.md)) |
| Metrics | Each Section has several Metrics, each switchable per Surface. Defaults are listed below |
| Probes | The user chooses the Probes (protocol × Target). Defaults are listed below |
| Cadence | Each Probe Group has its own interval, default 1 s. Everything pauses while the display is off or the session is locked |
| Hop Trace | Continuous, at 1 s by default. ICMP needs no admin. TCP depends on the M0 spike. UDP is an Elevated Extra |
| Verdict | Rule-based, shown at the top of the Panel and in the tooltip |
| Processes | Process Snapshot only while the Panel is open ([ADR 0003](adr/0003-process-snapshot-only-while-panel-open.md)) |
| History | 15 min of Samples in RAM, plus 7 days of Rollups in SQLite |
| Alerts | None in v1; Meters change color when thresholds are crossed |
| Panel UI | egui + egui_plot with a custom dark theme and Mica; opens with a configurable global hotkey |
| Dev loop | Cross-compile on Linux (`cargo-xwin`); GitHub Actions on Windows builds, tests and takes screenshots; no local Windows builds |
| Releases | release-please plus a rolling `main-latest` prerelease; build attestations; SignPath later ([ADR 0005](adr/0005-release-please-and-rolling-prerelease.md)) |
| Screenshots | Rendered headless from fixtures. Shown in a sticky PR comment and the job summary, and fetched here from the `ci-screenshots` branch ([ADR 0004](adr/0004-ci-screenshots-from-headless-rendering.md)) |
| License | MIT OR Apache-2.0. GPL tools (mtr, WinMTR, MenuMeters, Rainmeter) are studied for technique only |

### Default Metrics per Section (★ = on the Meter by default; all can be switched on or off)

| Section | Metrics |
|---|---|
| CPU | ★ total %, kernel share, per-core (Panel), processor queue length |
| Memory | ★ commit charge % (Memory Pressure), ★ hard faults/s (as color), physical in-use %, compressed |
| Network | ★ download/upload throughput on the Followed Adapter (cube-root scale), errors/discards, per-adapter (Panel) |
| Disk | ★ active time %, read/write MB/s, average response ms, queue length |
| Latency | ★ RTT to the primary Target, ★ loss (red bars), jitter, per-Probe lines (Panel), Hop Traces (Panel) |

### Default Probe Groups

| Group | Probes | Interval |
|---|---|---|
| Reachability | ICMP echo + TCP 443 handshake → Gateway, 1.1.1.1, 8.8.8.8 | 1 s |
| DNS | UDP DNS query → 1.1.1.1 and the system resolver | 1 s |
| Hop Trace | ICMP (and TCP if the spike succeeds) → 1.1.1.1 | 1 s |
| System | All Section Metrics | 1 s |
| Process Snapshot | Top processes per Section, only while the Panel is open | 1 s |

### Decisions from the M1 review

- A Meter graphs the first enabled Metric in its Section (Network graphs download and upload together). Metrics are kept in catalog order, so ticking order doesn't change which one is drawn.
- The Latency Meter always shows RTT and loss for the primary Probe. Jitter and the per-Probe lines are Panel-only, so the Latency Section has no Metric toggles.
- Memory "compressed" is dropped: it has no cheap counter without admin.
- Gateway Probes are ICMP only, because home routers rarely accept TCP 443.
- Hop rows are kept in RAM only; history ranges apply to Probes and Metrics.
- The Panel uses a rounded dark Win11 frame. A Mica backdrop is deferred, because a failed backdrop on a transparent window would leave the window invisible.
- Pause triggers: session locked, screen saver running, or display off (`GUID_CONSOLE_DISPLAY_STATE`).
- The Gateway and system resolver follow the default route even when the Network adapter is pinned.

## Architecture

Platform-neutral crates build, test and render on Linux; only `collect`, `probe` and `app` call Win32 (behind `cfg(windows)` with Linux stubs).

```
crates/
  model/     Section, Metric, Sample, Probe, Target, ProbeStat, Hop types
  config/    TOML schema, defaults, load/save
  history/   RAM ring + SQLite Rollups; probe stats; mtr hop rows
  verdict/   diagnose(Input) -> Vec<Finding>
  collect/   Collector: PDH, GetIfEntry2, NtQuerySystemInformation
  probe/     Prober: ICMP (IcmpSendEcho2), TCP connect, UDP DNS, hop traces
  render/    Meter and Strip images (tiny-skia, ab_glyph)
  meters/    History + Config -> Meter/Strip images and tooltips
  panel/     egui Panel and theme
  fixtures/  synthetic scenarios for tests and screenshots
app/         engine thread, Tray Icons, Taskbar Strip window, Panel window, hotkey
tools/screenshots/  renders Meters, Strip and Panel from fixtures to PNG
```

Seams:
- `Collector` and `Prober` are the only adapters to the OS; the engine thread owns both and writes into `History`.
- `verdict::diagnose` is pure and table-tested.
- `meters` and `panel` read `History`; the same code draws the app and the CI screenshots.

## Milestones

See [`README.md`](../README.md#milestones).

## Risks / open items

- The Win11 Taskbar Strip may break on Windows updates; mitigated by fallback (ADR 0001).
- Win11 hides new tray icons in overflow by default. A first-run hint tells the user to drag them out; low-confidence source.
- Corporate firewall/EDR may block ICMP or flag taskbar re-parenting. Running TCP and DNS Probes side by side keeps Latency meaningful.
- `IcmpSendEcho2` RTT has whole-millisecond resolution (docs/research/01-prior-art.md §3). The TCP handshake can be timed with QPC for sub-ms precision.
- Rendering the Panel to PNG headlessly (egui offscreen, e.g. `egui_kittest`) is unverified; the fallback is to screenshot Meters only.
