# Every default comes from the merged defaults

ADR 0011 embedded `spec/defaults.json`, but only `config` read it. The Prober's timeout, DNS query name and Hop Trace limits, and History's hop window and recent window, stayed constants in their crates, held equal to the defaults by a test. So `defaults.override.json` could not change them, and the Graph and readout aggregates were hard-coded too.

- `config::defaults` decodes every key in `defaults.json`.
- `history::Limits` and `probe::Limits` are passed in when History and the Prober are built. `Config::history_limits` and `Config::probe_limits` fill them from the merged defaults plus the user's retention choices. The drift test is gone because drift can't happen.
- `History::now(probe)` is the Probe's stats over the recent window, so callers no longer pass `RECENT_SECS`.
- `Config::default()` takes the Graph aggregate (`p95`) and each Probe's readout aggregate (`p50`) and window from the defaults (issues #27, #28).
