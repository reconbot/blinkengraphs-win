# 03 — macOS and iOS: what to share with the Windows build

Researched 2026-11-21. Question: should a macOS menu bar app and an iOS app share a Rust core with the Windows build, or stay separate and agree only on rules, graphing concepts and vocabulary? Numbered citations such as [S1] point to the Sources list at the end. Crate versions come from the crates.io API on 2026-11-21.

Confidence markers, as in 02:
- **(verified)**: read in a primary source (Apple docs, XNU/IOKit source, crate README or docs).
- **(inferred)**: my reasoning from the sources.
- **(low confidence)**: forum or community evidence only. Prototype before relying on it.

---

## 1. What the current code would share

Lines of Rust per crate from `wc -l`, split at `mod tests`. Totals: about 5,340 lines of code and 740 lines of tests.

| Crate | Code | Tests | Depends on | Portability |
|---|---:|---:|---|---|
| `model` | 269 | – | serde | Portable, but `Metric` is Windows-shaped: `MemCommit`, `MemHardFaults`, `DiskResponse`, `DiskQueue`, `CpuQueue` are PDH concepts. |
| `verdict` | 285 | 123 | model, history | Portable logic. It reads `MemCommit`, `MemHardFaults`, `DiskActive`, `DiskResponse` and `NetUp`, so the Local Load rules assume Windows metrics. |
| `history` | 341 | 89 | model, rusqlite (bundled) | Portable. On Apple platforms SQLite ships with the OS, so the bundled copy only adds size. |
| `config` | 214 | 43 | serde, toml | Portable, but a TOML file in `%APPDATA%` is not idiomatic on macOS or iOS, where UserDefaults is normal. |
| `fixtures` | 213 | – | model, history | Portable. These are synthetic scenarios already normalized to 0..=1, so they are halfway to golden vectors. |
| `render` | 242 | 33 | tiny-skia, ab_glyph | Portable. Holds `cube_root` scaling, the `WARN` 0.6 and `CRIT` 0.85 thresholds, and Meter colors. |
| `meters` | 203 | 33 | render, verdict, fixtures | Portable. |
| `engine` | 208 | – | collect, probe | Portable orchestration, but it is wired to the Windows collectors. |
| `collect/calc.rs` | 151 | 150 | model | Portable counter math: deltas, rates, shares. |
| `collect` (lib, stub, `win/`) | 640 | – | windows-sys (PDH, IP Helper) | Windows only. |
| `probe` (lib, dns, net, resolve, trace) | 577 | 143 + 124 integ. | socket2 | Mostly portable: DNS packet build and parse, hop statistics, the scheduler. |
| `probe/win.rs` | 376 | – | windows-sys (IcmpSendEcho2) | Windows only. Apple platforms would use an unprivileged ICMP socket instead [S1]. |
| `panel` | 893 | – | egui, egui_plot | Builds on macOS, but egui draws its own widgets, so the Panel would not look native. Not usable as an iOS UI. **(inferred)** |
| `app` | 730 | – | Win32 | Windows only: tray, Taskbar Strip, hotkey, session events. |

Grouped:

| Group | Code | What it is |
|---|---:|---|
| Rules and data (model, verdict, history, config, fixtures) | ~1,320 | The part the owner wants consistent |
| Graphing (render, meters) | ~445 | cube-root scaling, thresholds, colors, loss marks |
| Portable plumbing (engine, calc, probe minus win.rs) | ~935 | Useful to a macOS app, partly useful to iOS |
| Windows-bound (collect win, probe win, app) | ~1,750 | Rewritten per platform whatever the choice |
| egui Panel | ~890 | Replaced by SwiftUI on Apple platforms if a native look matters |

The part that carries the consistency requirement, Verdict's rules, is **285 lines**. The graphing rules that matter across platforms are about a dozen lines: `cube_root`, two thresholds and the color table. Colors are already duplicated today between `render/src/lib.rs` (tiny-skia `rgba`) and `panel/src/theme.rs` (egui `Color32`). **(verified, from the repo)**

## 2. Platform capabilities without special entitlements

### macOS

| Need | API | Unsandboxed, direct distribution | App Sandbox (required for the Mac App Store [S2]) |
|---|---|---|---|
| CPU total and per core | `host_statistics64`, `host_processor_info` (Mach) [S3] | Works. This is what Activity Monitor-style tools read. **(inferred)** | `host_statistics64` is documented for macOS 10.6+ [S3]. Per-core via `host_processor_info` is reported to work in the sandbox; one forum report says per-processor totals don't add up there [S4]. **(low confidence)** |
| Memory Pressure | `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE` (normal, warn, critical events) [S5]; `sysctl kern.memorystatus_level`, a read-only kernel int [S6]; `host_statistics64` VM counters for compressed memory and swap | Works | Dispatch source works on all Apple platforms [S5]. The sysctl is undocumented. An Apple DTS engineer warns that "free memory" is not a useful statistic [S7]. **(low confidence on sysctl in sandbox)** |
| Per-process CPU and memory | `libproc` (`proc_listpids`, `proc_pidinfo`, `proc_pid_rusage`) | Works for the user's processes. Other users' and root processes need privileges. **(low confidence)** | Blocked: "Those APIs are also blocked by the App Sandbox, failing with EPERM" [S8]; logs show `deny process-info-listpids` [S9]. |
| Network bytes per interface | `sysctl` `NET_RT_IFLIST2` returns `if_msghdr2` with `if_data64` (64-bit counters) [S10]. `getifaddrs` gives `if_data` with 32-bit `ifi_ibytes`, which wraps at 4 GiB [S11]. | Works | Believed to work; the Stats and iStat Menus App Store builds show network rates. **(low confidence)** |
| Disk throughput and latency | IOKit `IOBlockStorageDriver` "Statistics" dictionary: bytes, operations, total read/write time [S12] | Works | IOKit reads are allowed in some App Store apps; Apple has not stated a policy. **(low confidence)** |
| ICMP echo without root | `socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP)`. "Apple platforms include a special facility that allows you to ping without privileges" [S1] | Works | Needs `com.apple.security.network.client` [S13]. **(inferred)** |
| Hop Trace | `IP_TTL` on the ICMP datagram socket | Reported inconsistent on some networks; one developer switched to UDP probes plus a passive ICMP receiver [S14] **(low confidence)** | Same |
| Probe to the Gateway | Any TCP/UDP to a LAN address | **Local network privacy applies to macOS 15+**: a TCP connect or UDP send to a local address triggers a user prompt. Daemons, root and Terminal tools are exempt; `launchd` agents are not [S15]. | Same prompt. DNS to a local resolver is exempt [S15]. TN3179 does not mention ICMP. **(low confidence whether ICMP is gated)** |
| Menu bar item | `NSStatusItem` via `NSStatusBar.statusItem(withLength:)` [S16]; SwiftUI `MenuBarExtra` | Works | Works |

Precedent: iStat Menus 7 ships both on the Mac App Store [S17] and as a direct download. A user reports the App Store build lacks fan control and CPU frequency, and Bjango says it follows the same sandbox rules [S18] **(low confidence: forum)**. Stats (exelban/stats, macOS 12+) is distributed only as a DMG and through Homebrew [S19].

Implication: per-process lists, the Process Snapshot, need direct distribution. Everything else in today's five Sections appears possible in the sandbox. **(inferred)**

Rust options on macOS: `tray-icon` 0.26.1 supports macOS menu bar items [S20], `objc2-app-kit` 0.3.2 binds AppKit [S21], and `sysinfo` 0.40.4 [S22] and `libproc` 0.15.0 [S23] wrap the collectors. An all-Rust macOS build is possible; a native-looking Panel is not, unless it is written in SwiftUI. **(inferred)**

### iOS

| Need | Availability to a sandboxed App Store app |
|---|---|
| System-wide CPU | Not supported. DTS: iOS apps are meant to see only themselves; grandfathered Mach host APIs "may be restricted later" [S24]. `host_statistics` is in the iOS SDK; whether App Review accepts it needs a direct answer from App Review [S25]. `host_processors` fails without privileges [S26]. **(low confidence, forum)** |
| System-wide memory | Only pressure events for your own process via the dispatch source [S5]. sysctl is "heavily restricted" on modern iOS and tightening [S27]. |
| Per-process | Own process only [S24] |
| Network bytes per interface | Unknown. sysctl is restricted [S27]; I found no current primary source on whether `getifaddrs` `if_data` counters are still populated. **(low confidence: needs a device test)** |
| Network path | `NWPathMonitor`, iOS 12+: status, interface type, expensive/constrained flags [S28] |
| ICMP echo | SimplePing: "the core code works just fine on all versions of iOS" [S1]. Ping and traceroute apps exist on the App Store [S14]. |
| Gateway probes | Local network privacy since iOS 14: TCP/UDP to LAN addresses prompts the user and needs `NSLocalNetworkUsageDescription` [S15]. Multicast and broadcast need an extra entitlement [S15]. |
| Disk space | Required-reason API: `statfs` and `volumeAvailableCapacityKey` need privacy-manifest reason `85F4.1` or `E174.1`; displaying disk space has its own reason and the data may not leave the device [S29]. |
| Uptime and timers | `mach_absolute_time` and `systemUptime` are required-reason APIs: declare `35F9.1` for elapsed-time measurement [S29]. Apps that omit the manifest have been rejected since 2024-05-01 [S29]. |
| Background sampling | None continuous. `BGAppRefreshTask` is "a short task" for "small bits of information" and needs the `fetch` background mode [S30]. Guideline 2.5.4: background services only for "VoIP, audio playback, location, task completion, local notifications" [S31]. |
| Widgets | WidgetKit budget: "40 to 70 refreshes" a day for a frequently viewed widget, "roughly … every 15 to 60 minutes"; timeline entries "at least about 5 minutes apart" [S32] |
| Live Activities | Up to eight hours; no network access in the Live Activity itself; 4 KB payload limit; updated from the app or by ActivityKit push [S33] |

Relevant App Review rules [S31]:
- 1.1.6 rejects "inaccurate device data".
- 2.3.1 cites "iOS-based virus and malware scanners" as misleading.
- 2.4.2 rejects apps that "rapidly drain battery".
- 2.5.1 requires public APIs only.
- 2.5.5 requires apps to work on IPv6-only networks, which applies to probes hard-coded to 1.1.1.1 and 8.8.8.8. **(inferred)**

What an iOS version can be **(inferred)**: a foreground network diagnosis tool. It would offer the Latency and Network Verdicts (Gateway, ISP, DNS, Remote, ICMP Filtering), `NWPathMonitor` path state, Hop Traces, and possibly a widget showing the last result. It would have no CPU, Memory or Disk Sections and no continuous monitoring. Self-Saturation depends on interface byte counters, which are unverified on iOS. The Local Load and Self-Saturation Fault Domains mostly drop out.

## 3. Rust and Swift interop

| Tool | Version, date | How it works | Fit |
|---|---|---|---|
| UniFFI (mozilla) | 0.32.2, 2026-09-23 [S34] | Proc-macro or UDL; generates a C header, a modulemap and a Swift file [S35]. "Ready for production use", "a long way from a 1.0 release", "more advanced things might break as you upgrade" [S36]. Swift 6 support is partial [S35]. | Most used. Breaking releases come several times a year (0.29 to 0.32 since 2025) [S37]. |
| swift-bridge | 0.1.59, 2026-01-06 [S34] | Bridge module macro; generates the Swift and C sides; supports async and generics [S38] | Smaller community, still 0.1.x |
| cbindgen + hand-written Swift | 0.29.4, 2026-06-09 [S34] | C header from Rust; Swift wrapper written by hand. libsignal does this: Swift on "the C FFI produced by rust/bridge/ffi/" [S39]. | Most control, most boilerplate |
| cargo-swift | 0.11.1, 2026-05-20 [S34] | Packages a UniFFI crate as a Swift Package with an XCFramework [S40] | Convenience; each cargo-swift release is pinned to a UniFFI version [S40] |

Packaging pattern used in production: build static libs for `aarch64-apple-ios`, `aarch64-apple-ios-sim` and macOS, then combine them into an XCFramework. Publish it as a zip with a checksum and reference it from `Package.swift` as a `.binaryTarget(url:checksum:)` [S39][S41]. Ferrostar documents the steps by hand [S42].

Projects shipping a Rust core to Swift:

| Project | Binding | Distribution |
|---|---|---|
| Signal libsignal | cbindgen-style C FFI plus a Swift package | Prebuilt XCFramework per release; the Swift package and binary must share a version or linking fails on `signal_ffi_version_X_Y_Z` [S39] |
| Matrix rust-sdk / Element X iOS | UniFFI (`matrix-sdk-ffi`) [S43] | Separate `matrix-rust-components-swift` package pinned with `exactVersion: 26.11.17` [S44] |
| Mozilla application-services / Firefox iOS | UniFFI | Formerly through `rust-components-swift` with XCFramework binary targets; now deprecated, and "firefox-ios gets releases directly from application-services" [S45] |
| Bitwarden | UniFFI (`bitwarden-uniffi`) [S46] | `sdk-swift` package with `BitwardenFFI.xcframework` as a remote binary target [S47] |
| 1Password | Rust core ("1Password Core") with SwiftUI on iOS [S48][S49] | Typeshare generates Swift, Kotlin and TypeScript types from Rust [S50] |

Binary size **(low confidence: anecdotal and build-dependent)**: one test produced a ~25 MB release static lib for aarch64 iOS [S51]. Another went from 31 MB to 7.1 MB with fat LTO, and reached ~1 MB compressed with `panic=abort`, strip, and a rebuilt std [S52]. XCFrameworks include simulator slices that App Store thinning removes [S53]. For a ~2 kLOC rules crate with no heavy dependencies, expect low single-digit MB before thinning. **(inferred)**

Costs the projects above accept: a second toolchain in the iOS build, a release pipeline for binaries, version lockstep between the Swift wrapper and the binary [S39], and UniFFI upgrade churn [S36]. These projects share large cryptography or protocol cores, where a second implementation is a security risk. blinkengraphs' shared logic is ~285 lines of rules. **(inferred)**

## 4. Alternatives: share a spec and test vectors, not code

Precedent for language-neutral conformance suites:
- **JSON Schema Test Suite**: "a set of JSON objects that implementers … can use to test their implementations … language agnostic and should require only a JSON parser". It is consumed as a git submodule or subtree [S54].
- **WHATWG URL / web-platform-tests `urltestdata.json`**: URL parsing tests "suitable for any URL parser implementation", each with an `input` and the expected fields [S55].
- **Test262**: the conformance suite for ECMA-262, shared by every JavaScript engine [S56].

Applied here **(inferred)**:
- A `spec/` folder or repo holding `verdict/*.json` cases: probe stats, metric values and upload peak in; ordered Findings (Fault Domain, Severity, headline key) out.
- `scaling.json` holding `cube_root` input/output pairs, plus loss-mark and threshold cases.
- Rust tests in `verdict` and `render` load the cases; a Swift `XCTest` or Swift Testing target loads the same files.
- `crates/fixtures` already holds deterministic scenarios; exporting them as JSON gives the vectors.

Design tokens: the W3C Design Tokens Community Group format reached its first stable version, 2025.10, on 2025-10-28 [S57][S58]. Style Dictionary generates platform outputs including iOS from one token source [S59]. For a dozen colors and two thresholds, a hand-maintained JSON read by a `build.rs` and a Swift codegen script may be enough. **(inferred)**

Vocabulary: `CONTEXT.md` is already platform-neutral apart from Windows terms (Tray Icons, Taskbar Strip, Elevated Extras, commit charge). A macOS build would need Surface names such as Menu Bar Item, and Memory Pressure would need a platform definition. **(inferred)**

## 5. Repository structures

| Option | Shape | Versioning | Tradeoffs **(inferred)** |
|---|---|---|---|
| (a) Monorepo | `crates/` core + `apple/` Xcode project + Windows `app/` | One version, atomic changes to rules and all apps | Mac-only CI runners for Apple builds; release-please per component; Xcode project noise in a Rust repo |
| (b) Separate repos depending on a published core | Core repo publishes crates plus an XCFramework zip; Swift package uses `.binaryTarget(url:checksum:)` [S41] | Semver on the core. Apps pin exact versions, as Element X does [S44]. Wrapper and binary must match [S39]. | Binary release pipeline; FFI layer to maintain; Rust toolchain stays out of the Swift repo; each rule change is a core release followed by a bump in each app |
| (c) Separate repos + shared spec repo | Spec repo with JSON vectors, token file and vocabulary; each app pulls it as a git submodule or subtree, as JSON Schema recommends [S54] | Spec versioned by tag; each app declares which spec version it passes | No FFI and no cross-compilation; rules are implemented twice (~285 lines of Verdict, each side); drift is caught only by the cases the vectors cover |

A hybrid exists: (c) for iOS, which needs only the network rules, and (a) or (b) for macOS. The macOS app could reuse `probe`, `history` and `engine` directly in an all-Rust app [S20][S21]. **(inferred)**

## 6. Implications (inputs to the proposal, not a recommendation)

- The shareable rule surface is small: Verdict 285 lines, scaling and thresholds about a dozen. FFI tooling, XCFramework release and version lockstep cost more than rewriting it in Swift. **(inferred)**
- `model::Metric` and Verdict's Local Load rules encode Windows concepts (commit charge, hard faults, disk response time). macOS Memory Pressure comes from different signals [S5][S6], so a shared core needs a platform-neutral model first, whether or not code is shared.
- iOS cannot feed CPU, Memory or Disk rules [S24][S27]. Its Verdict is a subset, so an iOS core would use maybe half of today's rules.
- Gateway probes hit local network privacy on iOS 14+ and macOS 15+ [S15]; Windows has no equivalent. That needs a "permission denied" Probe outcome, distinct from loss, in the shared vocabulary.
- Process Snapshot on macOS needs direct distribution, not the Mac App Store [S8].
- `crates/fixtures` and the in-crate Verdict tests (123 lines) are the natural seed for golden vectors.
- Colors are already duplicated across `render` and `panel/theme`. A token file would fix this on Windows alone.
- An all-Rust macOS build (tray-icon + objc2 + egui) maximizes reuse but gives a non-native Panel. A SwiftUI app maximizes native feel and pushes toward option (c) or an FFI core.

---

## Sources

- [S1] Apple, SimplePing README: https://developer.apple.com/library/archive/samplecode/SimplePing/Listings/README_md.html
- [S2] Apple, App Sandbox ("To distribute a macOS app through the Mac App Store, you must enable the App Sandbox capability"): https://developer.apple.com/documentation/security/app-sandbox
- [S3] Apple, `host_statistics64` (macOS 10.6+): https://developer.apple.com/documentation/kernel/1502863-host_statistics64
- [S4] Apple Developer Forums, "Obtaining CPU usage by process": https://developer.apple.com/forums/thread/655349
- [S5] Apple, `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`: https://developer.apple.com/documentation/dispatch/dispatch_source_type_memorypressure
- [S6] XNU source, `kern_memorystatus.c` (`SYSCTL_INT(_kern, OID_AUTO, memorystatus_level, CTLFLAG_RD …)`): https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_memorystatus.c
- [S7] Apple Developer Forums, "How to retrieve the memory pressure information from macOS": https://developer.apple.com/forums/thread/802656
- [S8] Apple Developer Forums, "Getting process info for other processes?": https://developer.apple.com/forums/thread/769021
- [S9] Apple Developer Forums, "How to avoid sandbox violation for process-info-listpids": https://developer.apple.com/forums/thread/691857
- [S10] XNU source, `bsd/net/if.h` (`struct if_msghdr2` with `if_data64`): https://github.com/apple-oss-distributions/xnu/blob/main/bsd/net/if.h
- [S11] XNU source, `bsd/net/if_var.h` (`u_int32_t ifi_ibytes` in `if_data`, `u_int64_t` in `if_data64`): https://github.com/apple-oss-distributions/xnu/blob/main/bsd/net/if_var.h
- [S12] IOStorageFamily, `IOBlockStorageDriver.h` (`kIOBlockStorageDriverStatisticsKey`): https://github.com/apple-oss-distributions/IOStorageFamily/blob/main/IOBlockStorageDriver.h
- [S13] Apple, Configuring the macOS App Sandbox (`com.apple.security.network.client`): https://developer.apple.com/documentation/xcode/configuring-the-macos-app-sandbox
- [S14] dtisch, "How I implemented ping and traceroute on iOS without entitlements" (blog): https://dev.to/dtisch/how-i-implemented-ping-and-traceroute-on-ios-without-entitlements-2fm
- [S15] Apple, TN3179: Understanding local network privacy: https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy
- [S16] Apple, `NSStatusItem`: https://developer.apple.com/documentation/appkit/nsstatusitem
- [S17] Mac App Store, iStat Menus 7: https://apps.apple.com/app/id6499559693
- [S18] MacRumors forum, iStat Menus 7.0 thread: https://forums.macrumors.com/threads/istat-menus-7-0-brings-comprehensive-redesign-and-new-features.2432615/page-5
- [S19] exelban/stats README: https://github.com/exelban/stats
- [S20] crates.io, `tray-icon`: https://crates.io/crates/tray-icon
- [S21] crates.io, `objc2-app-kit`: https://crates.io/crates/objc2-app-kit
- [S22] crates.io, `sysinfo`: https://crates.io/crates/sysinfo
- [S23] crates.io, `libproc`: https://crates.io/crates/libproc
- [S24] Apple Developer Forums, "how to overall cpu utilization of iphone device": https://developer.apple.com/forums/thread/11393
- [S25] Apple Developer Forums, "Programatically get free and used RAM in ios": https://developer.apple.com/forums/thread/723334
- [S26] Apple Developer Forums, "Right way to call host_processors": https://developer.apple.com/forums/thread/748948
- [S27] Apple Developer Forums, "iOS / Background network monitoring": https://developer.apple.com/forums/thread/107042
- [S28] Apple, `NWPathMonitor` (iOS 12+, macOS 10.14+): https://developer.apple.com/documentation/network/nwpathmonitor
- [S29] Apple, Describing use of required reason API: https://developer.apple.com/documentation/bundleresources/describing-use-of-required-reason-api and `NSPrivacyAccessedAPIType`: https://developer.apple.com/documentation/bundleresources/app-privacy-configuration/nsprivacyaccessedapitypes/nsprivacyaccessedapitype
- [S30] Apple, `BGAppRefreshTask`: https://developer.apple.com/documentation/backgroundtasks/bgapprefreshtask
- [S31] Apple, App Review Guidelines: https://developer.apple.com/app-store/review/guidelines/
- [S32] Apple, Keeping a widget up to date: https://developer.apple.com/documentation/widgetkit/keeping-a-widget-up-to-date
- [S33] Apple, Displaying live data with Live Activities: https://developer.apple.com/documentation/activitykit/displaying-live-data-with-live-activities
- [S34] crates.io API, `uniffi`, `swift-bridge`, `cbindgen`, `cargo-swift`: https://crates.io/api/v1/crates/uniffi (same pattern for the others)
- [S35] UniFFI, Swift bindings overview: https://mozilla.github.io/uniffi-rs/latest/swift/overview.html
- [S36] mozilla/uniffi-rs README: https://github.com/mozilla/uniffi-rs
- [S37] mozilla/uniffi-rs CHANGELOG: https://github.com/mozilla/uniffi-rs/blob/main/CHANGELOG.md
- [S38] chinedufn/swift-bridge README: https://github.com/chinedufn/swift-bridge
- [S39] signalapp/libsignal, `swift/README.md`: https://github.com/signalapp/libsignal/blob/main/swift/README.md
- [S40] antoniusnaumann/cargo-swift: https://github.com/antoniusnaumann/cargo-swift
- [S41] Apple, PackageDescription `binaryTarget(name:url:checksum:)`: https://developer.apple.com/documentation/packagedescription/target/binarytarget(name:url:checksum:)
- [S42] Stadia Maps, "Ferrostar: building a cross-platform navigation SDK in Rust, part 2": https://stadiamaps.com/news/ferrostar-building-a-cross-platform-navigation-sdk-in-rust-part-2/
- [S43] matrix-org/matrix-rust-sdk, bindings README: https://github.com/matrix-org/matrix-rust-sdk/tree/main/bindings
- [S44] element-hq/element-x-ios `project.yml` (MatrixRustSDK package, `exactVersion: 26.11.17`): https://github.com/element-hq/element-x-ios/blob/develop/project.yml
- [S45] mozilla/rust-components-swift README: https://github.com/mozilla/rust-components-swift
- [S46] bitwarden/sdk-internal README: https://github.com/bitwarden/sdk-internal
- [S47] bitwarden/sdk-swift `Package.swift`: https://github.com/bitwarden/sdk-swift/blob/main/Package.swift
- [S48] 1Password, "1Password 8: The Story So Far": https://1password.com/blog/1password-8-the-story-so-far
- [S49] 9to5Mac, "1Password 8 beta for iPhone and iPad" (2022): https://9to5mac.com/2022/04/13/1password-8-beta-for-iphone-and-ipad/
- [S50] 1Password, Typeshare docs: https://1password.github.io/typeshare/print.html
- [S51] LY Corporation Tech Blog (Rust on iOS size): https://techblog.lycorp.co.jp/en/20241002a
- [S52] bitdrift, "Optimizing Rust mobile SDK binary size": https://blog.bitdrift.io/post/optimizing-rust-mobile-sdk-binary-size
- [S53] Nutrient, iOS framework size FAQ: https://www.nutrient.io/guides/ios/faq/framework-size.md
- [S54] JSON Schema Test Suite README: https://github.com/json-schema-org/JSON-Schema-Test-Suite
- [S55] web-platform-tests, `url/README.md`: https://github.com/web-platform-tests/wpt/blob/master/url/README.md
- [S56] tc39/test262 README: https://github.com/tc39/test262
- [S57] W3C DTCG, "Design Tokens specification reaches first stable version": https://w3.org/community/design-tokens/2025/10/28/design-tokens-specification-reaches-first-stable-version/
- [S58] Design Tokens Format Module 2025.10: https://www.w3.org/community/reports/design-tokens/CG-FINAL-format-20251028/
- [S59] Style Dictionary README: https://github.com/style-dictionary/style-dictionary
