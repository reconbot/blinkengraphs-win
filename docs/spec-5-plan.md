# Plan: adopt spec 5.0.0

Issue #40. The rules are spec `rules/history.md`, `probe-stats.md`, `graph-aggregate.md` and `verdict.md` at v5.0.0. The decision is spec ADR 0001, and the cross-platform plan is spec `docs/history-redesign.md`. This file records how Windows gets there. Win32 facts are in [research 07](research/07-pauses-and-links.md). Code references are at `2bbb58a`.

`spec/` is at v3.0.0, so this also takes 4.0.0, the Verdict tie rule (`isp-loss-tie`).

## The model

The words are the spec's (`CONTEXT.md`): Round, Pause, Raw, Archive, Row, Link. The Windows types:

| Type | Is | Replaces |
|---|---|---|
| `LinkId` | Archive identity: interface (NET_LUID) and Gateway | nothing; History has no Link today |
| `Link` | `LinkId` plus alias, kind (Ethernet, Wi-Fi, VPN), Access Point, whether it carries the default route | `collect::AdapterChoice` for the Probe side |
| `Key` | `Probe(LinkId, Probe)`, `LinkMetric(LinkId, Metric)` for NetDown, NetUp and NetErrors, `Metric(Metric)` for the rest | `Key::{Metric, Probe}` |
| `Pause` | `[begin, end)` in f64 Unix seconds, machine-wide | `Shared.paused`, which no Surface reads and History never sees |
| `Round` | `at`, wall start and end, the Link, its results | per-result `Event::Probe` / `Event::Hop` |
| `Row` | `n, lost, sum, min, max, sk` for one key and step | `Minute`, which already has these fields and merges the same way |
| `GraphPoint` | `at, w, state (value, lost, paused), value, loss, provisional` | `Point {at, secs, value, loss}` |

**A Round reaches History whole.** Today each Probe's result is emitted as soon as it returns (`probe/src/lib.rs:271-291`). A Round records nothing if any part of it falls in a Pause, and that is only known when its last Probe ends. So the Ticker emits one `Event::Round` with its start and end instants. The engine drops it when `history::cut(round, pauses)` says so. That function is pure, and `vectors/history/rounds.json` tests it.

**Pauses are the engine's.** Suspend, resume, idle rest and launch all reach the Monitor as Commands or Sensor readings. Only the Monitor opens and closes Pauses and passes them to History. That way `tick` tests can drive fake time through a suspend (ADR 0015).

## The `history` crate

`History::series`, `record_*` and `now` keep their names (#40). They gain a Link where they need one. Inside, `lib.rs` splits along the spec's sections, each a pure core tested by its vectors with a thin SQLite shell:

- **`raw`**: results, Samples, Hops, Pauses and Link changes. They are kept with `at > now − raw_s` (`raw.json`). There is a copy in memory and a SQLite journal behind it, written in one transaction at most 10 s after each result and when a Pause begins. Launch reloads Raw, then opens the kill Pause from the newest `at` + 1 s to launch + settle (`raw.json` `kill`). This replaces `ram`, the `recent` table, `close` and `restore`.
- **`archive`**: `rows(raw, pauses, step, now, g)` builds A1 Rows and Paused seconds (`rows.json`). `merge` builds A2 from A1 and A3 from A2 (`archives.json`). It also handles retention. `Minute`'s merge and sketch code moves here unchanged. Rows go in one SQLite table keyed `(archive, key, m)`, plus `paused(archive, m, secs)`.
- **`range`**: `graph(range, now, source, pauses, agg)` returns closed points and the provisional edge (`ranges.json`, `graph-raw.json`, `graph-archive.json`, `graph-all.json`). This replaces `buckets`, `bucket_width` and the seam.
- **`stats`**: per-Probe stats with the window from `now` and the baseline over Raw. The result is `Option<ProbeStat>`, because an empty window has no stats (`probe-stats/*`, including `after-pause`, `resumed` and `raw-edge`). `aggregate` (readouts) uses the same window.

The pure functions take slices, so the conformance runner never touches SQLite. That matches the iOS split.

**Memory.** Two hours of Raw for 14 Metrics plus about 10 Probes is about 175k entries a Link. At 16 bytes each that is about 3 MB, and Hops add about as much again. That fits the 40 MB working set (ADR 0007). It needs measuring with two Links. Reading 1h graphs from SQLite on every Panel frame would cost more than keeping Raw in memory.

## Lifecycle

- **Suspend and resume:** `RegisterSuspendResumeNotification` on the hidden session window (`app/src/win.rs:703`). `PBT_APMSUSPEND` begins a Pause and flushes Raw. `PBT_APMRESUMEAUTOMATIC` ends it after the settle time. The fallback for Modern Standby, where delivery is uncertain, is in research 07.
- **Idle rest:** `idle()` (`win.rs:656-680`) already pauses the Prober. It now also opens and closes a Pause. `pause_when_idle` becomes the spec's Setting, default `true` as today, with an ADR. Lock detection could move from polling `OpenInputDesktop` to `WTSRegisterSessionNotification`. That change is optional and not needed for #40.
- **Settle time:** start at 2 s, the value the spec vectors use. Then set it from logged first-reply times after resume on Ethernet, Wi-Fi and a VPN, and record it in an ADR. The app logs `resume → first reply` per Link so the ADR has data.
- **Shutdown:** `close` becomes "flush Raw". Hibernate and kills need nothing more.
- **Verdict:** `FaultDomain::Paused` and `Input::since_pause_s`. `diagnose` passes only the default-route Link's Probes that have stats.

## Links

- **Enumerate:** take the adapters that are up and have a default route or a Gateway, and key each by NET_LUID and Gateway. React to `NotifyIpInterfaceChange` / `NotifyRouteChange2` instead of the 30 s resolver poll.
- **Send per Link:** `Transport` methods take the Link. TCP, UDP and DNS use `IP_UNICAST_IF` / `IPV6_UNICAST_IF`. ICMP uses `IcmpSendEcho2Ex` / `Icmp6SendEcho2` with the Link's source address. The scripted Transport in `probe/tests/rules.rs` covers the rules on Linux.
- **Routed Targets only:** a Probe is sent on a Link only if `GetBestRoute2`, constrained to that interface, routes its Target. Otherwise it records nothing, as for an unreachable IP version.
- **Per Link:** History keys, stats, baselines, Hop tracks, `probe_views` and palette overrides (both keyed by the Probe's display string today).
- **Panel:** one line per Link ("1.1.1.1 · Wi-Fi"), a Link picker, and Link-change markers on Raw ranges.
- **Followed Adapter:** stays the Network Section's adapter. Throughput is recorded per Link adapter, and the Section draws the Followed one.

## Surfaces

- **Meters** draw by time from Raw (`meters/src/lib.rs:25-64`): one column per second, so a Pause shows as Paused instead of joining old and new results.
- **Panel graphs** draw `GraphPoint.state`: Paused shading, a lost mark, and a provisional marker. `Range::All` comes from `range`, not from `now − oldest()`.
- **Settings:** "Keep history for N days" becomes A3's Rows kept, 1 to 365, default 365. The size estimate counts Rows, and CSV export writes Rows.

## Pull requests

Each one merges when CI is green. The `spec/` bump has to land with runners for every v5 vector, so the first PR carries the pure cores.

1. **Spec 5.0.0, pure cores.** Bump `spec/` and update `HistoryDefaults` (`raw_minutes`, `archives`; today's struct panics on v5). Add `raw`, `archive`, `range`, `stats` and `cut` as pure functions, with runners for every new vector. Drop the `minutes` and `buckets` runners. Add the Verdict `Paused` rule, `since_pause_s` and the 4.0.0 tie rule. The app keeps running on the old storage path.
2. **History on Raw and Archives.** Add the SQLite shell for Raw and Archives, the facade, the migration from the `minute` table, and the Panel states. Remove `ram`, `recent`, `Minute` storage and the seam. Supersede the Minute and seam parts of ADR 0017.
3. **Pauses.** Add suspend and resume, idle rest as a Pause, `Event::Round` and cut Rounds, the kill Pause on launch, the resuming Verdict, and the meters by time. Write ADRs for the settle time, the Raw write cadence and the idle-rest default.
4. **Links.** Enumerate Links, send per interface, key per Link, add the Panel picker and markers. Write an ADR for the Link identity.

## Open questions

Need the owner:
- **Existing history:** convert the `minute` table into A1, A2 and A3 (exact: Minutes merge into Rows), or start empty?
- **Slicing:** four PRs as above, or one?

Need a spec PR first:
- **Link identity:** "interface and Gateway" leaves a dual-stack Link (a v4 and a v6 Gateway) and a VPN with no Gateway (on-link default route, as on Wintun) undefined.
- **Followed Adapter versus default route:** with a pinned adapter, the Network Section and the Verdict speak for different Links. Is that intended?
- **`graph-raw.json` case 2:** its name says a bucket 8 s Paused draws its data. The bucket is 15 s, 8 > 7.5, and `expected` is `paused`, which follows the rule. The name is wrong, and no Raw vector covers a bucket Paused for half or less of its width that still draws.
