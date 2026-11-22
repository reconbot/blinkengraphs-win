//! Graph ranges: each draws closed points from exactly one source, Raw or one Archive, plus a
//! provisional edge (spec `rules/history.md`, Graph ranges and Drawing).

use crate::archive::{Archive, Row, written};
use crate::pause::{Pause, paused_s};
use model::Aggregate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    Min1,
    Min15,
    Hour,
    Day,
    Week,
    All,
}

/// Where a range's points come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Raw,
    A1,
    A2,
    A3,
}

impl Range {
    pub const ALL: [Range; 6] = [
        Range::Min1,
        Range::Min15,
        Range::Hour,
        Range::Day,
        Range::Week,
        Range::All,
    ];

    /// The fixed span; All runs from the oldest A3 Row to now.
    pub fn span_s(self) -> Option<u64> {
        Some(match self {
            Range::Min1 => 60,
            Range::Min15 => 900,
            Range::Hour => 3600,
            Range::Day => 86_400,
            Range::Week => 604_800,
            Range::All => return None,
        })
    }

    pub fn source(self) -> Source {
        match self {
            Range::Min1 | Range::Min15 | Range::Hour => Source::Raw,
            Range::Day => Source::A1,
            Range::Week => Source::A2,
            Range::All => Source::A3,
        }
    }

    /// The point width; All's is this times `k`.
    pub fn width_s(self) -> u64 {
        match self {
            Range::Min1 => 1,
            Range::Min15 => 3,
            Range::Hour => 15,
            Range::Day => 300,
            Range::Week => 3600,
            Range::All => 86_400,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Value,
    /// Every result lost: a loss mark, not a line point.
    Lost,
    /// More than half the bucket not measured.
    Paused,
}

/// One graph bucket `[at, at + w)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphPoint {
    pub at: u64,
    pub w: u64,
    pub state: State,
    /// The aggregate; only for `State::Value`.
    pub value: Option<f64>,
    /// Lost share; absent when Paused.
    pub loss: Option<f64>,
    /// Not finished yet, so it can still change.
    pub provisional: bool,
}

/// What a graph of one key can draw from: Raw `(at, value)` in arrival order, the Pauses, and
/// the key's A1, A2 and A3.
pub struct Sources<'a> {
    pub raw: &'a [(u64, Option<f32>)],
    pub pauses: &'a [Pause],
    pub archives: [&'a Archive; 3],
    /// `⌈max_timeout_ms / 1000⌉ + 1`.
    pub g: u64,
}

/// The All range's points span `k` days, so it draws at most 240.
pub fn all_k(oldest_a3: u64, now: u64) -> u64 {
    let rows = (now / 86_400).saturating_sub(oldest_a3 / 86_400) + 1;
    rows.div_ceil(240).max(1)
}

/// The points of `range` at `now`, ascending.
pub fn graph(range: Range, now: u64, src: &Sources, agg: Aggregate) -> Vec<GraphPoint> {
    let (w, from) = match range.span_s() {
        Some(span) => (range.width_s(), now.saturating_sub(span)),
        None => {
            let Some(oldest) = src.archives[2].rows.first().map(|r| r.0) else {
                return vec![];
            };
            (86_400 * all_k(oldest, now), oldest)
        }
    };
    let level = match range.source() {
        Source::Raw => 0,
        Source::A1 => 1,
        Source::A2 => 2,
        Source::A3 => 3,
    };
    let mut b = (from + 1).saturating_sub(w).div_ceil(w) * w;
    let mut out = Vec::new();
    while b <= now {
        let (row, paused) = if level == 0 {
            raw_bucket(src, b, w)
        } else {
            archive_bucket(src, level, b, w, now)
        };
        let provisional = now < b + w + src.g;
        let point = |state, value, loss| GraphPoint {
            at: b,
            w,
            state,
            value,
            loss,
            provisional,
        };
        if paused > w as f64 / 2.0 {
            out.push(point(State::Paused, None, None));
        } else if row.n + row.lost > 0 {
            let loss = Some(row.lost as f64 / (row.n + row.lost) as f64);
            if row.n == 0 {
                out.push(point(State::Lost, None, loss));
            } else {
                let value = if level == 0 {
                    exact(&raw_values(src, b, w), agg)
                } else {
                    row.value(agg)
                };
                out.push(point(State::Value, value, loss));
            }
        }
        b += w;
    }
    out
}

/// Raw values with `b ≤ at < b + w`, ascending by `at`, then in arrival order.
fn raw_values(src: &Sources, b: u64, w: u64) -> Vec<Option<f32>> {
    let mut v: Vec<(u64, Option<f32>)> = src
        .raw
        .iter()
        .copied()
        .filter(|(at, _)| (b..b + w).contains(at))
        .collect();
    v.sort_by_key(|r| r.0);
    v.into_iter().map(|r| r.1).collect()
}

fn raw_bucket(src: &Sources, b: u64, w: u64) -> (Row, f64) {
    (
        Row::of(&raw_values(src, b, w)),
        paused_s(src.pauses, b as f64, (b + w) as f64),
    )
}

/// The merge of a source's steps in `[b, b + w)`. A step not yet written is built the way its
/// Row will be, from the Archive below and, at the bottom, Raw.
fn archive_bucket(src: &Sources, level: usize, b: u64, w: u64, now: u64) -> (Row, f64) {
    let a = src.archives[level - 1];
    let mut row = Row::default();
    let mut paused = 0.0;
    let mut m = b;
    while m < b + w {
        let (r, p) = if written(m, a.step, now, src.g) {
            (a.row(m).cloned().unwrap_or_default(), a.paused_at(m))
        } else if level == 1 {
            raw_bucket(src, m, a.step)
        } else {
            archive_bucket(src, level - 1, m, a.step, now)
        };
        row.merge(&r);
        paused += p;
        m += a.step;
    }
    (row, paused)
}

/// Raw values in one bucket, aggregated exactly with the same rank rule as the sketch.
pub(crate) fn exact(values: &[Option<f32>], agg: Aggregate) -> Option<f64> {
    let mut v: Vec<f64> = values.iter().flatten().map(|x| *x as f64).collect();
    if v.is_empty() {
        return None;
    }
    let per_mille = match agg {
        Aggregate::Avg => return Some(v.iter().sum::<f64>() / v.len() as f64),
        Aggregate::Max => return v.into_iter().reduce(f64::max),
        Aggregate::P50 => 500,
        Aggregate::P95 => 950,
        Aggregate::P99 => 990,
    };
    v.sort_by(f64::total_cmp);
    Some(v[(per_mille * (v.len() as u64 - 1) / 1000) as usize])
}
