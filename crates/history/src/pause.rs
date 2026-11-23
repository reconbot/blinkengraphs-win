//! Pauses: time not measured (spec `rules/history.md`, Pauses).

/// `[begin, end)` in Unix seconds. Machine-wide: every key shares it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pause {
    pub begin: f64,
    pub end: f64,
}

impl Pause {
    /// The Pause a launch finds: from the second after the newest record of any kind in Raw
    /// (a result, a Sample, a mark or a Link change), or from the launch when Raw is empty, to
    /// the settle time after launch.
    pub fn at_launch(newest_record: Option<u64>, launch: f64, settle_s: f64) -> Pause {
        Pause {
            begin: newest_record.map_or(launch, |at| at as f64 + 1.0),
            end: launch + settle_s,
        }
    }
}

/// What a Link's appearances and changes at `changes` cut of its own Rounds: the settle time
/// after each. Not Pauses: they belong to one Link and nothing is Paused in History (spec
/// `rules/history.md`, Link settle).
pub fn link_settle(changes: &[f64], settle_s: f64) -> Vec<Pause> {
    changes
        .iter()
        .map(|&c| Pause {
            begin: c,
            end: c + settle_s,
        })
        .collect()
}

/// Whether a Round from `start` to the end of its last Probe falls in any Pause, so it records
/// nothing. One ending exactly as a Pause begins still records.
pub fn cut(start: f64, end: f64, pauses: &[Pause]) -> bool {
    pauses.iter().any(|p| start < p.end && end > p.begin)
}

/// Seconds of `[from, to)` in Pauses, which don't overlap.
pub fn paused_s(pauses: &[Pause], from: f64, to: f64) -> f64 {
    pauses
        .iter()
        .map(|p| (p.end.min(to) - p.begin.max(from)).max(0.0))
        .sum()
}

/// Whether Raw still keeps a result at `at`.
pub fn raw_keeps(at: u64, now: u64, raw_s: u64) -> bool {
    at + raw_s > now
}
