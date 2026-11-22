# History follows the spec's history rules

Superseded in part by [ADR 0018](0018-history-is-raw-plus-archives.md): the Minute, seam and RAM parts no longer hold.

Spec 2.0.0 adds `rules/history.md` and one percentile rank, so Windows and iOS collect, keep and draw data the same way. Windows changes:

- **Rounds keep a fixed beat.** `probe::next_round` puts each round on the grid `start + k × interval`. Before, the grid restarted after any late round.
- **One rank.** Every percentile is `⌊(n − 1) × X / 100⌋`. `rtt_ms` was the upper median, so it moves to the lower median for even counts.
- **RAM drops a result exactly `ram_s` old.** RAM also inserts in time order, because rounds from different groups can report out of order.
- **A Minute is written once nothing more can land in it,** at `m + 60 + g` with `g = ⌈max_timeout⌉ + 1` s, and before any eviction. A late Probe result used to miss its minute. RAM keeps at least `61 + g` s so that no minute's first results are evicted before it is written.
- **Storage hands over to RAM where written Minutes end.** `history::buckets(since, until, w, written, ram, stored, agg)` is the pure function the spec's bucket vectors test. Before, the seam minute was read from both storage and RAM.
- **An empty bucket or a loss-only bucket breaks the line.** Each `Point` carries its bucket width so the Panel can tell. A lone bucket draws as a level segment across its width.
- **RTTs are whole milliseconds,** recorded as `⌊d + 0.5⌋` by `probe::rtt_ms`. `model::rtt_text` displays them as whole milliseconds, or "< 1 ms" for 0. Disk response time is a Metric, not an RTT, and keeps its decimal.
