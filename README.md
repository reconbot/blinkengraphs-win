# blinkengraphs

Live CPU, memory, disk, network and latency graphs in the Windows 11 taskbar, inspired by iStat Menus, MenuMeters, gping and mtr. Click a graph to see a plain-language verdict on why the machine is slow or the network isn't responding.

**Download:** the [`main-latest`](https://github.com/reconbot/blinkengraphs/releases/tag/main-latest) prerelease, rebuilt on every push to `main`, or a [tagged release](https://github.com/reconbot/blinkengraphs/releases). Builds are x64 and ARM64. They are not Authenticode-signed, so SmartScreen will warn. Check a download with `gh attestation verify blinkengraphs-x64.exe -R reconbot/blinkengraphs`.

## What it shows

| Section | Meter (always visible) | Panel (on click) |
|---|---|---|
| CPU | total % | per-core, kernel share, top processes |
| Memory | commit charge, hard faults | physical, compressed, top processes |
| Network | down/up on the followed adapter | per adapter, errors |
| Disk | active time % | MB/s, response time, queue |
| Latency | RTT + loss to 1.1.1.1 | ICMP/TCP/DNS probes to gateway, 1.1.1.1, 8.8.8.8; per-hop trace |

Every metric, probe and interval is configurable. Nothing needs Administrator; when run elevated, it adds per-process network bytes and UDP hop traces.

## Milestones

- **M0: skeleton.** Status: in progress.
  - [x] Workspace; cross-compile from Linux (`cargo xwin build --target x86_64-pc-windows-msvc`)
  - [x] Meter renderer and fixture screenshots
  - [x] Tray demo that replays a fixture
  - [ ] Windows CI for x64 and ARM64, with screenshots in the PR comment and job summary
  - [ ] release-please, the rolling `main-latest` prerelease, and build attestations
  - [ ] TCP hop-trace spike, with the result recorded as an ADR
- **M1: Tray Icons, Probes, Panel.**
  - Real collectors
  - ICMP/TCP/DNS probes and ICMP hop trace
  - Verdict
  - egui Panel
  - Settings menu and TOML config
  - Hotkey
  - Pause when the display is off or the session is locked
- **M2: Taskbar Strip and History** (M1 + M2 = v1.0).
  - Strip docked in the taskbar, falling back to tray icons
  - 7 days of per-minute history in SQLite
- **M3: Elevated Extras and distribution.**
  - Per-process network bytes and UDP hop trace
  - Autostart
  - SignPath signing

## Docs

- [`docs/PLAN.md`](docs/PLAN.md): decisions, defaults, architecture
- [`CONTEXT.md`](CONTEXT.md): vocabulary
- [`docs/adr/`](docs/adr/): why things are the way they are
- [`docs/research/`](docs/research/): prior art and the Rust/Windows stack, with sources

## License

MIT OR Apache-2.0
