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

## Architecture

It is a Cargo workspace. The **core crates are platform-neutral**, so they build, test and render screenshots on Linux. Only the adapter crates touch Win32.

```
crates/
  model/        Section, Metric, Sample, Probe, Target, Hop, Verdict types. No logic.
  history/      Deep module: RAM ring buffer + SQLite Rollups behind one interface.
  verdict/      Deep module: pure fn diagnose(&Snapshot) -> Verdict.
  render/       Deep module: draw a Meter or the Strip to an RGBA image (tiny-skia).
  panel/        egui Panel: takes a read-only view of History, Verdict and Process Snapshot.
  config/       TOML load/save, defaults, migration.
  collect-win/  Adapters: Metrics (PDH, GetSystemTimes, GetIfTable2, NtQuerySystemInformation).
  probe-win/    Adapters: ICMP (IcmpSendEcho2), TCP connect (+ TTL/ICMP error info), UDP DNS.
  surface-win/  Adapters: Tray Icons (tray-icon), Taskbar Strip (windows-rs, layered window).
  fixtures/     Synthetic and recorded Samples/Probe results; fake adapters for tests.
app/            blinkengraphs.exe: wiring, scheduler, power/lock handling, hotkey.
tools/screenshots/  Renders every visual from fixtures to PNG (runs on Linux + Windows).
spikes/tcp-hop/ M0 experiment, deleted once the decision is recorded.
```

### Seams and interfaces

**Collector** (`collect-win`, fake in `fixtures`)
- Interface: `sample(&mut self, enabled: &MetricSet) -> Sample`.
- Hides: PDH query handles, counter deltas, adapter following, and Process Snapshot enumeration (`snapshot_processes()`, called only while the Panel is open).
- It's a real seam because it has two adapters: Windows and the fixture replay.

**Prober** (`probe-win`, fake in `fixtures`)
- Interface: `start(groups: Vec<ProbeGroup>) -> Receiver<ProbeResult>` and `reconfigure(...)`.
- Hides: threads, timeouts, ICMP handle reuse, TTL sweeps for Hop Traces, Gateway discovery and DNS packet encoding.

**History**
- Interface: `record(Sample | ProbeResult)` and `series(metric_or_probe, range) -> Series`.
- The caller never knows whether a range comes from RAM or from SQLite Rollups.

**Verdict**
- `diagnose(&Snapshot) -> Verdict`. It is pure and table-tested with fixture scenarios:
  - Wi-Fi drop
  - ISP loss
  - Slow DNS
  - ICMP filtered while TCP is fine
  - Upload saturation
  - CPU pinned

**Renderer**
- `render_meter(&MeterSpec, &Series, size) -> Image` and `render_strip(...)`.
- Shared by both Surfaces and by the screenshots tool, so CI screenshots are pixel-identical to the app.

**Surface** (`surface-win`)
- Interface: `show(&[Image])` and `events() -> Receiver<SurfaceEvent>` (click, menu, anchor rect for the Panel).
- Two adapters: Tray Icons and Taskbar Strip. The app swaps to Tray Icons when the Strip reports `Detached`.

## Milestones

**M0: skeleton, pipeline, spike**
- Workspace, `cargo-xwin` cross-compiling on Linux, and CI on `windows-latest` that builds x64 + ARM64.
- `render` crate + `tools/screenshots` + publishing (orphan `ci-screenshots` branch, sticky PR comment, job summary).
- release-please, the rolling `main-latest` prerelease, build attestations.
- **TCP Hop Trace spike on CI**, with the result recorded as an ADR.

**M1: Tray Icons, Probes, Panel**
- Collector, Prober (ICMP/TCP/DNS, ICMP Hop Trace), in-memory History, Verdict.
- Five Tray Icons, the egui Panel with live graphs and the Verdict, the menu for Metrics/Probes/intervals, TOML config, the hotkey, pausing when the display is off or the session is locked.
- Measure the CPU/RAM budget.

**M2: Taskbar Strip, persistent History** (M1 + M2 = v1.0)
- Taskbar Strip with fallback, SQLite Rollups, history ranges in the Panel.

**M3: Elevated Extras and distribution**
- Bytes per process on the network (ETW), UDP Hop Traces, autostart via HKCU Run, SignPath Authenticode, installer if wanted.

## Risks / open items

- The Win11 Taskbar Strip may break on Windows updates; mitigated by fallback (ADR 0001).
- Win11 hides new tray icons in overflow by default. A first-run hint tells the user to drag them out; low-confidence source.
- Corporate firewall/EDR may block ICMP or flag taskbar re-parenting. Running TCP and DNS Probes side by side keeps Latency meaningful.
- `IcmpSendEcho2` RTT has whole-millisecond resolution (docs/research/01-prior-art.md §3). The TCP handshake can be timed with QPC for sub-ms precision.
- Rendering the Panel to PNG headlessly (egui offscreen, e.g. `egui_kittest`) is unverified; the fallback is to screenshot Meters only.
