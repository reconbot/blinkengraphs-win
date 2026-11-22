# History is Raw plus Archives in SQLite

Spec 5.0.0 replaces Minutes and the moving seam with Raw and three fixed-step Archives (spec ADR 0001, `rules/history.md`). This supersedes the Minute, seam and RAM parts of [ADR 0017](0017-history-follows-the-spec.md); its Round timing and percentile rank still hold.

- **Raw is in memory and in SQLite.** The in-memory copy serves stats, Meters and the 1m, 15m and 1h graphs. Pending results are written in one transaction every 5 s, within the spec's 10 s bound, so a kill or hibernate loses at most 5 s. Raw also keeps every A1 step not yet written, so a restart writes the steps it missed.
- **`synchronous=NORMAL` with WAL.** A commit survives a crash or kill, and power loss costs at most the last commits (research 07 §5). FULL would sync on every 5 s write for no gain against kills.
- **Archive Rows are read from SQLite, not held in memory.** A year of A3 plus A1 and A2 for every key would cost tens of MB in sketches, against the 40 MB budget (ADR 0007). The Panel reads the Rows of the keys it draws, cached until the next step is written.
- **Each Archive has a cursor,** its first unwritten step, stored with the Rows. It skips empty stretches, so a week away costs a few queries rather than one per step.
- **An older database starts empty.** `PRAGMA user_version` below 2 drops the `minute`, `recent` and `rollup` tables. The owner chose this over converting Minutes into Rows.
- **The user's retention is All's length,** `archive_days`, 1 to 365, default 365 (A3's Rows kept). The old `rollup_days` key is ignored.
- **Hop statistics stay in memory** until Links (#40) key them per Link.
