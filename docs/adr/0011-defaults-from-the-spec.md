# Defaults come from the spec

The spec's `defaults.json` was generated from `Config::default()` here. Now the spec owns it, hand-edited, and every platform merges its own overrides over it: this repo's `defaults.override.json`, per the spec's `rules/defaults.md`.

- `config::defaults` embeds `spec/defaults.json` and `defaults.override.json` at build time and merges them. `Config::default()` takes the Probe Group, primary Probe, system interval and History lengths from the result.
- `spec/` is a git submodule on a spec release tag. It replaces `SPEC_VERSION`, so one pin both feeds the build and is what CI tests against.
- The spec authors `vectors/defaults/merge.json`, the cases every platform's merge must reproduce (ADR 0013).
- The engine's timeout, DNS query name, Hop Trace limits and recent window stay compile-time constants. A test fails if they drift from the merged defaults, but overriding them takes effect only once those crates read `config::defaults`.
- The first shared-default change rides along: Quad9 (9.9.9.9) replaces Google DNS (8.8.8.8) as the second internet Target. Verdict vectors keep 8.8.8.8 only as a label in their inputs.
