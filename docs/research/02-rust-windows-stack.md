# 02 — Rust stack for a Windows 11 tray system monitor

Researched 2026-11-21. Target: a Rust app for Windows 11, including managed machines (Entra-joined, Intune/Defender policy). It lives in the notification area and draws live mini graphs for CPU, memory, network throughput, and ping latency to 1.1.1.1. Clicking it opens a detail panel with per-core CPU, top processes, and history graphs. Budget: under 1% CPU and a small RAM footprint.

Versions come from the crates.io API (`https://crates.io/api/v1/crates/<name>`) on 2026-11-21. Numbered citations such as [S1] point to the Sources list at the end.

Confidence markers:
- **(verified)**: read in a primary source (docs, API reference, or source code).
- **(inferred)**: my reasoning from the sources, not stated in them.
- **(low confidence)**: weak or community-only evidence. Prototype these before relying on them.

---

## TL;DR recommendations

| Area | Recommendation | Main alternative |
|---|---|---|
| Tray icon | `tray-icon` 0.26.1. Redraw an RGBA bitmap each second and call `set_icon`. Use one icon per metric (2–4 icons). | Raw `Shell_NotifyIconW` through `windows` 0.62.2, for full control of `guidItem` and `NIM_SETVERSION` |
| In-taskbar drawing | Don't do it for v1. Reconsider later as an opt-in "taskbar mode". | A TrafficMonitor-style window: `SetParent` into `Shell_TrayWnd` plus `UpdateLayeredWindow` |
| Popup UI | `eframe`/`egui` 0.37 with the `glow` renderer, plus `egui_plot` 0.38. Use a borderless, always-on-top viewport positioned from the tray `rect`. | Slint 1.19 (smaller and retained-mode, but no built-in chart widget). Raw Direct2D if size is critical. |
| Metrics | Call Win32 directly: `GetSystemTimes`, PDH per-core counters, `GetIfTable2`, `GlobalMemoryStatusEx`, and `NtQuerySystemInformation(SystemProcessInformation)` for the process list. Treat `sysinfo` 0.40.4 as the fast-start fallback. | `sysinfo` for everything |
| Per-process network | Not available without admin. Show per-process connection counts instead, or gate it behind an elevated/ETW mode. | ETW kernel network, which needs Administrators or Performance Log Users |
| Ping | `IcmpSendEcho2` through `windows`, on a dedicated thread with a 1 s timeout. No admin needed. | The `winping` crate (unmaintained since 2020) or `pinger` 2.2.0 (wraps `winping` on Windows) |
| Packaging | Signed exe plus a per-user MSI from `cargo-dist` 0.34 (WiX v3). Start at login with an HKCU `Run` value. Sign with Azure Artifact Signing. | MSIX with `windows.startupTask`. Or hand the signed MSI to IT for Intune deployment. |

---

## 1. Tray and notification-area icons

### What the platform allows (verified)
- **Updating the icon:** `NIM_MODIFY` modifies an existing icon. `NIM_SETVERSION` "must be called every time a notification area icon is added (NIM_ADD). It does not need to be called with NIM_MODIFY" [S1]. The docs set no update-rate limit. Updating once a second with a freshly drawn `HICON` is an ordinary `NIM_MODIFY` call.
- **Several icons from one process:** "You can have multiple icons associated with a single hWnd by assigning each a different uID. If guidItem is specified, uID is ignored" [S2].
- **Icon identity:** Microsoft recommends `guidItem` on Windows 7 and later. The GUID registration includes the binary's path. Settings survive moving the exe only if both copies are "Authenticode-signed by the same company" [S2]. This matters for the user's "always show" choice. It is a reason to sign the exe and use a fixed install path.
- **Popup anchoring:** With `NOTIFYICON_VERSION_4`, `wParam` carries the X and Y anchor coordinates for `NIN_SELECT`, `NIN_POPUPOPEN`, and mouse messages [S2]. `Shell_NotifyIconGetRect` returns the icon's rectangle [S3]. `CalculatePopupWindowPosition` positions a popup relative to an anchor point [S4].
- **DPI and icon size:** If you supply only a 16×16 icon, "it is scaled to a larger size in a system set to a high dpi value," which can look bad. Microsoft recommends `LoadIconMetric` [S2]. The UX guide says to provide 16×16, 20×20, and 24×24 icons [S5]. For icons drawn at runtime, size the bitmap with `GetSystemMetricsForDpi(SM_CXSMICON, dpi)` [S6] and redraw on `WM_DPICHANGED` (inferred).
- **UX guidance:** The guide says "Provide only one notification area icon per component" and "Don't change the icon rapidly" [S5]. It is a Windows 7-era guide, but those two rules are still the reason to keep it to a small number of icons (inferred).
- **Hidden by default on Windows 11 (low confidence, community sources only):** Windows 11 puts new tray icons in the overflow menu by default. Users must enable each one under Settings > Personalization > Taskbar > Other system tray icons. Reports say the setting can reset when an app updates [S7]. Plan first-run instructions for this.

### Crates

| Crate | Version / updated | Notes |
|---|---|---|
| `tray-icon` (tauri-apps) | 0.26.1 / 2026-10-07 [S8] | Needs an event loop running on the thread on Windows. Create the `TrayIcon` only after the loop is running [S9]. Has `set_icon(Option<Icon>)`, `rect()`, and `set_tooltip` [S10]. `TrayIconEvent::Click` carries `position` and `rect` [S11]. The type is `!Send`/`!Sync` [S10]. Each icon has its own `TrayIconId` [S9], which suggests several icons are supported (inferred). Works with `winit`/`tao` through `set_event_handler` [S9]. **Pick this one.** |
| `windows` (microsoft/windows-rs) | 0.62.2 / 2025-10-06 (latest stable) [S12] | Gives direct `Shell_NotifyIconW`, `CreateIconIndirect`, and so on. Most control, most boilerplate. Also needed for metrics and ping, so it is in the dependency tree anyway. |
| `trayicon` (ciantic) | 0.4.1 / 2026-06-02 [S13] | Smaller and Windows-focused. Maintained but less widely used. |
| `native-windows-gui` (nwg) | 1.0.13 / 2022-09-05 [S14] | README: "the backlog is empty, and it will most likely stay that way", and future work "will be done in other libraries" [S15]. **Avoid for new work.** |

**Plan:** Use 2–4 `tray-icon` instances (CPU, MEM, NET, PING), or one wider combined icon. Each tick, render 16–32 px sparklines into an RGBA buffer with `tiny-skia` 0.12.0 [S16] or plain pixel writes, then call `Icon::from_rgba` and `set_icon`.

Prototype the Explorer-side cost of 1 Hz `NIM_MODIFY` across four icons. I found no published measurement **(low confidence that it is negligible, though widely done)**.

---

## 2. Drawing in the Windows 11 taskbar (the TrafficMonitor approach)

### How TrafficMonitor does it (verified from source, master branch)
1. It finds the taskbar with `FindWindow("Shell_TrayWnd")`. It also finds secondary-monitor taskbars, which `TaskbarHelper.cpp` identifies by the class `Shell_SecondaryTrayWnd` [S17][S18].
2. It sets `WS_EX_TOOLWINDOW` on its dialog, then calls `::SetParent(this->m_hWnd, GetParentHwnd())`. The source comment reads "把程序窗口设置成任务栏的子窗口" ("make the program window a child of the taskbar") [S17].
3. **Windows 11 detection:** the taskbar is treated as Windows 11-style if `Shell_TrayWnd` has a child of class `Windows.UI.Composition.DesktopWindowContentBridge` [S19]. It reads `TaskbarAl` from `HKCU\...\Explorer\Advanced` to detect centered alignment [S20].
4. **Rendering:** it uses `WS_EX_LAYERED` with Direct2D through `UpdateLayeredWindowIndirect` and falls back to GDI if that fails [S17]. On Windows 11 it avoids a pure-black background "to fix the right-click menu not appearing in dark mode" [S17]. A 1000 ms timer drives repaints [S17].
5. It detects the Windows Web Experience Pack (Widgets) to avoid overlapping it [S17].

### Fragility (verified)
TrafficMonitor's release notes keep chasing Windows 11 layout problems:
- The window sits lower when no icons are pinned. The workaround is to pin one icon.
- A blank gap appears from the "avoid Widgets" fix.
- There were issues with classic-taskbar restorers [S21].

`SetParent` across processes forces a DPI-awareness reset of the child window's process [S22]. Raymond Chen describes cross-process parent/child windows as technically legal but tying both processes' input queues together, so it is easy to get wrong [S23].

### Feasibility from Rust
All of these exist in `windows` 0.62: `FindWindowW`, `FindWindowExW`, `SetParent`, `UpdateLayeredWindow` [S24], and Direct2D/DirectComposition. Porting is mechanical (inferred).

The risks are behavioral: Explorer restarts (you must re-parent on `TaskbarCreated`), taskbar layout changes in Windows feature updates, and corporate EDR possibly flagging a process that injects UI into Explorer **(low confidence; not verified against Defender documentation)**.

**Recommendation:** Ship tray icons first. If in-taskbar graphs are wanted, add them later as an opt-in mode. Isolate it behind a trait so a taskbar change can disable it without breaking the app.

---

## 3. Popup and detail-panel UI

| Option | Version | Idle CPU | Binary / RAM | Borderless anchored popup | Mica / acrylic | Charts |
|---|---|---|---|---|---|---|
| **egui / eframe** | 0.37.0 [S25] | "egui only repaints when there is interaction (e.g. mouse movement) or an animation"; about 1–2 ms per frame [S26] | Switching from wgpu to `glow` "can significantly reduce your binary size" [S27]. eframe "has a lot of dependencies" [S26]. | `ViewportBuilder::with_decorations(false)`, `with_always_on_top`, `with_position`, `with_taskbar(false)` (Windows), `with_transparent` [S28] | Via `window-vibrancy` 0.8.1 (`apply_mica` on Windows 11; acrylic has drag/resize lag) [S29] or `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE)` (Windows 11 build 22621+) [S30] | `egui_plot` 0.38.0 [S31] |
| **Slint** | 1.19.0 [S32] | Reactive property system [S33] | Runtime "fits in less than 300KiB RAM" [S33]. Software and FemtoVG renderers are light; Skia has a "heavy disk-footprint" [S34]. | winit backend (inferred: winit window attributes) | Same DWM approach (inferred) | No built-in chart widget; draw `Path` or images yourself (inferred) |
| **iced** | 0.14.0 (2025-12-07) [S35] | Not documented | wgpu, or tiny-skia software fallback [S36] | Multi-window not documented on the README | Same DWM approach (inferred) | No chart widget listed [S36]; canvas only |
| **Tauri 2 + WebView2** | 2.12.7 [S37] | WebView2 runs a multi-process model (browser, renderer, and GPU processes) [S38] | Small exe [S39], but WebView2 processes dominate RAM (inferred). Conflicts with "small RAM". | `tauri-plugin-positioner` 2.4.0 has tray-relative positions [S40][S41] | `window-vibrancy` comes from the Tauri org [S29] | Any JS chart library |
| **Raw Direct2D (windows-rs)** | `windows` 0.62.2 | Lowest; repaint only on a timer | Smallest | Full control (`CalculatePopupWindowPosition` [S4]) | DWM attribute [S30] | Hand-written |

**Licensing:** Slint is available under GPL, a "royalty-free permissive license for desktop, mobile, and web," or a paid license [S33]. Check that the royalty-free terms fit a corporate internal tool.

**Recommendation:** Use `eframe` with the `glow` renderer and `egui_plot`. Keep the popup viewport hidden or closed when it isn't shown. Close it on focus loss, as the UX guide recommends ("Dismiss the popup window if the user clicks anywhere outside the window" [S5]).

Its reactive repaint model means zero redraws while the popup is closed [S26]. While it is open, use `request_repaint_after(1s)` (inferred API usage).

Keep the tray rendering independent of egui: one tiny thread renders the icons, and the GPU context is created only when the popup opens (inferred). If the binary size or GPU context cost of a prototype is too high, fall back to Slint with the software renderer.

---

## 4. Metrics

### What `sysinfo` 0.40.4 does on Windows (verified from source, main branch)
- **CPU (total and per core):** PDH queries on `\Processor(_Total)\% Idle Time` and `\Processor(N)\% Idle Time` [S42][S43]. Frequency comes from `CallNtPowerInformation` [S42].
- **Process list:** `CreateToolhelp32Snapshot`/`Process32NextW` [S43]. Per-process CPU uses `GetProcessTimes` and `GetSystemTimes` for each process. Memory uses `GetProcessMemoryInfo`. I/O uses `GetProcessIoCounters`. It opens a handle to every process, with `PROCESS_QUERY_LIMITED_INFORMATION` as the fallback [S44]. That is one `OpenProcess` per process per refresh (inferred cost driver).
- **Network:** `GetIfTable2` [S45]. There is no per-process network data.
- **Docs guidance:** "use `refresh_specifics(...)` methods with only what you need" and keep one `System` instance [S46].

### Direct-API plan (all run without admin, verified)

| Metric | API | Notes |
|---|---|---|
| Total CPU | `GetSystemTimes` [S47] | One call per tick. Take deltas of idle, kernel, and user time. |
| Per-core CPU | PDH `\Processor(*)\...` [S48] (what sysinfo does) or `NtQuerySystemInformation(SystemProcessorPerformanceInformation)` [S49] | The NtQuery route is documented but "may be altered or unavailable"; load it dynamically [S49]. |
| Memory | `GlobalMemoryStatusEx` (inferred, standard) | |
| Process list (CPU and memory) | `NtQuerySystemInformation(SystemProcessInformation)` [S49] | One call returns every process with working set and memory counters. No per-process handles, so it should be much cheaper than sysinfo's approach (inferred). Only compute it while the popup is open. |
| Network throughput | `GetIfTable2` → `MIB_IF_ROW2` [S50] | Free the table with `FreeMibTable` [S50]. Filter out loopback and virtual adapters; take deltas of the 64-bit octet counters. |

### Per-process network bytes (admin is the blocker, verified)
- **`GetPerTcpConnectionEStats`:** extended statistics must first be enabled with `SetPerTcpConnectionEStats`, which "can only be called by a user logged on as a member of the Administrators group", and under UAC needs `requireAdministrator` [S51]. It covers TCP only.
- **ETW (Microsoft-Windows-Kernel-Network / kernel logger):** "Only users with administrative privileges, users in the Performance Log Users group, and services running as LocalSystem, LocalService, NetworkService can control event tracing sessions." Microsoft also warns that cross-process sessions are a "limited system resource" [S52]. Rust crates exist: `ferrisetw` 1.2.0, last updated 2024-06 [S53].
- **Without admin:** `GetExtendedTcpTable` gives per-connection owning PIDs [S54]. That supports "top processes by connection count" but not bytes (inferred).

**Recommendation:** Start with `sysinfo` behind a `MetricsSource` trait to get a working version quickly. Then replace the hot path with direct calls: `GetSystemTimes`, `GetIfTable2`, `GlobalMemoryStatusEx`, and PDH per-core. Refresh the process table only while the popup is visible.

Mark per-process network as "requires elevated helper" and leave it out of v1. A corporate user is unlikely to be in Administrators or Performance Log Users (inferred).

---

## 5. Ping without admin

| Option | Mechanism | Admin? |
|---|---|---|
| `IcmpSendEcho2` / `IcmpSendEcho2Ex` (windows-rs, `Win32_NetworkManagement_IpHelper`) | Iphlpapi ICMP helper. Runs synchronously, or asynchronously with an Event or APC [S55][S56]. | No elevation mentioned in the docs [S55]. The `winping` README says it "Does not require elevated permissions" [S57]. |
| Raw `SOCK_RAW` ICMP (`surge-ping` 0.9.1) | surge-ping tries DGRAM sockets and falls back to RAW; its unprivileged mode is documented for Linux only [S58] | **Yes on Windows:** "only members of the Administrators group can create sockets of type SOCK_RAW" [S59] |
| `pinger` 2.2.0 (gping) | On Windows it does **not** shell out to `ping.exe`. It uses `winping` (`use winping::{Buffer, Pinger as WinPinger}`; Cargo dependency `winping = "0.10.1"`). Elsewhere it spawns the system `ping` [S60][S61]. | No |
| `winping` 0.10.1 | IcmpSendEcho wrapper, last release 2020-11 [S62] | No |
| `ping-rs` 0.1.2 | Last release 2023-01. Depends on `windows` 0.43 with the IpHelper feature, consistent with IcmpSendEcho [S63] **(inferred)** | No (inferred) |

**Recommendation:** Call `IcmpSendEcho2` directly through `windows` 0.62. It is about 50 lines of code and avoids the stale `windows` 0.43 and `winping` dependencies.

Use a dedicated thread: `IcmpCreateFile` once, then call `IcmpSendEcho2` synchronously with `Timeout = 1000` ms each second and read `RoundTripTime` from `ICMP_ECHO_REPLY` [S55]. Treat `IP_REQ_TIMED_OUT` as loss.

Caveat: corporate firewalls or VPNs may block ICMP to 1.1.1.1. Fall back to TCP-connect latency to 1.1.1.1:443 (inferred).

---

## 6. Packaging and distribution on a corporate PC

- **Signing and SmartScreen (verified):**
  - "EV certificates no longer bypass SmartScreen". Signed apps still show "unrecognized" until reputation builds. "Enterprise policy can prevent continuation entirely" for unsigned apps [S64].
  - On Windows 11, Smart App Control "will block execution of unsigned files unless the file has a positive reputation" [S64].
  - Microsoft recommends **Azure Artifact Signing** (formerly Trusted Signing) for apps outside the Store. It starts at $9.99/month [S64]. Tiers: Basic is 5,000 signatures/month and Premium is 100,000 [S65]. It needs a paid Azure subscription [S66].
  - Public Trust is available to organizations in the US, Canada, EU, UK, Australia, NZ, Japan, South Korea, Singapore, Switzerland, Norway, and Israel. Individual developers must be in the US or Canada. Private Trust has no geographic limits [S67]. Private Trust or CI-policy profiles fit an internal tool that IT whitelists through App Control [S67].
- **`cargo-dist` 0.34.0 [S68]:**
  - Builds MSIs with WiX v3, with "WiX v4 isn't yet supported", using `cargo-wix` as a library. Persists `upgrade-guid` and `path-guid` in `Cargo.toml` [S69].
  - Default install location is `%ProgramFiles%\{app}`, which implies per-machine install and elevation (inferred) [S69].
  - Signs EXEs and MSIs with SSL.com eSigner or Azure Artifact Signing, x86_64 only [S70].
  - `cargo-wix` 0.3.9, last updated 2025-03 [S71].
- **Starting at login:**
  - The HKCU `Run` key needs no admin. Windows "may choose to delay" `Run` programs. The command line is limited to 260 chars [S72]. **Recommended.**
  - MSIX `windows.startupTask` can start enabled on first launch (`Enabled`). Enabling it on install without launching requires a restricted capability [S73][S74].
  - Task Scheduler (`ONLOGON` trigger) is the heavier alternative [S75].
- **MSIX vs plain exe/MSI:**
  - MSIX gives clean install and uninstall and identity [S76].
  - Interaction with the `NOTIFYICONDATA` GUID-path rule is a point to test, because MSIX paths change per version **(low confidence; untested)** [S2].
  - For an Entra/Intune machine, the simplest supported path is a signed MSI that IT deploys as an Intune Win32 or LOB app (inferred).
  - A per-user `.zip` with a signed exe plus a `Run` key also works, if App Control policy allows user-writable paths **(low confidence; depends on tenant policy)**.

---

## Open questions to settle with a prototype
1. Measure Explorer and own-process CPU with four tray icons updated at 1 Hz.
2. Measure eframe+glow idle RAM with the popup closed vs open, compared with Slint software rendering.
3. Check the cost of `NtQuerySystemInformation(SystemProcessInformation)` vs sysinfo `refresh_processes` on a machine running about 300 processes.
4. Find out whether corporate policy blocks ICMP to 1.1.1.1, and whether App Control allows an exe in `%LOCALAPPDATA%`.

---

## Sources
- [S1] Shell_NotifyIconW — https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw
- [S2] NOTIFYICONDATAW — https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw
- [S3] Shell_NotifyIconGetRect — https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyicongetrect
- [S4] CalculatePopupWindowPosition — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-calculatepopupwindowposition
- [S5] Notification Area UX guidelines — https://learn.microsoft.com/en-us/windows/win32/uxguide/winenv-notification
- [S6] GetSystemMetricsForDpi — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetricsfordpi ; LoadIconMetric — https://learn.microsoft.com/en-us/windows/win32/api/commctrl/nf-commctrl-loadiconmetric
- [S7] Microsoft Q&A, tray icons re-hidden on Windows 11 (community) — https://learn.microsoft.com/en-us/answers/questions/5600219/other-system-tray-icons-constantly-having-to-activ ; https://www.ghacks.net/2021/11/28/how-to-display-all-system-tray-icons-in-windows-11/
- [S8] tray-icon on crates.io — https://crates.io/crates/tray-icon
- [S9] tray-icon crate docs — https://docs.rs/tray-icon/latest/tray_icon/
- [S10] tray_icon::TrayIcon — https://docs.rs/tray-icon/latest/tray_icon/struct.TrayIcon.html
- [S11] tray_icon::TrayIconEvent — https://docs.rs/tray-icon/latest/tray_icon/enum.TrayIconEvent.html
- [S12] windows crate — https://crates.io/crates/windows ; repo https://github.com/microsoft/windows-rs
- [S13] trayicon crate — https://crates.io/crates/trayicon
- [S14] native-windows-gui crate — https://crates.io/crates/native-windows-gui
- [S15] native-windows-gui README — https://github.com/gabdube/native-windows-gui
- [S16] tiny-skia — https://crates.io/crates/tiny-skia
- [S17] TrafficMonitor TaskBarDlg.cpp — https://github.com/zhongyang219/TrafficMonitor/blob/master/TrafficMonitor/TaskBarDlg.cpp
- [S18] TrafficMonitor TaskbarHelper.cpp — https://github.com/zhongyang219/TrafficMonitor/blob/master/TrafficMonitor/TaskbarHelper.cpp
- [S19] TrafficMonitor TrafficMonitor.cpp (Windows 11 taskbar detection, around line 1305) — https://github.com/zhongyang219/TrafficMonitor/blob/master/TrafficMonitor/TrafficMonitor.cpp
- [S20] TrafficMonitor WindowsSettingHelper.cpp — https://github.com/zhongyang219/TrafficMonitor/blob/master/TrafficMonitor/WindowsSettingHelper.cpp
- [S21] TrafficMonitor releases (V1.86, V1.85.1 notes) — https://github.com/zhongyang219/TrafficMonitor/releases
- [S22] SetParent (DPI-awareness remarks) — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent
- [S23] The Old New Thing, cross-process parent/child windows — https://devblogs.microsoft.com/oldnewthing/20130412-00/?p=4683
- [S24] UpdateLayeredWindow — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow
- [S25] eframe crate — https://crates.io/crates/eframe
- [S26] egui README — https://github.com/emilk/egui
- [S27] eframe docs (renderer feature flags) — https://docs.rs/eframe/latest/eframe/
- [S28] egui ViewportBuilder — https://docs.rs/egui/latest/egui/viewport/struct.ViewportBuilder.html
- [S29] window-vibrancy README — https://github.com/tauri-apps/window-vibrancy ; crate https://crates.io/crates/window-vibrancy
- [S30] DWMWINDOWATTRIBUTE — https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute
- [S31] egui_plot — https://crates.io/crates/egui_plot ; https://github.com/emilk/egui_plot
- [S32] slint crate — https://crates.io/crates/slint
- [S33] slint.dev (memory footprint, licensing) — https://slint.dev/
- [S34] Slint backends and renderers — https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/
- [S35] iced crate — https://crates.io/crates/iced
- [S36] iced README — https://github.com/iced-rs/iced
- [S37] tauri crate — https://crates.io/crates/tauri
- [S38] WebView2 process model — https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model
- [S39] Tauri app size — https://v2.tauri.app/concept/size/
- [S40] Tauri positioner plugin — https://v2.tauri.app/plugin/positioner/
- [S41] Tauri system tray — https://v2.tauri.app/learn/system-tray/
- [S42] sysinfo src/windows/cpu.rs — https://github.com/GuillaumeGomez/sysinfo/blob/main/src/windows/cpu.rs
- [S43] sysinfo src/windows/system.rs — https://github.com/GuillaumeGomez/sysinfo/blob/main/src/windows/system.rs
- [S44] sysinfo src/windows/process.rs — https://github.com/GuillaumeGomez/sysinfo/blob/main/src/windows/process.rs
- [S45] sysinfo src/windows/network.rs — https://github.com/GuillaumeGomez/sysinfo/blob/main/src/windows/network.rs
- [S46] sysinfo docs — https://docs.rs/sysinfo/latest/sysinfo/ ; crate https://crates.io/crates/sysinfo
- [S47] GetSystemTimes — https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getsystemtimes
- [S48] Using PDH functions — https://learn.microsoft.com/en-us/windows/win32/perfctrs/using-the-pdh-functions-to-consume-counter-data
- [S49] NtQuerySystemInformation — https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntquerysysteminformation
- [S50] GetIfTable2 — https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getiftable2
- [S51] GetPerTcpConnectionEStats — https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getpertcpconnectionestats
- [S52] StartTraceW (ETW access rights) — https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew
- [S53] ferrisetw — https://crates.io/crates/ferrisetw
- [S54] GetExtendedTcpTable — https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getextendedtcptable
- [S55] IcmpSendEcho2 — https://learn.microsoft.com/en-us/windows/win32/api/icmpapi/nf-icmpapi-icmpsendecho2
- [S56] IcmpSendEcho2Ex — https://learn.microsoft.com/en-us/windows/win32/api/icmpapi/nf-icmpapi-icmpsendecho2ex
- [S57] winping README — https://github.com/TyPR124/winping
- [S58] surge-ping README — https://github.com/kolapapa/surge-ping ; crate https://crates.io/crates/surge-ping
- [S59] TCP/IP raw sockets — https://learn.microsoft.com/en-us/windows/win32/winsock/tcp-ip-raw-sockets-2
- [S60] pinger src/windows.rs — https://github.com/orf/gping/blob/master/pinger/src/windows.rs
- [S61] pinger Cargo.toml — https://github.com/orf/gping/blob/master/pinger/Cargo.toml ; crate https://crates.io/crates/pinger
- [S62] winping crate — https://crates.io/crates/winping
- [S63] ping-rs Cargo.toml — https://github.com/ruxo/ping-rs/blob/master/Cargo.toml ; crate https://crates.io/crates/ping-rs
- [S64] SmartScreen reputation for Windows app developers — https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation
- [S65] Artifact Signing pricing — https://azure.microsoft.com/en-us/pricing/details/artifact-signing/
- [S66] Artifact Signing FAQ — https://learn.microsoft.com/en-us/azure/artifact-signing/faq
- [S67] Artifact Signing quickstart (prerequisites) — https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart ; overview https://learn.microsoft.com/en-us/azure/artifact-signing/overview
- [S68] cargo-dist crate — https://crates.io/crates/cargo-dist
- [S69] cargo-dist MSI installer — https://axodotdev.github.io/cargo-dist/book/installers/msi.html
- [S70] cargo-dist Windows signing — https://axodotdev.github.io/cargo-dist/book/supplychain-security/signing/windows.html
- [S71] cargo-wix — https://crates.io/crates/cargo-wix ; https://volks73.github.io/cargo-wix/cargo_wix/
- [S72] Run and RunOnce registry keys — https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys
- [S73] desktop:StartupTask — https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop-startuptask
- [S74] desktop:Extension — https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop-extension
- [S75] Task Scheduler — https://learn.microsoft.com/en-us/windows/win32/taskschd/task-scheduler-start-page
- [S76] MSIX overview — https://learn.microsoft.com/en-us/windows/msix/overview
