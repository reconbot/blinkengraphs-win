# 04 — Elevated Extras without an elevated UI

Researched 2026-11-21. Question: how should blinkengraphs get its two privileged features without running the Panel and Tray Icons as Administrator? The features are:
- bytes per process on the network;
- raw-socket Hop Traces (UDP, and TCP if `TCP_ICMP_ERROR_INFO` falls short).

The prompt for this note was "if we add an installer, can it install the app with admin rights whenever it's run?" Constraints:
- [ADR 0002](../adr/0002-no-admin-by-default.md): never require admin.
- [ADR 0007](../adr/0007-footprint-is-a-feature.md): exe ≤ 7 MiB, CPU < 1%, working set < 40 MB.
- Managed fleets: Entra, Intune, EDR and security review.

Numbered citations such as [S1] point to the Sources list at the end. Confidence markers, as in 02:
- **(verified)**: read in a primary source.
- **(inferred)**: my reasoning from the sources.
- **(low confidence)**: weak, dated or community-only evidence. Spike it first.

The code base today (checked 2026-11-21) does the following:
- `app/src/win.rs` detects elevation with `TokenElevation`.
- `crates/probe/src/win.rs` already sets `TCP_FAIL_CONNECT_ON_ICMP_ERROR`.
- No ETW or `SOCK_RAW` code exists yet.

---

## 1. The direct answer: an elevated installer does not elevate later launches

- Under UAC, Explorer runs with the user's *standard* token, and "all other user-initiated processes inherit their access token" from it. An app gets the admin token only after a consent prompt (admins) or a credential prompt (standard users) [S1] (verified).
- So an installer that ran elevated changes nothing about how the app is later launched (inferred from [S1]). It can only arrange for elevation later. Each way of doing that is weighed below.
  - Scheduled task with "Run with highest privileges": §3.3.
  - `requireAdministrator` manifest: §3.4.
  - A privileged component that runs separately from the UI (§3.1, §3.2).
- **Administrator protection** is now shipping. It removes auto-elevation: "users need to interactively authorize every admin operation" [S3]. The feature is off by default, but Windows security baselines turn it on from the October 2026 update, and Intune can configure it [S2] (verified). A plan that depends on silent elevation is betting against where managed Windows is going (inferred).

## 2. What each privileged feature actually needs

### 2.1 Bytes per process

| Route | Privilege | Fit | Evidence |
|---|---|---|---|
| NT Kernel Logger (`EVENT_TRACE_FLAG_NETWORK_TCPIP`) | "Only users with administrative privileges and services running as LocalSystem can control an NT Kernel Logger session." There is only one such session system-wide, and it is shared with other tools | Poor: it contends with WPR, Process Explorer and EDR | [S6][S7][S45] (verified) |
| System logger (`EVENT_TRACE_SYSTEM_LOGGER_MODE`, SystemTraceProvider or System IO Provider `SYSTEM_IO_KW_NETWORK`) | A non-admin, non-TCB caller needs the profile privilege plus ACL grants on the session and provider GUIDs. At most 8 system loggers, 2 reserved | Workable for a service | [S6][S8][S9] (verified) |
| **Microsoft-Windows-Kernel-Network** manifest provider `{7dd42a49-5329-4832-8dfd-43d979153a88}` in an ordinary real-time session | Sessions: Administrators, Performance Log Users, or services running as LocalSystem, LocalService or NetworkService [S6][S7] (verified). Whether those principals may *enable this provider* is not documented. Velociraptor says it attaches only "when running as NT_USER/SYSTEM" [S11] (**low confidence**) | **Best candidate.** Events 10/11 (TCP send/recv) and 42/43 (UDP) carry `PID` and `size` | [S10][S11] (verified fields) |
| `GetPerTcpConnectionEStats` | Reading needs no admin. But collection is off by default and must be enabled *per connection* with `SetPerTcpConnectionEStats`, which "can only be called by a user logged on as a member of the Administrators group" and needs `requireAdministrator` under UAC. If enabling fails, reads return "meaningless random data" | Poor: something privileged must enable every new connection. TCP only, so QUIC/UDP traffic is invisible (inferred) | [S13][S14] (verified) |
| `ConnectionProfile.GetAttributedNetworkUsageAsync` (WinRT, per-app usage) | Restricted capability. The API page names `networkDataPlanProvisioning` and the capability page names `networkDataUsageManagement`; the docs disagree | Poor: it returns bytes per time window, not live per-PID rates. Its reconciliation guidance assumes closed buckets and late adjustments | [S15][S16] (verified; that it is SRUM-backed is inferred) |
| PDH "Network Interface" / `GetIfTable2` | None | Per adapter only, never per process | 02 §4 (verified) |
| `GetProcessIoCounters` (`IO_COUNTERS`) | None (`PROCESS_QUERY_LIMITED_INFORMATION`) | Counts "all I/O operations" with no split by kind, so network cannot be separated from file I/O. Usable only as a "busy process" hint | [S17] (verified). That it includes network I/O is (inferred) |
| `GetExtendedTcpTable` | None | Connections per process, no bytes | 02 §4 (verified) |

**Finding:** no documented admin-free API gives live per-process network bytes. The only least-privilege candidate is an ETW real-time session on Microsoft-Windows-Kernel-Network, run by a LocalService-class service or by a user in Performance Log Users. Microsoft documents neither as able to enable that specific provider, so it needs a spike (§5).

### 2.2 Hop Traces

| Protocol | Admin-free route | Status |
|---|---|---|
| ICMP | `IcmpSendEcho2` with `IP_OPTION_INFORMATION.Ttl`. The reply's `Address` names the router [S19] | Shipping. WinMTR precedent (01 §3a) (verified) |
| TCP | `IP_TTL` + `TCP_FAIL_CONNECT_ON_ICMP_ERROR`. `connect` returns `WSAEHOSTUNREACH`, and "the source address of the error will then be available via the `TCP_ICMP_ERROR_INFO` socket option". `ICMP_ERROR_INFO` carries `srcaddress`, type and code. Windows 10 2004+ | Documented and admin-free [S20][S21] (verified). Whether real routers' time-exceeded replies arrive this way on real networks is unproven ([ADR 0006](../adr/0006-tcp-hop-traces-experimental.md)) |
| UDP | None found. The `IPPROTO_IP` option list has no `IP_RECVERR`-style way for a datagram socket to receive ICMP errors [S22] (verified absence). Raw sockets need Administrators: "only members of the Administrators group can create sockets of type SOCK_RAW" [S18] (verified) | Needs a raw socket. PingPlotter says its UDP mode was reworked to "work out of the box" from v4.11.1, by an unpublished mechanism [S40] (**low confidence**) |

Raw sockets are checked against **Administrators membership**, not against a privilege a service can be granted [S18]. Among the built-in service accounts, only LocalSystem's token contains `BUILTIN\Administrators` [S23]. LocalService has "minimum privileges" and is not an administrator [S24] (verified). **So a raw-socket extra forces LocalSystem**; the ETW extra may not (inferred).

## 3. Options compared

### 3.1 Companion Windows service + named pipe (the proposed answer)

**Account (least privilege):**

| Account | ETW Kernel-Network | Raw sockets | Notes |
|---|---|---|---|
| LocalService, with service SID type `RESTRICTED`/`UNRESTRICTED` and trimmed required privileges | Probably. It may control sessions [S6][S7]; provider enable is unverified (§2.1) | No [S18][S24] | Preferred if the spike passes. Ownership can be granted to the service SID with `EventAccessControl` if the default ACL falls short [S7] (inferred) |
| NetworkService | Same as LocalService | No | Presents machine credentials on the network [S23]; this service needs no network identity (inferred) |
| Virtual account (`NT SERVICE\<name>`) | Not in the default ETW list [S7]. It would need an `EventAccessControl` grant or Performance Log Users membership (inferred) | No | Narrowest identity, most setup (inferred) |
| LocalSystem | Yes, including the NT Kernel Logger [S6] | Yes [S18][S23] | Microsoft: "Most services do not need such a high privilege level" [S23]. Trim it with `SERVICE_REQUIRED_PRIVILEGES_INFO`: the SCM "will remove the privileges that are not required from the process token" [S25]. The Administrators SID stays (inferred) |

**Pipe ACL and client validation:**
- The default pipe DACL grants read to Everyone and Anonymous [S28][S29] (verified), so set an explicit DACL. Use SYSTEM plus Interactive Users, or the console session's logon SID, which Microsoft recommends "to prevent remote users or users on a different terminal services session" from connecting [S28].
- Set `PIPE_REJECT_REMOTE_CLIENTS` [S29].
- Create the first instance with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so another process that already squats the name makes the service fail closed [S29] (verified flag; squatting rationale inferred).
- Server side: `GetNamedPipeClientProcessId` [S30], then check that the client's image path is the installed app under `%ProgramFiles%` (inferred). That is not a security boundary against code already running as the same user, so the protocol must be safe for *any* local user to call (inferred).
- Client side: `GetNamedPipeServerProcessId` must equal the PID from `QueryServiceStatusEx`, so the UI never talks to an impostor (inferred).

**Attack surface** (all inferred unless cited):
- Keep the service to a fixed, read-only vocabulary: "subscribe to per-PID byte deltas" and "trace target X with protocol P at ≤ 1 Hz". No paths, no command lines, no arbitrary packet bytes. A raw-socket service that accepts caller-chosen packets is a packet-crafting oracle for any local user.
- Per-PID bytes for *all* sessions is a cross-user information leak. Filter to processes in the caller's session.
- Tailscale's Windows local API, bound to localhost TCP with no Host-header check, was exploitable from a web page as CVE-2022-41924 (CVSS 9.6) [S37] (verified). Use a pipe, not a localhost socket.
- Write the service in Rust with no egui and no text rendering: a small parser and a small privileged binary.

**Install / uninstall:**
- **MSI (WiX):** `ServiceInstall` with `StartName` `NT AUTHORITY\LocalService` or LocalSystem [S27]. `MsiServiceConfig` sets `SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO` and `SERVICE_CONFIG_SERVICE_SID_INFO` declaratively (Windows Installer 5.0+) [S26]. Uninstall deletes the service only if `ServiceControl` has `msidbServiceControlEventUninstallDelete` [S27] (verified). The install is per-machine and elevated (inferred).
- **Intune:** a Win32 app in system context; installs must be silent [S33] (verified).
- **winget:** set `ElevationRequired: elevationRequired` and machine `Scope` [S34] (verified).
- **MSIX:** services work from Windows 10 2004 and need admin to install [S31][S32].
  - They also need the `packagedServices` restricted capability, plus `localSystemServices` for LocalSystem. Microsoft does not recommend these for Store apps: "In most cases, the use of this capability won't be approved." [S16] (verified).
  - Fine for Intune or sideload, not for the Store.
- **Keep the base app installable per user without admin** (02 §6). Ship the service as an optional MSI feature or a separate package that an admin or Intune adds (inferred).

**Footprint** (inferred):
- A separate service exe keeps egui out of the privileged binary and leaves the 7 MiB budget to the UI exe. Expect a few hundred KiB to ~1 MiB with `windows` + `ferrisetw`, not yet measured.
- ETW cost scales with send/recv calls. Follow [ADR 0003](../adr/0003-process-snapshot-only-while-panel-open.md) and Microsoft's ETW guidance: "only start the trace when collection is specifically needed … stop the trace as soon as the scenario is completed" [S6] (verified quote). The session runs only while the Panel is open, and the service is otherwise idle.
- Start the service on demand rather than at boot (inferred).

**Verdict:** best fit for managed fleets and the only option that works for standard users. Its weight drops sharply if it can run as LocalService with ETW only.

### 3.2 Performance Log Users instead of a service

- Members may control sessions and "provide and consume event data" by default [S7]. Members "can't use the Windows Kernel Trace event provider" [S12] (verified).
- It covers ETW only, not raw sockets [S18].
- Intune can add users through the LocalUsersAndGroups CSP (`U` = update action) [S35] (verified).
  - Entra *group* SIDs must be given as SIDs, and the CSP says Entra logon privileges are evaluated only for Administrators, Users, Guests, Power Users, Remote Desktop Users and Remote Management Users [S35].
  - Whether an Entra-group SID in Performance Log Users reaches the user's token is **low confidence**. Per-user `AzureAD\UPN` entries avoid the question.
- A per-user installer cannot do it: changing local group membership needs admin (inferred).
- **Security cost:** membership hands every process the user runs, malware included, machine-wide rights to start traces and consume real-time events (inferred from [S7]). That is broader than a single-purpose service. A security reviewer is likely to prefer the service (inferred).

**Verdict:** acceptable where the org already grants the group to engineers; not the default.

### 3.3 Scheduled task at logon, "Run with highest privileges", whole app elevated

- **Standard users:** `HighestAvailable` gives them nothing. The task runs with their standard token. Registering a `TASK_RUNLEVEL_HIGHEST` task needs an elevated process [S4][S5] (verified). On a fleet of standard users the app stays unprivileged (inferred).
- **Admin users:** it silently starts an elevated process at every logon. That is the persistent, no-prompt elevation pattern UAC and Administrator protection exist to remove [S1][S3]. Administrator protection removes auto-elevation and separates profiles, so the elevated app's HKCU and library folders map to a hidden system-managed admin account, not the user's [S3] (verified). The app's settings and SQLite history would split between two profiles (inferred).
- Whether highest-privilege tasks still run interactively under Administrator protection is not documented by Microsoft. A vendor KB says they cannot [web: 2BrightSparks] (**low confidence**).
- **egui UI as admin:**
  - A high-integrity UI widens the attack surface: config and TOML parsing, font rendering, clipboard. Anything the user can drive becomes admin code (inferred).
  - Lower-integrity processes "can't modify data in applications with higher integrity levels" [S1], so drag-and-drop and some shell messages from Explorer are filtered (inferred; not tested).
  - EDR tooling commonly flags elevated persistence through scheduled tasks (inferred, not sourced).

**Verdict:** reject. It fails standard users, and for admins it is a UAC bypass by design.

### 3.4 `requireAdministrator`: whole app, or an on-demand helper

- **Whole app:** standard users get a credential prompt they cannot answer, and admins get a prompt at every launch, which also breaks autostart [S1] (verified prompt behavior). That contradicts ADR 0002. Reject.
- **On-demand helper:** a tiny `requireAdministrator` helper exe, launched with `ShellExecuteEx("runas")` when the user clicks "Enable Elevated Extras", which streams data back over the same pipe protocol as §3.1 (inferred design).
  - Leaves no persistent privileged code and needs no installer.
  - Prompts every session; under Administrator protection that is Windows Hello every time, and the helper runs in the separate admin profile [S2][S3].
  - Useless for standard users without admin credentials [S1].
  - Pairs naturally with §3.1, since it is the same binary and protocol run as a process instead of a service (inferred).
  - **Verdict:** good fallback for unmanaged power users; not a fleet answer.

### 3.5 Avoid privilege

- **Hop Traces:** ICMP is done. TCP is documented admin-free (§2.2) and needs only ADR 0006's real-network check. Only **UDP** genuinely needs raw sockets. UDP traceroute adds little over ICMP and TCP for "where does the path break?" (inferred), so dropping it removes the only reason for LocalSystem.
- **Per-process bytes:** nothing admin-free is accurate (§2.1). The honest admin-free Panel shows connections per process (`GetExtendedTcpTable`) and labels total process I/O as "I/O", not "network" (inferred).

## 4. Precedent

| Product | How it gets privilege | Source |
|---|---|---|
| Tailscale | `tailscaled` runs as a Windows service named "Tailscale", "the more privileged daemon that does all the network handling". The tray GUI is separate and installed per-machine by MSI. CVE-2022-41924 came from its localhost-TCP local API | [S36][S37] (verified). Service account not documented (**low confidence**) |
| NetLimiter | Driver, service and client. The service is "a proxy which connects user interface with the driver" | [S38] (verified, 2015 doc) |
| GlassWire | Kernel driver plus a service, with the GUI separate | [S39] (changelog confirms driver). Service account **low confidence** |
| Windows Resource Monitor | Its network view is built from ETW (Kernel-Network among the providers), and it is reproducible with `logman` | [S41] (verified; whether it elevates is **low confidence**) |
| Sysinternals TCPView | Endpoint list with no admin requirement stated; the page documents no byte counters | [S42] (verified absence) |
| PingPlotter | Default ICMP "Windows DLL" route needs no admin. Raw ICMP/UDP need admin; TCP needs Npcap, or running "as a service" | [S40] (verified for the v5 manual) |
| WinMTR | `IcmpSendEcho2` + TTL, no admin | 01 §3a (verified) |
| Trippy | "must be run with Administrator privileges on Windows" | [S43] (verified) |

The pattern for products that need kernel or ETW data continuously is UI as user plus a separate privileged service. Tools that run elevated whole are run-on-demand diagnostics (inferred).

## 5. Spikes before committing

All run on CI (`windows-latest`) and on one real network (inferred plan):
1. **LocalService + Kernel-Network:** install a test service as `NT AUTHORITY\LocalService` and start a real-time session enabling `{7dd42a49-…}` keywords `0x10|0x20`. Count events 10/11/42/43 per PID against `GetIfTable2` totals. Repeat as a Performance Log Users member and as a virtual account with an `EventAccessControl` grant. This settles §3.1's account and §3.2.
2. **CPU cost:** at 1 Gbit/s sustained, measure service CPU with the session on, against the < 1% budget.
3. **ADR 0006 on a real network:** if TCP Hop Traces work admin-free, the raw-socket extra shrinks to UDP only.
4. **Administrator protection:** with it on, confirm the §3.4 helper prompts once per session and that the service route is unaffected.

## 6. Recommendation

**Answer to the question:** no. An installer that runs as admin does not make later launches admin [S1]. The ways to force that (§3.3, §3.4 whole-app) are what UAC and Administrator protection are removing [S2][S3].

**Ranked:**
1. **Shrink the problem first.** Ship ICMP and TCP Hop Traces admin-free (§2.2, ADR 0006). Drop UDP Hop Traces, or keep them only when the app is already elevated, as today. The single remaining Elevated Extra is then per-process bytes.
2. **Optional companion service, least privilege.** A separate small Rust service exe, LocalService with a restricted service SID and trimmed privileges, consuming Microsoft-Windows-Kernel-Network only while the Panel is open. It talks over a DACL'd, local-only named pipe with a fixed read-only protocol, filtered to the caller's session. It ships as an optional per-machine MSI feature, deployed by Intune or winget, and the base app stays per-user. Fall back to LocalSystem with `SERVICE_REQUIRED_PRIVILEGES_INFO` trimming only if spike 1 fails.
3. **On-demand elevated helper** (§3.4) for unmanaged admin users. Same binary and protocol, no installer, one prompt per session.
4. **Performance Log Users** (§3.2) only where the org already grants it. ETW only, and it broadens the user rather than one service.
5. **Rejected:** scheduled task "highest privileges" (§3.3); `requireAdministrator` on the whole app (§3.4).

**What would change the ranking:**
- Spike 1 fails for LocalService, Performance Log Users and virtual accounts alike: option 2 becomes LocalSystem. It still ranks first for fleets, but expect a harder security review, and §3.4 gains ground for individuals.
- ADR 0006's TCP route fails on real networks, or UDP traces become a must-have: raw sockets return, which forces LocalSystem [S18][S23].
- Fleet users are local admins with Administrator protection off: §3.4 becomes cheap enough to be the default.
- Microsoft Store distribution becomes a goal: packaged LocalSystem services "won't be approved" in most cases [S16], so option 2 needs MSI or Intune, or the extras go.
- A documented admin-free per-process byte API appears: options 2–4 become moot.

---

## Sources

- [S1] How User Account Control works: https://learn.microsoft.com/en-us/windows/security/application-security/application-control/user-account-control/how-it-works
- [S2] Administrator protection (updated 2026-11-21): https://learn.microsoft.com/en-us/windows/security/application-security/application-control/administrator-protection
- [S3] Windows Developer Blog, "Enhance your application security with administrator protection" (2025-05-19): https://blogs.windows.com/windowsdeveloper/2025/05/19/enhance-your-application-security-with-administrator-protection/
- [S4] Task Scheduler, Security Contexts for Tasks: https://learn.microsoft.com/en-us/windows/win32/taskschd/security-contexts-for-running-tasks
- [S5] Task Scheduler, RunLevel element: https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-runlevel-principaltype-element
- [S6] StartTraceW (access rules, system loggers, session limits): https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew
- [S7] EventAccessControl (default ETW permissions): https://learn.microsoft.com/en-us/windows/win32/api/evntcons/nf-evntcons-eventaccesscontrol
- [S8] Configuring and Starting a SystemTraceProvider Session: https://learn.microsoft.com/en-us/windows/win32/etw/configuring-and-starting-a-systemtraceprovider-session
- [S9] ETW System Providers: https://learn.microsoft.com/en-us/windows/win32/etw/system-providers
- [S10] Microsoft-Windows-Kernel-Network manifest dump (repnz/etw-providers-docs, Win10 18990): https://raw.githubusercontent.com/repnz/etw-providers-docs/master/Manifests-Win10-18990/Microsoft-Windows-Kernel-Network.xml
- [S11] Velociraptor artifact Windows.ETW.KernelNetwork: https://docs.velociraptor.app/artifact_references/pages/windows.etw.kernelnetwork
- [S12] Active Directory security groups, Performance Log Users: https://learn.microsoft.com/en-us/windows-server/identity/ad-ds/manage/understand-security-groups
- [S13] GetPerTcpConnectionEStats: https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getpertcpconnectionestats
- [S14] SetPerTcpConnectionEStats: https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-setpertcpconnectionestats
- [S15] ConnectionProfile.GetAttributedNetworkUsageAsync: https://learn.microsoft.com/en-us/uwp/api/windows.networking.connectivity.connectionprofile.getattributednetworkusageasync
- [S16] App capability declarations (restricted capabilities): https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations
- [S17] IO_COUNTERS and GetProcessIoCounters: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-io_counters , https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getprocessiocounters
- [S18] TCP/IP raw sockets: https://learn.microsoft.com/en-us/windows/win32/winsock/tcp-ip-raw-sockets-2
- [S19] IcmpSendEcho2: https://learn.microsoft.com/en-us/windows/win32/api/icmpapi/nf-icmpapi-icmpsendecho2
- [S20] IPPROTO_TCP socket options (`TCP_FAIL_CONNECT_ON_ICMP_ERROR`, `TCP_ICMP_ERROR_INFO`): https://learn.microsoft.com/en-us/windows/win32/winsock/ipproto-tcp-socket-options
- [S21] ICMP_ERROR_INFO: https://learn.microsoft.com/en-us/windows/win32/api/ws2ipdef/ns-ws2ipdef-icmp_error_info
- [S22] IPPROTO_IP socket options: https://learn.microsoft.com/en-us/windows/win32/winsock/ipproto-ip-socket-options
- [S23] LocalSystem account: https://learn.microsoft.com/en-us/windows/win32/services/localsystem-account
- [S24] LocalService account: https://learn.microsoft.com/en-us/windows/win32/services/localservice-account
- [S25] SERVICE_REQUIRED_PRIVILEGES_INFOW: https://learn.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-service_required_privileges_infow
- [S26] MsiServiceConfig table: https://learn.microsoft.com/en-us/windows/win32/msi/msiserviceconfig-table
- [S27] ServiceInstall table: https://learn.microsoft.com/en-us/windows/win32/msi/serviceinstall-table
- [S28] Named Pipe Security and Access Rights: https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights
- [S29] CreateNamedPipeW: https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew
- [S30] GetNamedPipeClientProcessId: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getnamedpipeclientprocessid
- [S31] MSIX, Converting an installer with services: https://learn.microsoft.com/en-us/windows/msix/packaging-tool/convert-an-installer-with-services
- [S32] MSIX features and supported platforms: https://learn.microsoft.com/en-us/windows/msix/supported-platforms
- [S33] Intune Win32 app management: https://learn.microsoft.com/en-us/intune/app-management/deployment/win32
- [S34] winget installer manifest schema 1.12.0 (`ElevationRequirement`, `Scope`): https://github.com/microsoft/winget-pkgs/blob/master/doc/manifest/schema/1.12.0/installer.md
- [S35] LocalUsersAndGroups Policy CSP: https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-localusersandgroups
- [S36] Tailscale, tailscaled daemon: https://tailscale.com/kb/1278/tailscaled ; MSI install: https://tailscale.com/kb/1189/install-windows-msi
- [S37] GHSA-vqp6-rc3h-83cp / CVE-2022-41924: https://github.com/advisories/GHSA-vqp6-rc3h-83cp
- [S38] NetLimiter components: https://www.netlimiter.com/docs/internals/3-netlimiter-components
- [S39] GlassWire changelog: https://www.glasswire.com/changes/
- [S40] PingPlotter manual, packet options and raw TCP sockets: https://pingplotter.com/manual/packetoptions , https://pingplotter.com/manual/raw_tcp_sockets_and_winpcap
- [S41] "How to pull the information that Resource Monitor (ResMon.exe) provides" (Microsoft blog archive): https://learn.microsoft.com/en-us/archive/blogs/yongrhee/how-to-pull-the-information-that-resource-monitor-resmon-exe-provides
- [S42] TCPView: https://learn.microsoft.com/en-us/sysinternals/downloads/tcpview
- [S43] Trippy privileges guide: https://trippy.rs/guides/privileges/
- [S45] Configuring and Starting the NT Kernel Logger Session: https://learn.microsoft.com/en-us/windows/win32/etw/configuring-and-starting-the-nt-kernel-logger-session
- [web: 2BrightSparks] Administrator Protection and Task Scheduler (vendor KB, secondary): https://www.2brightsparks.com/syncback/help/adminprotwindows.htm

### Known gaps
- **Not documented anywhere I found:** which principals may enable Microsoft-Windows-Kernel-Network. Spike 1 decides.
- **Not fetched:** the Process PDH counter Explain text, which would confirm that `IO_COUNTERS` includes network I/O; `GetNamedPipeServerProcessId` and `QueryServiceStatusEx` pages (standard APIs, cited by name only); `ChangeWindowMessageFilterEx` for the UIPI point.
- **Low confidence:** service accounts used by Tailscale and GlassWire; PingPlotter's admin-free UDP mechanism; scheduled-task behavior under Administrator protection.
