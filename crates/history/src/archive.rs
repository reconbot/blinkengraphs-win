//! Archives: one Row per key and fixed step, written once its step can get no more results
//! (spec `rules/history.md`, Archives).

use crate::Sketch;
use crate::pause::{Pause, paused_s};
use model::Aggregate;
use std::collections::BTreeMap;

/// One step of one key, or a merge of several (spec `rules/graph-aggregate.md`, A Row).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub n: u64,
    pub lost: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub sketch: Sketch,
}

impl Row {
    /// `values` in arrival order, so `sum` is reproducible.
    pub fn of(values: &[Option<f32>]) -> Self {
        let mut m = Row {
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            ..Row::default()
        };
        for v in values {
            match v {
                Some(v) => {
                    let v = *v as f64;
                    m.n += 1;
                    m.sum += v;
                    m.min = m.min.min(v);
                    m.max = m.max.max(v);
                    m.sketch.add(v);
                }
                None => m.lost += 1,
            }
        }
        m
    }

    /// Merge Rows in ascending step order; f64 addition is not associative.
    pub fn merge(&mut self, o: &Row) {
        if o.n == 0 && o.lost == 0 {
            return;
        }
        if self.n == 0 && self.lost == 0 {
            *self = o.clone();
            return;
        }
        self.n += o.n;
        self.lost += o.lost;
        self.sum += o.sum;
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        self.sketch.merge(&o.sketch);
    }

    pub fn value(&self, agg: Aggregate) -> Option<f64> {
        if self.n == 0 {
            return None;
        }
        match agg {
            Aggregate::Avg => Some(self.sum / self.n as f64),
            Aggregate::Max => Some(self.max),
            Aggregate::P50 => self.sketch.quantile(500, self.min, self.max),
            Aggregate::P95 => self.sketch.quantile(950, self.min, self.max),
            Aggregate::P99 => self.sketch.quantile(990, self.min, self.max),
        }
    }

    pub fn loss(&self) -> f32 {
        let total = self.n + self.lost;
        if total == 0 {
            0.0
        } else {
            self.lost as f32 / total as f32
        }
    }
}

/// One Archive's Rows for one key, and the Paused seconds of its steps, both ascending by step.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Archive {
    pub step: u64,
    pub rows: Vec<(u64, Row)>,
    pub paused: Vec<(u64, f64)>,
}

/// How long after its `at` a result can still arrive: `⌈max_timeout_ms / 1000⌉ + 1` s.
pub fn grace_s(max_timeout_ms: u64) -> u64 {
    max_timeout_ms.div_ceil(1000) + 1
}

/// Whether the step at `m` is written by `now`.
pub fn written(m: u64, step: u64, now: u64, g: u64) -> bool {
    now >= m + step + g
}

/// Whether an Archive keeping `rows` steps still holds the step at `m`.
pub fn kept(m: u64, step: u64, rows: u64, now: u64) -> bool {
    m + step > now.saturating_sub(rows * step)
}

impl Archive {
    /// A1 as written by `now`, from Raw `(at, value)` results in arrival order.
    pub fn from_raw(
        results: &[(u64, Option<f32>)],
        pauses: &[Pause],
        step: u64,
        now: u64,
        g: u64,
    ) -> Archive {
        let mut sorted = results.to_vec();
        // Stable, so equal `at`s keep arrival order, the order `sum` adds in.
        sorted.sort_by_key(|r| r.0);
        let mut steps: BTreeMap<u64, Vec<Option<f32>>> = BTreeMap::new();
        for (at, v) in sorted {
            steps.entry(at / step * step).or_default().push(v);
        }
        let rows = steps
            .into_iter()
            .filter(|(m, _)| written(*m, step, now, g))
            .map(|(m, vs)| (m, Row::of(&vs)))
            .collect();
        let mut paused: BTreeMap<u64, f64> = BTreeMap::new();
        for p in pauses {
            let mut m = p.begin.max(0.0) as u64 / step * step;
            while (m as f64) < p.end && written(m, step, now, g) {
                paused.insert(m, paused_s(pauses, m as f64, (m + step) as f64));
                m += step;
            }
        }
        Archive {
            step,
            rows,
            paused: paused.into_iter().filter(|p| p.1 > 0.0).collect(),
        }
    }

    /// The next Archive, at `step`: each of its steps merges this one's Rows in ascending
    /// order and adds their Paused seconds.
    pub fn merge(&self, step: u64) -> Archive {
        let mut rows: Vec<(u64, Row)> = Vec::new();
        for (m, r) in &self.rows {
            let start = m / step * step;
            match rows.last_mut() {
                Some((s, acc)) if *s == start => acc.merge(r),
                _ => rows.push((start, r.clone())),
            }
        }
        let mut paused: Vec<(u64, f64)> = Vec::new();
        for (m, secs) in &self.paused {
            let start = m / step * step;
            match paused.last_mut() {
                Some((s, acc)) if *s == start => *acc += secs,
                _ => paused.push((start, *secs)),
            }
        }
        Archive { step, rows, paused }
    }

    /// Drops Rows and Paused seconds with `m + step ≤ now − rows × step`.
    pub fn retain(&mut self, now: u64, rows: u64) {
        let step = self.step;
        self.rows.retain(|(m, _)| kept(*m, step, rows, now));
        self.paused.retain(|(m, _)| kept(*m, step, rows, now));
    }

    pub fn row(&self, m: u64) -> Option<&Row> {
        let i = self.rows.binary_search_by_key(&m, |r| r.0).ok()?;
        Some(&self.rows[i].1)
    }

    pub fn paused_at(&self, m: u64) -> f64 {
        self.paused
            .binary_search_by_key(&m, |p| p.0)
            .map_or(0.0, |i| self.paused[i].1)
    }
}
