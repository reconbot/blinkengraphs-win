# Proposal: macOS and iOS versions

Evidence: [`docs/research/03-cross-platform.md`](../research/03-cross-platform.md).

## Recommendation

Use separate repositories per platform, each written natively, kept consistent by a shared spec repository rather than a shared code library.

```
blinkengraphs-spec      vocabulary, rules, test vectors, design tokens (no code)
blinkengraphs           Windows: Rust (this repo)
blinkengraphs-mac       macOS menu bar: Swift + SwiftUI/AppKit
blinkengraphs-ios       iOS network diagnosis: Swift + SwiftUI + WidgetKit
```

Each app repo pins a tagged spec version, as a git submodule or a SwiftPM package of resources, and its CI runs the spec's test vectors against its own implementation.

## Why not a shared Rust core

- **What must stay consistent is small.** The Verdict logic is about 285 lines. Meter scaling, thresholds and colors are about a dozen. Rewriting them in Swift is cheaper than owning an FFI bridge.
- **What is shareable today is shaped for Windows.** `Metric` and the Local Load rules read commit charge, hard faults and disk response time. Neither macOS nor iOS has the same counters, so the model has to be redesigned per platform whatever happens.
- **The FFI tax is permanent:**
  - UniFFI is "a long way from 1.0" and changes in breaking ways.
  - Every shipping example publishes an XCFramework as a SwiftPM binary target and releases the core and the app in lockstep: Signal, Matrix/Element X, Mozilla, Bitwarden.
  - That is a team-sized process for a code budget of about 300 lines.
- **Footprint is a feature (ADR 0007).** A Rust static library inside a Swift app adds megabytes; the reported figures run from about 1 MB to tens of MB, low confidence. A native Swift implementation adds kilobytes.
- **The look should be native.** An iStat-grade Mac app means SwiftUI/AppKit. egui would carry Windows' look onto a Mac.

## What goes in `blinkengraphs-spec`

| Path | Contents | Consumed as |
|---|---|---|
| `CONTEXT.md` | The vocabulary (Section, Metric, Probe, Verdict, Finding, Fault Domain…), moved here from this repo | Read by people and agents; each repo links to it |
| `rules/verdict.md` | Each Verdict rule in prose: conditions, thresholds, severity, headline template | The authority the code implements |
| `vectors/verdict/*.json` | Input (Probe stats, Sample, upload peak) → expected Findings (domain, severity, headline) | Table tests in Rust and Swift |
| `vectors/meters/*.json` | Raw series → normalized 0–1 values (cube-root network, latency scale, loss marks) | Table tests |
| `tokens/*.json` | Colors, warn/crit thresholds and spacing, in W3C design-tokens format | Style Dictionary generates Rust consts and Swift `Color`/`CGFloat` |
| `fixtures/scenarios/*.json` | The scenario histories from `crates/fixtures` (idle, CPU pinned, ISP loss, upload saturation) | Screenshot and preview data on every platform |

Precedent for test vectors shared across languages: the JSON Schema Test Suite, WPT `urltestdata.json`, Test262.

## Per-platform scope

- **Windows (this repo):** everything as built.
- **macOS:**
  - The full Section set from Mach `host_statistics64`, memory-pressure level, `NET_RT_IFLIST2` and IOKit.
  - Unprivileged ICMP.
  - `NSStatusItem` Meters.
  - Distributed outside the App Store, because the sandbox blocks the libproc Process Snapshot.
- **iOS:**
  - A foreground network diagnoser: latency, DNS, `NWPathMonitor` path, Hop Trace, and the network half of the Verdict.
  - No system CPU or memory.
  - Widgets refresh about every 5 minutes at most, so they show the last Verdict, not live graphs.

## Spec changes needed before a second platform

1. **Platform-neutral Metrics in the spec.** Name the concept ("memory pressure", "disk busy"). Map each platform's counters to it in that platform's repo.
2. **A `denied` Probe outcome**, distinct from loss. Local Network Privacy on macOS 15+ and iOS 14+ can block Gateway probes.
3. **Verdict input as data:** the JSON shape the vectors use. It also becomes the contract each platform's engine fills.

## Cost of this path

- The Verdict exists twice. Mitigation: vectors are the authority. A rule change is a spec PR plus one PR per platform; CI turns red until each platform passes the new vectors.
- No shared probing or history code. These are thin wrappers over each OS's APIs anyway, about 935 portable lines today, much of it Win32-shaped.

## When to reconsider

- If more than two people maintain the rules, or the rules grow past about 1,000 lines, move `verdict` into a Rust crate with a UniFFI binding and pay the lockstep cost.
- If the Mac version is meant to be a quick port rather than a native app, it can depend on this repo's crates directly: `tray-icon` + `objc2` + egui. That puts the whole engine into a single shared Rust codebase, at the cost of a non-native Panel.
