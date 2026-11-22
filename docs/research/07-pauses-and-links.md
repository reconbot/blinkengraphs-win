# 07 — Pauses across sleep and idle, and Probes pinned to a Link

Researched 2026-11-22. Question: how should a non-elevated Rust tray app on Windows 11 (`windows` 0.62.2, `rusqlite` 0.43 bundled) record a Pause when the machine sleeps or rests idle, detect Rounds cut by a suspend, decide when the network is usable again, and send each Probe out a chosen Link (Ethernet, Wi-Fi, VPN)? Also: what SQLite durability costs at one small write per ≤10 s.

Citations such as [S1] point to Sources at the end. Markers as in 02 and 06:
- **(verified)**: read in a primary source.
- **(inferred)**: my reasoning from the sources.
- **(low confidence)**: weak or missing evidence. Prototype before relying on it.

Every API named below exists in `windows` 0.62.2 under the module given (verified by reading the crate source [S36]).

---

## 1. Suspend, resume and idle rest

### 1.1 Which notification

| API | Delivery | Notes |
|---|---|---|
| Top-level window, no registration | `WM_POWERBROADCAST` broadcast | A message-only window (`HWND_MESSAGE`) "does not receive broadcast messages" [S1] (verified). |
| `RegisterSuspendResumeNotification(hwnd, DEVICE_NOTIFY_WINDOW_HANDLE)` (`Win32_System_Power`) | `WM_POWERBROADCAST` to that window | Windows 8+. "Similar to `PowerRegisterSuspendResumeNotification`, but operates in user mode and can take a window handle" [S2] (verified). |
| `RegisterSuspendResumeNotification(.., DEVICE_NOTIFY_CALLBACK)` / `PowerRegisterSuspendResumeNotification` | `DeviceNotifyCallbackRoutine` with `PBT_APMSUSPEND`, `PBT_APMRESUMESUSPEND`, `PBT_APMRESUMEAUTOMATIC` [S2][S3] (verified) | For apps without a window or wanting "a direct callback" [S4] (verified). |

- Registration is what opts a desktop process into suspend/resume messages on Modern Standby machines: the Desktop Activity Moderator (DAM) sends `WM_POWERBROADCAST` only "to those processes subject to suspension that have opted-in to message delivery (via API call or compatibility shim)" [S4] (verified). So register even though the tray window already exists (inferred).
- **Recommendation:** `RegisterSuspendResumeNotification` with the tray's existing HWND, so suspend arrives on the UI thread's message loop like display and lock events (inferred). Use the callback form only if the monitor thread must hear it without the UI thread (inferred).

### 1.2 Timing and ordering

- After `PBT_APMSUSPEND`, "the system allows approximately two seconds for an application to handle this notification", after which it "may interrupt the application" [S5] (verified). Under Modern Standby the broadcast goes "concurrently to all opted in processes" and "the timeout … is global, not per process" [S4] (verified). Budget well under 2 s: write the Pause start and return (inferred).
- A **critical suspension** (e.g. critical battery) sends no notification before sleeping [S6] (verified). The clock check in §1.5 must catch it.
- `PBT_APMRESUMEAUTOMATIC` is sent "every time the system resumes"; `PBT_APMRESUMESUSPEND` follows it only if user input triggered the resume [S7][S8] (verified). Key the end of a sleep on `PBT_APMRESUMEAUTOMATIC`, not on `RESUMESUSPEND` (inferred).
- Since Windows 10 1507, a wake from sleep straight into hibernation delivers no resume message [S8] (verified). Under Modern Standby, the resume message's "relative time of delivery to the system exit from connected standby is not guaranteed" [S4] (verified).
- `WM_POWERBROADCAST` "do[es] not distinguish between different low-power states" [S7] (verified): sleep, hibernate and Modern Standby look the same.

### 1.3 Modern Standby (S0 low-power idle)

- Modern Standby starts **when the display turns off**: power button, lid, Sleep, or idle timeout [S9] (verified).
- Phases run in order: No-CS (waits for power requests; "block … indefinitely on AC power, and for up to 5 minutes on DC power"), Connection, PLM, Maintenance, then **DAM**, which "suspends desktop applications", typically in under a second [S9] (verified). "Windows prevents desktop applications from running during any part of modern standby after completing the DAM phase" [S9] (verified). Interactive-session processes are "subject to suspension": "all their threads suspended (not allowed to run under any circumstances)" [S4] (verified). So the app does not run in standby; there is nothing to measure (inferred).
- Display-off therefore precedes suspension by anywhere from about a second to indefinitely on AC (inferred from the phase table [S9]). Our Pause already begins at display-off (PLAN.md), so Modern Standby needs no extra trigger; `PBT_APMSUSPEND` from DAM only confirms it (inferred).
- Network: the resiliency phase turns off adapters "that do not support modern connected standby (D3)" [S9]; on connected standby "the network is still active" for Store-app background tasks [S10] (verified). Either way our probes are suspended; after resume, Wi-Fi may need to reassociate on disconnected-standby hardware (inferred).
- DAM warns of "inconsistencies in uptime / runtime vs. wall clock time" and in timer behaviour across suspension [S4] (verified).

### 1.4 Resting while idle: display, lock, screen saver

- `RegisterPowerSettingNotification(hwnd, &GUID, DEVICE_NOTIFY_WINDOW_HANDLE)` (`Win32_System_Power`; GUIDs in `Win32_System_SystemServices`) delivers `PBT_POWERSETTINGCHANGE` with a `POWERBROADCAST_SETTING` [S11][S12] (verified).
- **`GUID_SESSION_DISPLAY_STATUS`** is the one to use: "All applications that run in an interactive user-mode session should use this setting"; values Off 0 / On 1 / Dimmed 2 [S13] (verified). `GUID_CONSOLE_DISPLAY_STATE` is the console's display, which that note reserves for kernel-mode callers [S13] (verified). PLAN.md names `GUID_CONSOLE_DISPLAY_STATE`; switch it (inferred). With Remote Desktop the console and the session differ (inferred).
- `GUID_MONITOR_POWER_ON` is legacy: "New applications should use GUID_CONSOLE_DISPLAY_STATE instead" [S13] (verified).
- Treat Dimmed as on (inferred): the user may still be watching.
- `GUID_SESSION_USER_PRESENCE` reports `PowerUserInactive` when "the user activity timeout has elapsed" [S13] (verified); an optional extra rest trigger (inferred).
- Docs do not say whether registration delivers the current value at once (low confidence). Read initial state explicitly; for lock state, assume unlocked at start (inferred).
- Lock: `WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)` (`Win32_System_RemoteDesktop`) sends `WM_WTSSESSION_CHANGE`; it can fail with `RPC_S_INVALID_BINDING` before Remote Desktop services start, signalled by `Global\TermSrvReadyEvent` [S14] (verified). Retry at login autostart (inferred).
- Screen saver: no event found; poll `SystemParametersInfoW(SPI_GETSCREENSAVERRUNNING)`, which reports whether "a screen saver is currently running on the window station of the calling process" [S15] (verified). Polling once per Round costs nothing (inferred).

### 1.5 Detecting a suspend after the fact

| Clock | Counts time asleep? |
|---|---|
| `QueryPerformanceCounter` (Rust `Instant` on Windows [S16]) | Yes: "including the time when the machine was in a sleep state such as standby, hibernate, or connected standby" [S17] (verified) |
| `QueryInterruptTime`, `GetTickCount64` | Yes (biased) [S18][S19] (inferred from "biased" wording and [S20]) |
| `QueryUnbiasedInterruptTime` (`Win32_System_WindowsProgramming`) | No: "does not include time the system spends in sleep or hibernation" [S21] (verified) |
| `GetSystemTimePreciseAsFileTime` | Yes, but jumps with clock changes [S17] (verified) |

- **Rule:** at Round start and end, read `QueryInterruptTime` and `QueryUnbiasedInterruptTime` (both 100 ns units). If `(biased − unbiased)` grew by more than a threshold (say 1 s), the system slept during the Round; mark it cut (inferred). No wall clock, so NTP steps cannot fake a sleep (inferred).
- Whether unbiased time stops during **Modern Standby** DRIPS is not documented; the kernel doc says only that updates stop "when the computer enters a sleep state" [S20] (low confidence). Add a second check: a Round whose `Instant` duration exceeds its timeout by several seconds while the process was runnable was cut (inferred). DAM suspension freezes our threads, so `Instant` still jumps (inferred from [S4][S17]).
- `Instant` arithmetic across a suspend therefore includes the sleep; never report such a span as RTT (inferred).

## 2. When is the network usable after resume?

| Signal | What it says | Earliest? |
|---|---|---|
| `NotifyIpInterfaceChange` [S22] | An IP interface changed; the row has only `Family`, `InterfaceLuid`, `InterfaceIndex`; call `GetIpInterfaceEntry` for state (e.g. `Connected`) [S22][S23] (verified) | Earliest: link up (inferred) |
| `NotifyRouteChange2` [S24] | A route changed; same "incomplete data" rule [S24] (verified) | Default route returns after DHCP (inferred) |
| `NotifyNetworkConnectivityHintChange` / `GetNetworkConnectivityHintForInterface` (Windows 10 2004+) [S25][S26] | Aggregate level: Unknown, None, LocalAccess, InternetAccess, ConstrainedInternetAccess (captive portal), Hidden [S27] (verified) | After NCSI decides (inferred) |
| `INetworkListManagerEvents` | Per-network connectivity (V4/V6 Local/Internet) [S28] (verified) | Same source as the hint (inferred) |

- NCSI decides "Internet" with an HTTP active probe (IPv4 and IPv6 in parallel; either success counts) plus passive inspection of inbound traffic; on Windows 11 it lives in the Network List Manager service [S29] (verified). Corporate proxies can make it fail with HTTP 403 [S29] (verified), so the hint can say LocalAccess on a network that works for our Probes (inferred).
- All MIB callbacks are serialized; never call `CancelMibChangeNotify2` from inside one, or it deadlocks [S22][S24] (verified). Forward events over a channel (inferred).
- **Recommendation:** do not gate on NCSI. Use interface and route notifications only to start the settle clock early, then let our own Probes decide: end the Pause after the settle time **and** a successful Round on the Link, or after a cap (inferred). This keeps NCSI's proxy and captive-portal heuristics out of the measurement.
- **Reconnect time after resume:** no Microsoft figure found. Forum reports range from seconds to minutes and are not evidence [S30] (low confidence). Expect Ethernet in about 1–3 s and Wi-Fi with WPA2-PSK in about 2–10 s; 802.1X and VPN reconnects take longer (low confidence, from general experience, not a source). Make settle time a config default, not a constant.

## 3. Sending each Probe out a specific Link, without admin

### 3.1 Sockets: TCP connect and UDP DNS

- `IP_UNICAST_IF` "gets or sets the outgoing interface for sending IPv4 traffic … This DWORD parameter must be an interface index **in network byte order**" [S31] (verified). In Rust: `(if_index as u32).to_be()`. Reading it back returns host byte order [S31] (verified).
- `IPV6_UNICAST_IF` takes the index "in **host** byte order" [S32] (verified).
- Neither option changes the receiving interface [S31][S32] (verified). Both work on Windows 10 [S31][S32] (verified). The docs do not restrict them by socket type (verified by absence [S31]); set them before `connect` or `sendto` so route selection sees them (inferred).
- For DNS, an own UDP socket with `IP_UNICAST_IF`, sent to that Link's resolver from `FirstDnsServerAddress`, pins both path and server (inferred). Alternatively `DnsQueryEx` takes `DNS_QUERY_REQUEST.InterfaceIndex` ("the interface index over which the query is sent") and `pDnsServerList` [S33] (verified).

### 3.2 ICMP: does a source address force the egress interface?

- `IcmpSendEcho2Ex` takes "the IPv4 source address on which to issue the echo request" and is "useful in cases where a computer has multiple network interfaces" [S34] (verified). `Icmp6SendEcho2` takes a `sockaddr_in6` source [S35] (verified). Neither doc says the source fixes the interface (verified by absence).
- Windows Vista+ uses the **strong host model** by default for IPv4 and IPv6 (`WeakHostSend` = FALSE) [S23][S37] (verified). With strong host send, a specified source makes IP do "a constrained lookup … only those routes with a next-hop interface of the source interface are considered", so "the next-hop interface is always the same as the source interface" [S37] (verified). `MIB_IPINTERFACE_ROW` says the same: "a unicast packet can be sent out a specific interface that does not have the lowest metric for a route by binding the socket to the source address" [S23] (verified).
- So `IcmpSendEcho2Ex` with the Link's address pins egress **if** ICMP.dll's sends follow the normal IP send path (inferred; high likelihood, not documented). If that interface has `WeakHostSend` enabled, the lookup is unconstrained and the packet may leave elsewhere with that source [S37] (verified). Read `WeakHostSend` from `GetIpInterfaceEntry` per Link and flag such Links (inferred).
- If the source interface has no route to the target, expect an error or timeout, not a silent reroute (inferred). Report it as "no route on this Link", not loss (inferred).
- Raw sockets need Administrators: creation fails with `WSAEACCES` [S38] (verified). ICMP without admin means the `IcmpSendEcho2Ex` / `Icmp6SendEcho2` helpers only.
- Hop Traces use `IcmpSendEcho2` today (`crates/probe/src/win.rs`). Switch to `IcmpSendEcho2Ex` with a source address to pin them too (inferred).

## 4. Enumerating Links and their routes

- **`GetAdaptersAddresses`** with `GAA_FLAG_INCLUDE_GATEWAYS` gives, per adapter: `IfType`, `OperStatus`, `Luid`, `IfIndex`/`Ipv6IfIndex`, unicast addresses, DNS servers, `FirstGatewayAddress`, metrics, `TunnelType`, `ConnectionType`, `FriendlyName` [S39][S40] (verified). It "requires a significant amount of network resources and time"; pre-allocate 15 KB [S40] (verified). Call it on change notifications, not every Round (inferred).
- Since Windows 10 the list is ordered by route metric [S40] (verified). Indexes "may change when a network adapter is disabled and then enabled" [S39] (verified): key Links by `Luid` or `AdapterName` and re-resolve the index after each interface event (inferred).
- `GetIfTable2` (already used for throughput) and `GetIpInterfaceTable` give counters and per-family state, not addresses or gateways (inferred). Use `GetAdaptersAddresses` for discovery.
- **Classifying Links** (`IfType` values [S39], verified): Ethernet 6, Wi-Fi `IF_TYPE_IEEE80211` 71, PPP 23 (built-in RAS VPNs), tunnel 131. **Wintun/WireGuard reports `IF_TYPE_PROP_VIRTUAL` 53**: its INF sets `*IfType = 53`, `*MediaType = NdisMediumIP`, `NCF_VIRTUAL` [S41] (verified). Other VPNs (OpenVPN TAP, vendor clients) vary (low confidence). Rule: 6 → Ethernet, 71 → Wi-Fi, {23, 53, 131} or `TunnelType ≠ NONE` → VPN, then skip loopback 24, Teredo/ISATAP/6to4 tunnels and Links with no gateway (inferred). Hyper-V and WSL vEthernet also report 6 (low confidence); exclude by description or by having no default route (inferred).
- **Default-route Link:** `GetBestRoute2(None, 0, None, &dest, ..)` for a public target returns the route and `BestSourceAddress` [S42] (verified). The doc says one of `InterfaceLuid` or `InterfaceIndex` "must be initialized" [S42]; passing zero for both to mean "any" is undocumented (low confidence; test it).
- **Per-Link gateway:** `FirstGatewayAddress`, or `GetIpForwardTable2` rows with prefix length 0 and that `InterfaceLuid` (inferred).
- **Does a Link route a target?** `GetBestRoute2` with `InterfaceLuid` set: "if the InterfaceLuid is specified, then this member is used to determine the interface" [S42] (verified). Whether it then returns an error rather than another interface's route is not stated (low confidence; test it). A source address constrains it too: "If a source IP address is specified in the route lookup using GetBestRoute2 … the route lookup is restricted to the interface of the source IP address" [S23] (verified).
- **Split tunnels:** a full-tunnel Windows VPN client "automatically disables the default routes of LAN interfaces" (`DisableDefaultRoutes`) [S37][S23] (verified). Then the LAN Link has no default route, and a constrained Probe to an Internet target fails on it by design (inferred). Report that Link as "routes local only", not down (inferred). With a split tunnel, an Internet target belongs to the LAN Link and a corporate target to the VPN Link; `GetBestRoute2` per target picks the Link that would actually carry it (inferred). Third-party VPNs that filter with WFP rather than routes may drop pinned LAN Probes regardless (low confidence).

## 5. SQLite: what a write every ≤10 s costs and keeps

- `crates/history` sets only `journal_mode=WAL`. The bundled `libsqlite3-sys` 0.40.0 sets no `SQLITE_DEFAULT_SYNCHRONOUS` [S43], so the default is **FULL** [S44] (verified).
- **FULL in WAL:** "Writers sync the WAL on every transaction commit" [S45]; FULL is "ACID in WAL mode" [S46] (verified).
- **NORMAL in WAL:** commit skips the sync; "the checkpoint is the only operation to issue an I/O barrier or sync" (`FlushFileBuffers` on Windows) [S45]. "WAL mode is safe from corruption with synchronous=NORMAL … A transaction committed in WAL mode with synchronous=NORMAL might roll back following a power loss or system crash" [S46] (verified).
- **Process kill:** "Transactions are durable across application crashes regardless of the synchronous setting or journal mode" [S46] (verified). A committed transaction survives `TerminateProcess` under WAL+NORMAL; only an OS crash or power loss can lose the last commits (inferred from [S46]).
- **Checkpoints:** automatic, PASSIVE, "whenever a COMMIT occurs that causes the WAL file to be 1000 pages or more", about 4 MB [S45][S44] (verified). A reader that never ends starves checkpoints and the WAL "will grow without bound" [S45] (verified); keep reads short (inferred).
- **Cost** (low confidence, not measured): a commit of 1–5 pages appends 4–20 KiB to the WAL. At one per 10 s that is 8,640 commits and roughly 35–175 MB of WAL appends per day, plus checkpoint writes back to the database (inferred). Under FULL each commit adds one `FlushFileBuffers`, on the order of 1 ms on an NVMe SSD (low confidence). CPU and wear are negligible either way (inferred). SQLite itself recommends NORMAL with WAL: "transaction commit is much faster in WAL mode using synchronous=NORMAL than with the default synchronous=FULL" [S44] (verified).
- **Sleep and hibernate:** a commit made before suspend is in the OS cache; under NORMAL it reaches disk only at the next checkpoint or cache flush. A failed resume, a dead battery during sleep, or a crash on resume can lose it (inferred). Running `PRAGMA wal_checkpoint(PASSIVE)` inside the `PBT_APMSUSPEND` handler forces one sync and costs milliseconds of the ~2 s budget (inferred).

## Recommendations for the Windows implementation

1. Register on the tray HWND: `RegisterSuspendResumeNotification(DEVICE_NOTIFY_WINDOW_HANDLE)`, `RegisterPowerSettingNotification(GUID_SESSION_DISPLAY_STATUS)`, `WTSRegisterSessionNotification(NOTIFY_FOR_THIS_SESSION)` with retry. Poll `SPI_GETSCREENSAVERRUNNING` once per Round. Keep that window top-level hidden, not message-only.
2. Pause starts at the first of display off, lock, screen saver or `PBT_APMSUSPEND`. In the suspend handler: write the Pause start, run a PASSIVE checkpoint, return within well under 2 s.
3. On every Round, record the growth of `QueryInterruptTime − QueryUnbiasedInterruptTime`; growth above 1 s marks the Round cut and opens a Pause if none is open. Also treat a Round whose `Instant` span exceeds its timeout by several seconds as cut. This covers critical suspends and missed messages.
4. End a sleep at `PBT_APMRESUMEAUTOMATIC` (or the clock check). The spec ends a Pause a fixed settle time after measuring resumes (`rules/history.md`, Pauses), so set that time from logged `resume → NotifyIpInterfaceChange/NotifyRouteChange2 → first reply` delays per Link type. Ending on a network signal or a first good Round instead would need a spec change (inferred). Do not wait for NCSI.
5. Pin every Probe to its Link: `IP_UNICAST_IF` (index `.to_be()`) or `IPV6_UNICAST_IF` (host order) on TCP and UDP sockets before connect/send; `IcmpSendEcho2Ex` / `Icmp6SendEcho2` with the Link's source address for ICMP and Hop Traces. Flag Links with `WeakHostSend`.
6. Discover Links with `GetAdaptersAddresses(GAA_FLAG_INCLUDE_GATEWAYS)` on change, keyed by LUID. Classify by `IfType` (6, 71, then 23/53/131 or a tunnel type as VPN). Use `GetBestRoute2` per target to label which Link carries it and to tell "local only" from "down".
7. Set `PRAGMA synchronous=NORMAL` next to `journal_mode=WAL`. It keeps every commit through a process kill and loses at most the last few seconds on power loss.
8. Prototype the open points before relying on them: unbiased time during Modern Standby; `GetBestRoute2` behaviour when the constrained interface lacks a route; ICMP source pinning with a full-tunnel VPN; Wi-Fi settle times on real hardware.

---

## Sources

- [S1] Window Features: message-only windows — https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features
- [S2] RegisterSuspendResumeNotification — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registersuspendresumenotification
- [S3] PowerRegisterSuspendResumeNotification — https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-powerregistersuspendresumenotification
- [S4] Desktop Activity Moderator (Windows 8 cookbook; page updated 2025-03) — https://learn.microsoft.com/en-us/windows/win32/w8cookbook/desktop-activity-moderator
- [S5] PBT_APMSUSPEND — https://learn.microsoft.com/en-us/windows/win32/power/pbt-apmsuspend
- [S6] System Power Management Events — https://learn.microsoft.com/en-us/windows/win32/power/system-power-management-events
- [S7] WM_POWERBROADCAST — https://learn.microsoft.com/en-us/windows/win32/power/wm-powerbroadcast
- [S8] PBT_APMRESUMEAUTOMATIC — https://learn.microsoft.com/en-us/windows/win32/power/pbt-apmresumeautomatic
- [S9] Prepare software for modern standby (updated 2024-06, mentions 24H2) — https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/prepare-software-for-modern-standby
- [S10] Modern Standby vs S3 — https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/modern-standby-vs-s3
- [S11] RegisterPowerSettingNotification — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerpowersettingnotification
- [S12] Registering for Power Events — https://learn.microsoft.com/en-us/windows/win32/power/registering-for-power-events
- [S13] Power Setting GUIDs — https://learn.microsoft.com/en-us/windows/win32/power/power-setting-guids
- [S14] WTSRegisterSessionNotification — https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsregistersessionnotification
- [S15] SystemParametersInfoW (SPI_GETSCREENSAVERRUNNING) — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow
- [S16] Rust `std::time::Instant` (platform table) — https://doc.rust-lang.org/std/time/struct.Instant.html
- [S17] Acquiring high-resolution time stamps — https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps
- [S18] QueryInterruptTime — https://learn.microsoft.com/en-us/windows/win32/api/realtimeapiset/nf-realtimeapiset-queryinterrupttime
- [S19] GetTickCount64 — https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-gettickcount64
- [S20] KeQueryUnbiasedInterruptTime (biased vs unbiased) — https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-kequeryunbiasedinterrupttime
- [S21] QueryUnbiasedInterruptTime — https://learn.microsoft.com/en-us/windows/win32/api/realtimeapiset/nf-realtimeapiset-queryunbiasedinterrupttime
- [S22] NotifyIpInterfaceChange — https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-notifyipinterfacechange
- [S23] MIB_IPINTERFACE_ROW (strong host, DisableDefaultRoutes) — https://learn.microsoft.com/en-us/windows/win32/api/netioapi/ns-netioapi-mib_ipinterface_row
- [S24] NotifyRouteChange2 — https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-notifyroutechange2
- [S25] NotifyNetworkConnectivityHintChange — https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-notifynetworkconnectivityhintchange
- [S26] GetNetworkConnectivityHintForInterface — https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getnetworkconnectivityhintforinterface
- [S27] NL_NETWORK_CONNECTIVITY_LEVEL_HINT — https://learn.microsoft.com/en-us/windows/win32/api/nldef/ne-nldef-nl_network_connectivity_level_hint
- [S28] About the Network List Manager API — https://learn.microsoft.com/en-us/windows/win32/nla/about-the-network-list-manager-api
- [S29] NCSI overview (updated 2026-02) — https://learn.microsoft.com/en-us/windows-server/networking/ncsi/ncsi-overview
- [S30] Microsoft Q&A, Wi-Fi slow to reconnect after sleep (community, not authoritative) — https://learn.microsoft.com/en-us/answers/questions/4329921/windows-11-wifi-is-taking-long-time-to-reconnect-a
- [S31] IPPROTO_IP socket options (IP_UNICAST_IF; updated 2026-05) — https://learn.microsoft.com/en-us/windows/win32/winsock/ipproto-ip-socket-options
- [S32] IPPROTO_IPV6 socket options (IPV6_UNICAST_IF) — https://learn.microsoft.com/en-us/windows/win32/winsock/ipproto-ipv6-socket-options
- [S33] DNS_QUERY_REQUEST — https://learn.microsoft.com/en-us/windows/win32/api/windns/ns-windns-dns_query_request
- [S34] IcmpSendEcho2Ex — https://learn.microsoft.com/en-us/windows/win32/api/icmpapi/nf-icmpapi-icmpsendecho2ex
- [S35] Icmp6SendEcho2 — https://learn.microsoft.com/en-us/windows/win32/api/icmpapi/nf-icmpapi-icmp6sendecho2
- [S36] `windows` 0.62.2 crate source (modules `Win32::System::Power`, `System::WindowsProgramming`, `System::RemoteDesktop`, `System::SystemServices`, `NetworkManagement::IpHelper`, `Networking::WinSock`) — https://crates.io/crates/windows/0.62.2 ; https://docs.rs/windows/0.62.2/windows/
- [S37] The Cable Guy: Strong and Weak Host Models (TechNet Magazine 2007, archived) — https://learn.microsoft.com/en-us/previous-versions/technet-magazine/cc137807(v=msdn.10)
- [S38] TCP/IP raw sockets — https://learn.microsoft.com/en-us/windows/win32/winsock/tcp-ip-raw-sockets-2
- [S39] IP_ADAPTER_ADDRESSES_LH — https://learn.microsoft.com/en-us/windows/win32/api/iptypes/ns-iptypes-ip_adapter_addresses_lh
- [S40] GetAdaptersAddresses — https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getadaptersaddresses
- [S41] Wintun driver INF — https://git.zx2c4.com/wintun/plain/driver/wintun.inf
- [S42] GetBestRoute2 — https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getbestroute2
- [S43] libsqlite3-sys 0.40.0 `build.rs` (bundled compile flags) — https://crates.io/crates/libsqlite3-sys/0.40.0
- [S44] SQLite compile-time options (SQLITE_DEFAULT_SYNCHRONOUS, _WAL_SYNCHRONOUS, _WAL_AUTOCHECKPOINT) — https://www.sqlite.org/compile.html
- [S45] SQLite Write-Ahead Logging — https://www.sqlite.org/wal.html
- [S46] SQLite PRAGMA synchronous — https://www.sqlite.org/pragma.html#pragma_synchronous
