# 01 — Prior art: menubar/taskbar system meters and latency graphs

Research date: 2026-11-21. I checked primary sources where I could: vendor sites, Microsoft Learn, and GitHub repos. I read the GitHub repos as shallow clones at HEAD on 2026-11-21. Bracketed numbers like [S1] point to the Sources list. **Confidence** is marked where it is not high.

Product goal: a Rust Windows 11 app in the taskbar/notification area that shows live mini-graphs of CPU, memory, network throughput and ping latency to 1.1.1.1. It should answer two questions: "why is my computer slow?" and "why isn't my network responding?"

---

## 1. iStat Menus (Bjango, macOS)

- **Current version:** 7.5, released 14 Sep 2026. The 7.5 release notes still say "More details coming soon." The previous release was 7.3 (14 May 2026). It requires macOS 13.5+ [S1][S2].
- **Meters (menu bar items):** CPU and GPU, Memory, Disks, Network, Sensors, Battery/Power, Weather, Date and time, and Astronomy (new in 7.5). There is also **Combined**, which merges several items into one menu bar item [S1][S3].
- **Dropdown panels** [S1]:
  - CPU: per-core usage, history graphs, load averages, uptime, frequency, and **top apps by CPU**.
  - Memory: pressure, compressed, swap, and **top apps by memory**.
  - Network: history graphs, current bandwidth, **top apps by bandwidth**, connection details, public and private IPs, and a **connectivity indicator**.
  - Disks: space, activity, S.M.A.R.T., and per-app disk usage.
- **History ranges:** 7.0 added 10 min, 1/3/6/12 h and 1/3/7/14/28 days [S2]. Graphs rescale automatically so peaks always fit. Each hourly sample averages a 6-second period. Nothing is recorded during sleep, which shows as gaps [S4].
- **Rules/notifications:** These cover CPU, GPU, memory, disk, network, sensors and battery. Examples include "internet connection down", "public IP change" and "CPU above X% for N time" [S1].
- **Menu bar styles:** 7.3 added an "interface icon" menu bar mode for network [S2]. Bjango's help pages (bjango.com/help/istatmenus7/…) returned HTTP 403 to my fetcher. **Low confidence** on the full list of menu bar graph styles; it is not documented in what I could read.
- **What makes the UX good** (my synthesis of the above):
  1. The menu bar item is tiny and glanceable.
  2. One click opens a dense panel with *attribution* (top apps per resource) and *history* (multiple ranges).
  3. Rules handle the "tell me when it goes wrong" case.
  4. Combined mode saves menu bar space.

## 2. MenuMeters (yujitach fork, macOS)

- **Version and license:** Latest tag `2.1.7`, made by the commit "version bump to release" on 2026-10-28 [S5][S6]. The bundled `releases.html` changelog stops at 2.1.6.1 (13 Nov 2021) [S6]. License: **GPL-2.0** [S6].
- **Status:** The author's own README recommends using exelban/stats, iGlance or eul instead on modern macOS, because those are "maintained actively" [S6].
- **Meters:** CPU (with top-processes and uptime), Disk (I/O and space), Memory, Network. Source dirs are `MenuExtras/MenuMeter{CPU,Disk,Mem,Net}` [S6].
- **Display modes**, from the source enums [S6]:
  - CPU: `Percent | Graph | Thermometer | HorizontalThermometer`. These are bit flags, so they can be combined. Percent has Large, Small and Split variants. Graph length and multi-core rows are configurable.
  - Memory: `Pie | Bar | Graph | Number`.
  - Network: `Arrows | Throughput | Graph`. These are bit flags, with TX/RX orientation. **Scale:** interface speed or peak traffic, using a linear, square-root, **cube-root (default)** or log scale curve. The cube-root default is a useful idea for throughput graphs whose range covers several orders of magnitude.
- **How it draws:** It originally used `NSMenuExtra` loaded by SystemUIServer. That broke on El Capitan because SystemUIServer stopped loading non-Apple-signed extras. The fork runs as a faceless app with one `NSStatusItem` per meter [S6]. Each meter renders an `NSImage` (`lockFocus` → draw → `unlockFocus`) and assigns it to `statusItem.button.image` on every update (`MenuMetersMenuExtraBase.m`) [S6]. This is the macOS equivalent of a Windows tray-icon redraw.

## 3. gping (orf/gping)

- **Version and license:** Workspace version `gping 1.21.0`. The latest tag is `gping-v1.21.0` and the last commit was 2026-09-22. License: **MIT** [S7].
- **Features** [S7]:
  - Multiple hosts on one graph.
  - `--cmd` mode graphs command execution time.
  - `--tcp` mode does a TCP-connect "ping", with `--tcp-port` (default 80) and `--tcp-rst` to choose how a reset counts.
  - `-b/--buffer` sets the window in seconds (default 30).
  - `-n/--watch-interval` defaults to 0.2 s for ping and 0.5 s for cmd.
  - `--ymin/--ymax/-0` fix the axis.
  - `-s` switches from braille to dot graphics.
  - `-c` sets colors per series.
  - Cloud-region shorthands such as `aws:eu-west-1`.
  - Rendered as a terminal UI with `ratatui 0.30`.
- **How it pings on Windows:** It uses the `pinger` crate (same repo, v2.2.0, MIT). On `cfg(windows)` that crate depends on **`winping = "0.10.1"`** [S8]. `pinger/src/windows.rs` resolves the hostname, then calls `WinPinger::send(ip, &mut buffer)` in a loop on a thread. It reports `Duration::from_millis(rtt)`, so **RTT has whole-millisecond resolution on Windows**. The Windows ICMP echo API reports round-trip time in milliseconds [S9]. Other OSes shell out to `ping` and parse its output with a regex [S8].
- **Relevance:** gping supplies the graph idea and a known-good no-admin ICMP path on Windows. It is a full terminal UI rather than an ambient display, and it keeps no long history.

## 3a. mtr, WinMTR and Trippy: per-hop latency (traceroute + ping)

mtr combines traceroute and ping. It sends probes with increasing TTL, collects the ICMP "TTL exceeded" reply from each router, and keeps running, so you get **loss and latency per hop** over time. Together with gping-style graphs this gives the "where on the path does it break?" view the user wants.

- **mtr** (traceroute/mtr): **GPL-2.0**. Last commit 2026-11-02 (`e8ff9ca`) [S36]. Its README says using mtr on Windows **requires WSL**; the alternative is a Cygwin build (`packet/probe_cygwin.c`) [S36].
- **WinMTR** (White-Tiger fork): **GPL-2.0**, last commit 2014-01-13, so effectively unmaintained [S37]. The useful technique: `src/WinMTRNet.cpp` sets `IP_OPTION_INFORMATION.Ttl` per hop (lines 199/263) and calls **`IcmpSendEcho2`** (line 216). That is the same **no-admin** ICMP API gping uses [S9], so an ICMP-only mtr is possible on Windows **without elevation**.
- **Trippy** (fujiapple852/trippy): Rust, **Apache-2.0**. Latest tag `0.13.0`, last commit 2026-11-19 [S38]. It is an mtr-style terminal UI with ICMP/UDP/TCP tracing and per-hop charts. Its privileges guide says: "Trippy must be run with Administrator privileges on Windows", and unprivileged mode is unsupported on Windows (no `IPPROTO_ICMP` socket type) [S39]. On Windows its raw-socket path is `crates/trippy-core/src/net/platform/windows.rs` [S38].
- **Design consequence** (my inference): implement per-hop tracing as **ICMP via `IcmpSendEcho2` with TTL** for the non-admin default. Turn on **TCP/UDP per-hop tracing only when elevated**, the same opt-in as per-process network bytes. Trippy (Apache-2.0, license-compatible) is the best reference for hop/flow bookkeeping. WinMTR and mtr are GPL, so they are reference for technique only.

## 4. Existing Windows equivalents

| Tool | Maintained? (last release/commit) | License | Shows | Graph **in** taskbar/tray? | Technique |
|---|---|---|---|---|---|
| **TrafficMonitor** (zhongyang219) | Yes. V1.86, 2026-03-29; commit 2026-09-27 [S10][S11] | MIT text with an "Anti 996" clause appended (custom; check before reusing code) [S11] | Net up/down, CPU, RAM, GPU, disk; temps (Standard edition; via plugin); plugins; traffic history [S10] | **Yes**: text plus a "resource usage graph" in the taskbar window; the network-speed graph was added in an earlier release [S12] | **Child window of `Shell_TrayWnd`**: `SetParent(hwnd, Shell_TrayWnd)` and positioned next to `TrayNotifyWnd`/`Start`. It detects Win11 by finding the `Windows.UI.Composition.DesktopWindowContentBridge` child. Classic mode uses `ReBarWindow32`/`MSTaskSwWClass` [S11]. Also has a tray icon (`Shell_NotifyIcon`) [S11] |
| **taskbar-monitor** (leandrosa81) | Partly. v0.5.4; commit 2026-03-15 [S13] | GPL-3.0 [S13] | CPU, memory (plus more in later versions; **low confidence**) | Yes | **DeskBand** on Win10 (CSDeskBand library). A separate **`TaskbarMonitorWindows11`** exe does `SetParent` into `Shell_TrayWnd` [S13] |
| **PerfMonBar** (XhmikosR) | Effectively dormant. 1.0.8.191; last commit 2025-11-07 [S14] | GPL-3.0 [S14] | Any PDH counter via config.xml (CPU, RAM, net by default) | Text only | **DeskBand** (`IDeskBand` in `src/PerfBar.h`), so it cannot work on the Win11 taskbar [S14] |
| **NetSpeedMonitor** (Florian Gilles) | Discontinued; the developer's homepage no longer offers it (secondary sources) [S15] | Freeware | Net up/down | Text | DeskBand toolbar; reported not to work on Win11 [S15]. **Low confidence** (no primary source still online) |
| **XMeters** (entropy6) | Unclear; the site shows © 2017 only [S16] | Freeware for personal use [S16] | "Taskbar System Stats" (CPU/mem/disk/net per product description) | Yes on Win10 | Not documented on the site. **Low confidence**; widely described as a taskbar toolbar (DeskBand) |
| **Rainmeter** | Yes. 4.5.28 stable (2026-10-12); 5.0 RC2 (2026-11-16); commit 2026-11-16 [S17][S18] | GPL-2.0 [S17] | Anything, via desktop "skins" | Desktop skins, plus **one tray icon** that can draw a **16×16 histogram** of a `[TrayMeasure]` | `TrayIcon.cpp` draws a GDI+ 16×16 bitmap and calls `Shell_NotifyIcon(NIM_MODIFY)` [S18] |
| **Process Explorer** (Sysinternals) | Yes. Page published 2026-11-05; runs on Win11+ [S19] | Sysinternals freeware (proprietary) | Full process tree | **Tray icon CPU history graph** by default; optional I/O, GPU and memory tray icons (Options → Tray Icons) [S20] | Animated tray icon (`Shell_NotifyIcon`). **Medium confidence**: the Microsoft page does not describe tray icons; the help file and secondary sources do [S20] |
| **HWiNFO** | Active (commercial plus free). hwinfo.com returned 403 to my fetcher | Freeware/Pro | Sensors | Shows sensor **values** as tray icons (text, not history graphs) | Tray icons. **Low confidence**: no primary source retrieved |
| **PinStats** (airtaxi, C#/WinUI) | Yes. Commit 2026-11-08 [S21] | MIT [S21] | CPU, GPU, mem, net, storage, battery | Tray icon shows a **number** (every 0.25 s). The Win11 "taskbar widget" shows rings and readouts (no history graph) | Win11 taskbar strip plus a tray icon [S21] |
| **rmeters** (Rust) | Yes. v0.6.1; commit 2026-11-21 [S22] | No LICENSE file found (**treat as all rights reserved**) [S22] | CPU and RAM **scrolling sparkline tray icons** (60 s); disk ring icons with 85%/95% alerts | **Yes: graphs in tray icons**, high-DPI aware, re-adds icons on Explorer restart [S22] | One tray icon per metric, redrawn (`src/tray.rs`), using `windows` 0.58 and `sysinfo` [S22] |
| **StatsOverlay** (ruia, Rust) | Yes. v0.3.1; commit 2026-10-23 [S23] | No LICENSE file found [S23] | RAM split by Windows vs WSL/vmmem, commit charge, CPU, GPU, disk; graph of the last minute; hover panel | **Yes, inside the Win11 taskbar** | "A **layered child window of the taskbar**", plain Win32 plus Direct2D. Falls back to an always-on-top strip. Reads `TaskbarAl` and `TaskbarDa` to avoid the Widgets button [S23] |
| **Resource Monitor / resmon** (Garidev, Rust) | Yes. Commit 2026-08-18 [S24] | MIT [S24] | Up to 6 live tray icons (CPU, RAM, disk, net, FPS). Popup with 60-sample sparklines. **Click a metric for top apps by that metric**, a per-core grid, and an MCP server | Tray icons show values; sparklines live in the popup | Win32 tray plus a popup panel. Samples per-process data only while the panel is open [S24] |
| **PulseTray** (Aseloth99, Tauri 2 + Rust) | Yes. Commit 2026-11-16 [S25] | MIT [S25] | 10 tiles, including **Latency** (gateway, 1.1.1.1, 8.8.8.8: avg/min/max/jitter/loss) and **Top Apps**. Charts up to 1 h, including while the panel was closed. "Who caused that peak?" on hover | Tray icon shows a **percentage**, not a graph. Graphs only in the popup | Uses `IcmpSendEcho` (no admin). **Pings only on demand** (a timed test or while the panel is open) [S25] |
| **winmon-rs** (BernatRubio, Rust) | Commit 2026-10-20 [S26] | GPL-3.0 [S26] | CPU and RAM % | Number in tray icon | 32×32 RGBA, then `CreateIconIndirect`, with icons cached per value [S26] |

**Takeaway:**
- Graphs *in* the Win11 taskbar exist today only through the child-of-`Shell_TrayWnd` hack (TrafficMonitor, StatsOverlay, taskbar-monitor Win11).
- Graphs *in tray icons* exist in Process Explorer, Rainmeter and rmeters.
- No tool I found shows an **always-visible latency graph** plus throughput plus attribution together.
- PulseTray comes closest on features, but its latency is on-demand and lives only in the popup.

## 5. Windows 11 taskbar: what still works

### DeskBands are gone on Win11

- Microsoft's Windows 11 spec page, under "Feature deprecations and removals" → Taskbar, says: **"Apps can no longer customize areas of the Taskbar."** [S27]
- The `IDeskBand` reference has said since the Windows 7 era: "You should use thumbnail toolbars in new development in place of desk bands, **which are not supported as of Windows 7**." [S28]
- Desk bands were the COM objects the user enabled through taskbar right-click → **Toolbars** [S29]. The Win11 taskbar no longer has that menu (**medium confidence**; implied by [S27], not stated literally there).
- Practical confirmation in code:
  - PerfMonBar (DeskBand) has no Win11 path [S14].
  - taskbar-monitor ships a *separate* Win11 exe that abandons the DeskBand for `SetParent` [S13].
  - TrafficMonitor uses separate `ClassicalTaskbarDlg` and `Win11TaskbarDlg` paths [S11].

### Options on Win11 today

| Approach | How | Pros | Cons |
|---|---|---|---|
| **A. Animated tray icon(s)** | `Shell_NotifyIcon(NIM_ADD/NIM_MODIFY)` with a freshly drawn `HICON` each tick, one icon per metric. Identify icons by `guidItem` (NIF_GUID) [S30] | Fully supported, documented API. Survives taskbar redesigns. No positioning hacks. Click, hover and context menu come free. rmeters, Rainmeter and Process Explorer show it works. | Tiny canvas: 16×16 logical px. Supply 32×32 at high DPI or Windows scales the 16×16 icon up badly [S30]. Win11 may hide new icons in the overflow until the user pins them. Tooltip limited to 128 chars [S30]. Practical cap of ~1 metric per icon. `guidItem` is tied to the binary path, so moving the exe breaks it [S30]. |
| **B. Window parented into / overlaid on the taskbar** | `FindWindow("Shell_TrayWnd")`, then `SetParent` (or a topmost layered strip), placed beside `TrayNotifyWnd` or `Start`. Must handle centered vs left alignment (`TaskbarAl`), the Widgets button (`TaskbarDa`), DPI, multi-monitor and Explorer restarts [S11][S23] | Wide canvas, so it can show several labelled sparklines (closest to iStat Menus). Moves and auto-hides with the taskbar when it is a child window [S23]. | Undocumented and unsupported; Microsoft says apps "can no longer customize" the taskbar [S27]. Fragile across Win11 builds: TrafficMonitor's changelog has a long tail of Win11 22H2/22621 position fixes, Widgets overlap and touch-device fixes [S12]. Overlaps icons when the taskbar is full, which the author calls unfixable [S31]. Can break with StartAllBack or ExplorerPatcher [S12]. |
| **C. Windows Widgets Board** | Packaged (MSIX) Win32 app implementing `IWidgetProvider` with Adaptive Cards via the Windows App SDK, or a PWA [S32][S33] | Official and supported. Good for a richer "diagnosis card". | Not always visible: only shows when the user opens the board (Win+W or the taskbar Widgets icon) [S33]. Requires packaging. Adaptive Cards are poor at live 1 Hz sparklines (**medium confidence**). Web widgets are EEA-only [S33]. |
| **D. Xbox Game Bar widget** | UWP XAML app as a Game Bar extension; can be **pinned** on screen and click-through [S34][S35] | Overlay works over games. Pinned mode stays visible. | UWP only. Lives in the Game Bar overlay, not the taskbar. Gaming-oriented, so a poor fit for "why is my PC slow". |
| **E. Small always-on-top desktop window** | Normal topmost layered window (TrafficMonitor main window, PulseTray "Mini Gauge", StatsOverlay fallback) [S10][S23][S25] | Simple and robust. | Covers content and is not "in the taskbar". |

**Recommendation for later ADRs** (my inference, not a source claim):
- Make the **tray icon (A)** the baseline: per-metric 16/32 px sparklines (CPU, mem, net, ping) with a rich click-out panel.
- Offer **B** as an opt-in "taskbar strip" behind a feature flag, with a tray-only fallback, as StatsOverlay does.

## 6. Gaps / opportunity (vs. the user's goal)

1. **Combined "is it me, my PC, my LAN, or the internet?" diagnosis.** No tool reviewed correlates throughput, local load and latency to several hops into a verdict. PulseTray has the raw pieces (gateway, 1.1.1.1 and 8.8.8.8 latency with loss and jitter) but only on demand [S25]. Opportunity:
   - Ping **gateway + 1.1.1.1 (+ a DNS lookup)** continuously and cheaply.
   - Show a ping sparkline in the tray.
   - Classify the cause: gateway bad means Wi-Fi/LAN; gateway good but 1.1.1.1 bad means ISP/WAN; both good but DNS slow means DNS; latency high while upload is saturated means bufferbloat or self-inflicted traffic.
2. **Ambient latency graph.** gping-style latency graphs exist only in terminals [S7] or behind a click [S25]. A 16/32 px tray sparkline for ping, with loss drawn as red ticks, appears unclaimed.
3. **"Why is it slow?" attribution at the moment it happened.**
   - iStat Menus shows top apps per resource [S1]. resmon shows top apps on click [S24]. PulseTray answers "who caused that peak?" on hover [S25].
   - Most Windows taskbar tools show only totals (TrafficMonitor, PinStats, PerfMonBar).
   - Opportunity: keep a short rolling ring buffer of top-N processes per resource (CPU, RAM/commit, disk, **network per process**) so a spike can be explained after the fact.
4. **Memory pressure, not just "% used".** StatsOverlay points out that commit charge, not RAM, is what makes allocations fail [S23]. iStat Menus shows memory pressure [S1]. Most Windows tools show only RAM %.
5. **Low-overhead, Explorer-restart-safe, DPI-correct tray graphs** are table stakes (rmeters [S22]). The Win11 taskbar-embedding path stays a maintenance burden [S12][S31].
6. **History ranges.** iStat Menus offers 10 min to 28 days [S2]. Windows tray tools keep 30–60 s (rmeters 60 s, PinStats 30 s) or up to 1 h (PulseTray) [S21][S22][S25]. A cheap on-disk history ("it was slow at 14:05, what happened?") would set the app apart.
7. **Throughput scaling.** Use MenuMeters' cube-root/log scale [S6] so small and large traffic both show on a 16 px graph.


## 7. Helpful network monitoring tools (reference list)

| Tool | What it's good for | Platform / privileges | License |
|---|---|---|---|
| **gping** | Live latency graph for several hosts; `--tcp` connect mode; `--cmd` timing | Win/mac/Linux terminal; no admin on Windows (ICMP via `winping`) | MIT [S7][S8] |
| **mtr** | Continuous traceroute: loss and latency per hop | Linux/mac; Windows only via WSL or Cygwin | GPL-2.0 [S36] |
| **WinMTR** | Native Windows GUI mtr | Windows, no admin (`IcmpSendEcho2` + TTL); unmaintained since 2014 | GPL-2.0 [S37] |
| **Trippy** | mtr plus charts in Rust; ICMP/UDP/TCP, per-hop history | Win/mac/Linux terminal; **admin required on Windows** | Apache-2.0 [S38][S39] |
| **PulseTray** | Tray app: gateway/1.1.1.1/8.8.8.8 latency, jitter and loss on demand | Windows (Tauri) | MIT [S25] |

---

## Sources

- [S1] iStat Menus product page: https://bjango.com/mac/istatmenus/
- [S2] iStat Menus version history: https://bjango.com/mac/istatmenus/versionhistory/
- [S3] iStat Menus 7 help, Combined: https://bjango.com/help/istatmenus7/combined/
- [S4] iStat Menus 7 help, History graphs: https://bjango.com/help/istatmenus7/historygraphs/
- [S5] MenuMeters releases: https://github.com/yujitach/MenuMeters/releases
- [S6] MenuMeters repo (README, LICENSE, `Common/MenuMeter{CPU,Mem,Net}.h`, `MenuMetersMenuExtraBase.m`, `releases.html`, `InfoPlistPreprocessor.h`): https://github.com/yujitach/MenuMeters
- [S7] gping repo (readme, `gping/Cargo.toml`, `gping/src/main.rs`): https://github.com/orf/gping
- [S8] gping `pinger` crate (`pinger/Cargo.toml`, `pinger/src/windows.rs`, `pinger/src/tcp.rs`): https://github.com/orf/gping/tree/master/pinger
- [S9] IcmpSendEcho (Win32 ICMP API): https://learn.microsoft.com/en-us/windows/win32/api/icmpapi/nf-icmpapi-icmpsendecho (cited for the millisecond RTT field. I did not fetch this page in this session; **medium confidence**)
- [S10] TrafficMonitor README: https://github.com/zhongyang219/TrafficMonitor
- [S11] TrafficMonitor source (`TrafficMonitor/Win11TaskbarDlg.cpp`, `TaskBarDlg.cpp`, `TrafficMonitor.cpp`, `ClassicalTaskbarDlg.cpp`, `TrafficMonitorDlg.cpp`, `LICENSE`, `version_utf8.info`): https://github.com/zhongyang219/TrafficMonitor/tree/master/TrafficMonitor
- [S12] TrafficMonitor update log (EN): https://github.com/zhongyang219/TrafficMonitor/blob/master/UpdateLog/update_log_en-us.md
- [S13] taskbar-monitor (README, LICENSE, `TaskbarMonitor/CSDeskBand.cs`, `TaskbarMonitor/TaskbarManager.cs`, `TaskbarMonitorWindows11/Program.cs`): https://github.com/leandrosa81/taskbar-monitor
- [S14] PerfMonBar repo (LICENSE, `src/PerfBar.h`): https://github.com/XhmikosR/perfmonbar ; site: https://xhmikosr.github.io/perfmonbar
- [S15] NetSpeedMonitor status (secondary): https://www.giga.de/tipp/windows-11-netspeedmonitor-alternative-nutzen-so-gehts/ , https://www.deskmodder.de/blog/?p=207265
- [S16] XMeters: https://entropy6.com/xmeters/ and https://entropy6.com/xmeters/help/
- [S17] Rainmeter site: https://www.rainmeter.net/
- [S18] Rainmeter source (`Library/TrayIcon.cpp`, `Library/TrayIcon.h`): https://github.com/rainmeter/rainmeter
- [S19] Process Explorer (Microsoft Learn): https://learn.microsoft.com/en-us/sysinternals/downloads/process-explorer
- [S20] Process Explorer tray icons (help file mirror and secondary sources): https://documentation.help/procexp/Running_in_the_Tray.htm , https://www.computerworld.com/article/1346034/installing-and-tweaking-process-explorer-part-2.html
- [S21] PinStats: https://github.com/airtaxi/PinStats
- [S22] rmeters: https://github.com/andrewchuev/rmeters
- [S23] StatsOverlay (README, `src/taskbar.rs`): https://github.com/ruia/stats-overlay
- [S24] Resource Monitor (resmon): https://github.com/Garidev/resourcemonitor
- [S25] PulseTray: https://github.com/Aseloth99/pulsetray
- [S26] winmon-rs: https://github.com/BernatRubio/winmon-rs
- [S27] Windows 11 specifications, feature deprecations and removals: https://www.microsoft.com/en-us/windows/windows-11-specifications
- [S28] IDeskBand interface: https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ideskband
- [S29] Band objects / Desk Bands: https://learn.microsoft.com/en-us/windows/win32/shell/band-objects
- [S30] NOTIFYICONDATAW: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw
- [S31] TrafficMonitor Help (Win11 overlap FAQ): https://github.com/zhongyang219/TrafficMonitor/blob/master/Help_en-us.md
- [S32] Widget providers: https://learn.microsoft.com/en-us/windows/apps/develop/widgets/widget-providers
- [S33] Windows Widgets design overview: https://learn.microsoft.com/en-us/windows/apps/design/widgets/
- [S34] Game Bar widget overview: https://learn.microsoft.com/en-us/gaming/game-bar/overview
- [S35] XboxGameBarWidget API (PinningSupported/Pinned) and click-through: https://learn.microsoft.com/en-us/xbox/game-bar/api/xgb-widget , https://learn.microsoft.com/en-us/xbox/game-bar/guide/click-through

- [S36] mtr (README "Using mtr on Windows", COPYING, packet/probe_cygwin.c): https://github.com/traviscross/mtr
- [S37] WinMTR, White-Tiger fork (src/WinMTRNet.cpp, LICENSE): https://github.com/White-Tiger/WinMTR
- [S38] Trippy (README, LICENSE, crates/trippy-core/src/net/platform/windows.rs): https://github.com/fujiapple852/trippy
- [S39] Trippy privileges guide: https://trippy.rs/guides/privileges/

### Known gaps in this research

- **Not verified from primary sources:** the full list of iStat Menus menu bar styles (help pages returned 403), HWiNFO tray details (403), XMeters' technique and Win11 status, NetSpeedMonitor (no primary source still online), and the Game Bar pages (from search results; not opened).
- **Not fetched:** IcmpSendEcho's ms-resolution RTT [S9]. It is consistent with gping's `from_millis` code.
