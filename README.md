# blinkengraphs

Live CPU, memory, disk, network and latency graphs in the Windows 11 taskbar, inspired by iStat Menus, MenuMeters, gping and mtr. Click a graph to see a plain-language verdict on why the machine is slow or the network isn't responding.

**Download:** the [`main-latest`](https://github.com/reconbot/blinkengraphs-win/releases/tag/main-latest) prerelease, rebuilt on every push to `main`, or a [tagged release](https://github.com/reconbot/blinkengraphs-win/releases). Builds are x64 and ARM64. They are not Authenticode-signed, so SmartScreen will warn. Check a download with `gh attestation verify blinkengraphs-x64.exe -R reconbot/blinkengraphs-win`.

## What it shows

| Section | Meter (always visible) | Panel (on click) |
|---|---|---|
| CPU | total % | per-core, kernel share, top processes |
| Memory | commit charge, hard faults | physical, compressed, top processes |
| Network | down/up on the followed adapter | per adapter, errors |
| Disk | active time % | MB/s, response time, queue |
| Latency | RTT + loss to 1.1.1.1 | ICMP/TCP/DNS probes to gateway, 1.1.1.1, 8.8.8.8; per-hop trace |

Every metric, probe and interval is configurable. Nothing needs Administrator; when run elevated, it adds per-process network bytes and UDP hop traces.

## Screenshots

Rendered by CI from fixture data on every push to `main`.

![Panel during ISP packet loss](https://raw.githubusercontent.com/reconbot/blinkengraphs-win/ci-screenshots/main/panel-isp-loss-overview.png)
![Network tab with hop trace](https://raw.githubusercontent.com/reconbot/blinkengraphs-win/ci-screenshots/main/panel-isp-loss-network.png)

Taskbar Strip:

![Taskbar Strip during upload saturation](https://raw.githubusercontent.com/reconbot/blinkengraphs-win/ci-screenshots/main/strip-upload-saturation.png)

Tray Icons (actual size, then 4× zoom):

![Tray Icons during ISP loss](https://raw.githubusercontent.com/reconbot/blinkengraphs-win/ci-screenshots/main/isp-loss.png)

## Usage

- Click any Meter, or press `Ctrl+Shift+Alt+M`, to open the Panel. Click away or press `Esc` to close it.
- Right-click a Meter for Open, Settings and Quit.
- Settings live in `%APPDATA%\blinkengraphs\config.toml`; the Settings tab edits them live. History is stored in `history.db` in the same folder.
- Windows 11 hides new tray icons in the overflow menu: drag them onto the taskbar, or switch Surface to **Taskbar Strip**.

## Milestones

- [x] **M0: skeleton.** Cargo workspace, Windows CI for x64 and ARM64, screenshots from fixtures, release-please, rolling `main-latest`, TCP hop-trace spike ([ADR 0006](docs/adr/0006-tcp-hop-traces-experimental.md))
- [x] **M1: Tray Icons, Probes, Panel.**
  - Collectors (PDH, IP Helper) and ICMP/TCP/DNS probes with an ICMP hop trace
  - Verdict
  - egui Panel with Settings
  - Hotkey
  - Pause while locked, display off or screen saver running
- [x] **M2: Taskbar Strip and History.**
  - Strip docked in the taskbar, falling back to Tray Icons
  - 7 days of one-minute Rollups in SQLite, browsable in the Panel's history ranges
- [ ] **M3: Elevated Extras and distribution.** Per-process network bytes and UDP hop traces when elevated; autostart; SignPath signing; installer

## Docs

- [`docs/PLAN.md`](docs/PLAN.md): decisions, defaults, architecture
- [reconbot/blinkengraphs](https://github.com/reconbot/blinkengraphs): shared spec (vocabulary, Verdict rules, test vectors, design tokens) and the cross-platform plan
- [`docs/adr/`](docs/adr/): why things are the way they are
- [`docs/research/`](docs/research/): prior art and the Rust/Windows stack, with sources

## License

MIT OR Apache-2.0
