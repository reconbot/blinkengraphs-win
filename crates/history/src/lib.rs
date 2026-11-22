pub mod archive;
pub mod pause;
pub mod range;
mod sketch;
mod store;
pub use archive::Row;
pub use range::{GraphPoint, Range, State};
pub use sketch::{Sketch, index as sketch_index, rep as sketch_rep};

use archive::{Archive, written};
use model::{Aggregate, HopResult, Metric, Probe, ProbeResult, ProbeStat, Sample};
use pause::{Pause, paused_s};
use range::Sources;
use rusqlite::Connection;
use std::cell::RefCell;
use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    path::Path,
};
use store::Store;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Metric(Metric),
    Probe(Probe),
}

impl Key {
    fn id(&self) -> String {
        match self {
            Key::Metric(m) => format!("metric:{m:?}"),
            Key::Probe(p) => format!("probe:{p}"),
        }
    }

    fn from_id(id: &str) -> Option<Key> {
        if let Some(m) = id.strip_prefix("metric:") {
            return Metric::ALL
                .into_iter()
                .find(|x| format!("{x:?}") == m)
                .map(Key::Metric);
        }
        let (protocol, target) = id.strip_prefix("probe:")?.split_once(' ')?;
        Some(Key::Probe(Probe {
            protocol: model::Protocol::try_from(protocol.to_string()).ok()?,
            target: model::Target::from(target.to_string()),
        }))
    }
}

/// One row of an mtr-style table.
#[derive(Clone, Debug, PartialEq)]
pub struct Hop {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub sent: u32,
    pub loss: f32,
    pub last_ms: Option<f32>,
    pub best_ms: Option<f32>,
    pub avg_ms: Option<f32>,
    pub worst_ms: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    /// Absent when nothing replied.
    pub value: Option<f32>,
    pub loss: f32,
}

/// How much History keeps and what "now" means. Callers fill it from the merged defaults
/// (spec `defaults.json` `history` and `hop_trace`), plus the user's retention choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// How far back Raw keeps results.
    pub raw_s: u64,
    /// A1, A2 and A3: `(step_s, rows kept)`. A3's Rows kept are the user's days.
    pub archives: [(u64, u64); 3],
    /// Replies kept per Hop.
    pub hop_window: usize,
    /// The window "now" means for loss, RTT and the Verdict.
    pub recent_window_s: u64,
    /// How long after its `at` a result can still arrive (`archive::grace_s`).
    pub grace_s: u64,
}

#[derive(Default)]
struct HopTrack {
    addr: Option<IpAddr>,
    rtts: VecDeque<Option<f32>>,
    reached: bool,
}

/// Rows of one key and Archive read from the store, kept while no step is written.
struct Cached {
    generation: u64,
    from: u64,
    archive: Archive,
}

/// Raw reaches disk this often, inside the spec's 10 s bound (spec `rules/history.md`, Raw).
const RAW_WRITE_S: u64 = 5;

pub struct History {
    limits: Limits,
    raw: HashMap<Key, VecDeque<(u64, Option<f32>)>>,
    pauses: Vec<Pause>,
    hops: HashMap<Probe, Vec<HopTrack>>,
    last_sample: Option<Sample>,
    store: Store,
    unwritten: Vec<(String, u64, Option<f32>)>,
    unwritten_pauses: Vec<Pause>,
    raw_written_at: u64,
    /// The newest `now` seen.
    clock: u64,
    /// Per Archive, the first step not yet written; `None` until the first result.
    cursors: [Option<u64>; 3],
    /// Bumped whenever a step is written, so cached Rows are reread.
    generation: u64,
    cache: RefCell<HashMap<(usize, Key), Cached>>,
    storage: Option<Storage>,
}

/// What the history database holds now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Storage {
    pub bytes: u64,
    pub rows: u64,
}

/// Bytes per stored Row when the database is too young to measure (research 06, synthetic data).
const BYTES_PER_ROW: f64 = 94.0;

/// Disk needed to keep `days` of All for `series` keys, from the measured Row size once there is
/// enough data. Each key holds A1 and A2 in full and one A3 Row a day.
pub fn estimate_bytes(series: usize, limits: &Limits, days: u32, measured: Option<Storage>) -> u64 {
    let per_row = measured
        .filter(|s| s.rows >= 1000)
        .map_or(BYTES_PER_ROW, |s| s.bytes as f64 / s.rows as f64);
    let rows = limits.archives[0].1 + limits.archives[1].1 + days as u64;
    (series as f64 * rows as f64 * per_row) as u64
}

impl History {
    /// `db: None` keeps everything in memory (tests, or a disk that refuses writes).
    pub fn open(db: Option<&Path>, limits: Limits) -> Self {
        let store = Store::open(db);
        let now = model::now();
        let cursors = [0, 1, 2].map(|i| store.cursor(i));
        // Raw for steps not yet written is reloaded too, so a restart writes them.
        let since = now.saturating_sub(limits.raw_s);
        let since = cursors[0].map_or(since, |c| since.min(c.saturating_sub(1)));
        let mut h = Self {
            limits,
            raw: HashMap::new(),
            pauses: store.load_pauses(since as f64),
            hops: HashMap::new(),
            last_sample: None,
            unwritten: Vec::new(),
            unwritten_pauses: Vec::new(),
            raw_written_at: 0,
            clock: 0,
            cursors,
            generation: 0,
            cache: RefCell::new(HashMap::new()),
            storage: None,
            store,
        };
        for (id, at, v) in h.store.load_raw(since) {
            if let Some(key) = Key::from_id(&id) {
                h.push(key, at, v);
            }
        }
        h.storage = h.measure();
        h
    }

    pub fn record_sample(&mut self, s: &Sample) {
        self.maintain(s.at);
        for m in Metric::ALL {
            self.record(Key::Metric(m), s.at, Some(s.get(m) as f32));
        }
        self.last_sample = Some(s.clone());
    }

    pub fn record_probe(&mut self, r: &ProbeResult) {
        self.maintain(r.at);
        self.record(Key::Probe(r.probe.clone()), r.at, r.rtt_ms);
    }

    pub fn record_hop(&mut self, h: &HopResult) {
        let track = self.hops.entry(h.trace.clone()).or_default();
        let i = h.ttl.saturating_sub(1) as usize;
        if track.len() <= i {
            track.resize_with(i + 1, HopTrack::default);
        }
        let t = &mut track[i];
        if h.addr.is_some() {
            t.addr = h.addr;
        }
        t.reached = h.reached;
        t.rtts.push_back(h.rtt_ms);
        if t.rtts.len() > self.limits.hop_window {
            t.rtts.pop_front();
        }
        if h.reached {
            track.truncate(i + 1);
        }
    }

    pub fn probes(&self) -> Vec<Probe> {
        self.raw
            .keys()
            .filter_map(|k| match k {
                Key::Probe(p) => Some(p.clone()),
                Key::Metric(_) => None,
            })
            .collect()
    }

    pub fn last_sample(&self) -> Option<&Sample> {
        self.last_sample.as_ref()
    }

    /// Newest last; `None` marks a loss. Pads nothing: may return fewer than `n`.
    pub fn recent(&self, key: &Key, n: usize) -> Vec<Option<f32>> {
        self.raw
            .get(key)
            .map(|q| {
                q.iter()
                    .skip(q.len().saturating_sub(n))
                    .map(|(_, v)| *v)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The Probe's stats over the recent window (`Limits::recent_window_s`) back from `now`;
    /// `None` while that window is empty.
    pub fn now(&self, probe: &Probe, now: u64) -> Option<ProbeStat> {
        let q = self.raw.get(&Key::Probe(probe.clone()))?;
        let results: Vec<_> = q.iter().copied().collect();
        probe_stats(
            &results,
            now,
            self.limits.recent_window_s,
            self.limits.raw_s,
        )
    }

    /// One value for `probe` over the `window_secs` back from `now`, with the share of Probes lost.
    pub fn aggregate(&self, probe: &Probe, now: u64, window_secs: u64, agg: Aggregate) -> Summary {
        let Some(q) = self.raw.get(&Key::Probe(probe.clone())) else {
            return Summary::default();
        };
        let window: Vec<Option<f32>> = q
            .iter()
            .filter(|(t, _)| *t + window_secs > now)
            .map(|(_, v)| *v)
            .collect();
        let replies: Vec<f32> = window.iter().flatten().copied().collect();
        Summary {
            value: match agg {
                Aggregate::P50 => percentile(replies, 50),
                Aggregate::P95 => percentile(replies, 95),
                Aggregate::P99 => percentile(replies, 99),
                Aggregate::Avg => (!replies.is_empty())
                    .then(|| replies.iter().sum::<f32>() / replies.len() as f32),
                Aggregate::Max => replies.into_iter().reduce(f32::max),
            },
            loss: if window.is_empty() {
                0.0
            } else {
                1.0 - window.iter().flatten().count() as f32 / window.len() as f32
            },
        }
    }

    /// The Probe stats window, `Limits::recent_window_s`.
    pub fn window_s(&self) -> u64 {
        self.limits.recent_window_s
    }

    pub fn hops(&self, trace: &Probe) -> Vec<Hop> {
        let Some(track) = self.hops.get(trace) else {
            return vec![];
        };
        track
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let replies: Vec<f32> = t.rtts.iter().flatten().copied().collect();
                let sent = t.rtts.len() as u32;
                Hop {
                    ttl: i as u8 + 1,
                    addr: t.addr,
                    sent,
                    loss: if sent == 0 {
                        0.0
                    } else {
                        1.0 - replies.len() as f32 / sent as f32
                    },
                    last_ms: t.rtts.back().copied().flatten(),
                    best_ms: replies.iter().copied().reduce(f32::min),
                    avg_ms: (!replies.is_empty())
                        .then(|| replies.iter().sum::<f32>() / replies.len() as f32),
                    worst_ms: replies.iter().copied().reduce(f32::max),
                }
            })
            .collect()
    }

    /// The points of `range` for `key` at `now`, from the range's one source (spec
    /// `rules/history.md`, Graph ranges).
    pub fn series(&self, key: &Key, range: Range, now: u64, agg: Aggregate) -> Vec<GraphPoint> {
        // Graph no step the writer hasn't reached, so a step never reads as written but empty.
        let now = if self.clock > 0 {
            now.min(self.clock)
        } else {
            now
        };
        let level = match range.source() {
            range::Source::Raw => 0,
            range::Source::A1 => 1,
            range::Source::A2 => 2,
            range::Source::A3 => 3,
        };
        let empty = VecDeque::new();
        let raw: Vec<(u64, Option<f32>)> = self
            .raw
            .get(key)
            .unwrap_or(&empty)
            .iter()
            .copied()
            .collect();
        let mut archives: [Archive; 3] = Default::default();
        for (i, a) in archives.iter_mut().enumerate() {
            a.step = self.limits.archives[i].0;
        }
        if level > 0 {
            let from = match range.span_s() {
                Some(span) => now.saturating_sub(span),
                None => 0,
            };
            archives[level - 1] = self.archive(level - 1, key, from);
            // Steps the source hasn't written are built from the Archives below.
            for l in (0..level - 1).rev() {
                let from = self.cursors[l + 1]
                    .unwrap_or(0)
                    .min(now.saturating_sub(self.limits.archives[l + 1].0));
                archives[l] = self.archive(l, key, from);
            }
        }
        let src = Sources {
            raw: &raw,
            pauses: &self.pauses,
            archives: [&archives[0], &archives[1], &archives[2]],
            g: self.limits.grace_s,
        };
        range::graph(range, now, &src, agg)
    }

    /// Rows of `key` in Archive `level` from `from` on, from the cache or the store.
    fn archive(&self, level: usize, key: &Key, from: u64) -> Archive {
        let mut cache = self.cache.borrow_mut();
        if let Some(c) = cache.get(&(level, key.clone()))
            && c.generation == self.generation
            && c.from <= from
        {
            return c.archive.clone();
        }
        let archive = Archive {
            step: self.limits.archives[level].0,
            rows: self.store.rows(level, &key.id(), from),
            paused: self.store.paused(level, from),
        };
        cache.insert(
            (level, key.clone()),
            Cached {
                generation: self.generation,
                from,
                archive: archive.clone(),
            },
        );
        archive
    }

    /// Writes what is due and saves Raw, as a shutdown must.
    pub fn close(&mut self, now: u64) {
        self.maintain(now);
        self.write_raw();
    }

    /// Measured whenever an A1 step is written; `None` without a database.
    pub fn storage(&self) -> Option<Storage> {
        self.storage
    }

    /// Unix seconds of the oldest data held: the oldest A3 Row, else Raw.
    pub fn oldest(&self) -> Option<u64> {
        let raw = self
            .raw
            .values()
            .filter_map(|q| q.front().map(|(t, _)| *t))
            .min();
        match (self.store.oldest(2), raw) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The user's All retention: A3's Rows kept.
    pub fn set_archive_days(&mut self, days: u32) {
        self.limits.archives[2].1 = days.max(1) as u64;
    }

    fn record(&mut self, key: Key, at: u64, v: Option<f32>) {
        if self.store.persistent() {
            self.unwritten.push((key.id(), at, v));
        }
        self.push(key, at, v);
    }

    fn push(&mut self, key: Key, at: u64, v: Option<f32>) {
        let q = self.raw.entry(key).or_default();
        // Rounds of different Probe Groups can report out of order; keep Raw in time order.
        let i = q.iter().rposition(|(t, _)| *t <= at).map_or(0, |i| i + 1);
        q.insert(i, (at, v));
    }

    /// Ages out Raw, writes every Archive step that is due, and saves Raw when it is time.
    fn maintain(&mut self, now: u64) {
        self.clock = self.clock.max(now);
        let now = self.clock;
        for level in 0..3 {
            self.write_due(level, now);
        }
        let keep = self.raw_floor();
        for q in self.raw.values_mut() {
            while q.front().is_some_and(|(t, _)| *t < keep) {
                q.pop_front();
            }
        }
        self.pauses.retain(|p| p.end > keep as f64);
        if now >= self.raw_written_at + RAW_WRITE_S {
            self.write_raw();
        }
    }

    /// The oldest `at` Raw keeps: `at > now − raw_s`, and every A1 step not yet written.
    fn raw_floor(&self) -> u64 {
        let floor = (self.clock + 1).saturating_sub(self.limits.raw_s);
        self.cursors[0].map_or(floor, |c| floor.min(c))
    }

    fn write_raw(&mut self) {
        let oldest = self.raw_floor().saturating_sub(1);
        self.store
            .write_raw(&self.unwritten, &self.unwritten_pauses, oldest);
        self.unwritten.clear();
        self.unwritten_pauses.clear();
        self.raw_written_at = self.clock;
    }

    /// Writes each step of Archive `level` that is due by `now`, skipping stretches with nothing in them.
    fn write_due(&mut self, level: usize, now: u64) {
        let (step, rows) = self.limits.archives[level];
        let g = self.limits.grace_s;
        let first = if level == 0 {
            self.raw
                .values()
                .filter_map(|q| q.front().map(|(t, _)| *t))
                .min()
        } else {
            self.cursors[level - 1]
        };
        let Some(cursor) = self.cursors[level].or(first.map(|t| t / step * step)) else {
            return;
        };
        self.cursors[level] = Some(cursor);
        let mut wrote = false;
        while let Some(m) = self.cursors[level].filter(|m| written(*m, step, now, g)) {
            let next = self.next_data(level, m).map(|t| t / step * step);
            // The last step due is the one ending g s or more before now.
            let after_due = (now - g - step) / step * step + step;
            let target = next.unwrap_or(after_due).min(after_due);
            if target > m {
                self.cursors[level] = Some(target);
                continue;
            }
            self.write_step(level, m);
            self.cursors[level] = Some(m + step);
            wrote = true;
        }
        if wrote {
            // Rows go once `m + step ≤ now − rows × step` (spec `rules/history.md`, Archives).
            if let Some(limit) = now.checked_sub((rows + 1) * step) {
                self.store.retain(level, limit + 1);
            }
            self.generation += 1;
            if level == 0 {
                self.storage = self.measure();
            }
        }
    }

    /// The first time at or after `m` with anything for Archive `level` to write.
    fn next_data(&self, level: usize, m: u64) -> Option<u64> {
        if level > 0 {
            return self.store.next_step(level - 1, m);
        }
        let raw = self
            .raw
            .values()
            .filter_map(|q| q.iter().find(|(t, _)| *t >= m).map(|(t, _)| *t))
            .min();
        let paused = self
            .pauses
            .iter()
            .filter(|p| p.end > m as f64)
            .map(|p| (p.begin.max(m as f64)) as u64)
            .min();
        raw.into_iter().chain(paused).min()
    }

    fn write_step(&mut self, level: usize, m: u64) {
        let step = self.limits.archives[level].0;
        let (rows, paused): (Vec<(String, Row)>, f64) = if level == 0 {
            let rows = self
                .raw
                .iter()
                .filter_map(|(key, q)| {
                    let vs: Vec<Option<f32>> = q
                        .iter()
                        .filter(|(t, _)| (m..m + step).contains(t))
                        .map(|(_, v)| *v)
                        .collect();
                    (!vs.is_empty()).then(|| (key.id(), Row::of(&vs)))
                })
                .collect();
            (rows, paused_s(&self.pauses, m as f64, (m + step) as f64))
        } else {
            let rows = self
                .store
                .rows_in(level - 1, m, m + step)
                .into_iter()
                .map(|(key, rs)| {
                    let mut acc = Row::default();
                    for (_, r) in &rs {
                        acc.merge(r);
                    }
                    (key, acc)
                })
                .collect();
            let paused = self
                .store
                .paused(level - 1, m)
                .into_iter()
                .filter(|(x, _)| *x < m + step)
                .map(|(_, s)| s)
                .sum();
            (rows, paused)
        };
        self.store.write_step(level, m, &rows, paused, m + step);
    }

    fn measure(&self) -> Option<Storage> {
        let (bytes, rows) = self.store.measure()?;
        Some(Storage { bytes, rows })
    }
}

/// A Probe's stats from its `(at, rtt_ms)` results in arrival order (spec `rules/probe-stats.md`).
/// The window is `at > now − window_s` and the baseline spans Raw, `at > now − raw_s`, both
/// back from `now` so results from before a Pause age out. An empty window has no stats.
pub fn probe_stats(
    results: &[(u64, Option<f32>)],
    now: u64,
    window_s: u64,
    raw_s: u64,
) -> Option<ProbeStat> {
    let window: Vec<Option<f32>> = results
        .iter()
        .filter(|(t, _)| t + window_s > now)
        .map(|(_, v)| *v)
        .collect();
    if window.is_empty() {
        return None;
    }
    let replies: Vec<f32> = window.iter().flatten().copied().collect();
    let raw: Vec<f32> = results
        .iter()
        .filter(|(t, _)| t + raw_s > now)
        .filter_map(|(_, v)| *v)
        .collect();
    let jitter = (replies.len() > 1).then(|| {
        replies.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (replies.len() - 1) as f32
    });
    Some(ProbeStat {
        loss: 1.0 - replies.len() as f32 / window.len() as f32,
        rtt_ms: percentile(replies, 50),
        // Lowest decile: what the path does when nothing is queued, so a long congestion
        // episode can't become "normal".
        baseline_ms: percentile(raw, 10),
        jitter_ms: jitter,
    })
}

/// Writes every Archive Row of every key to `out` as CSV, reading `db` through its own connection
/// so the engine keeps sampling. Returns the number of rows.
pub fn export_csv(db: &Path, out: &Path) -> std::io::Result<u64> {
    use std::io::Write;
    let err = |e: rusqlite::Error| std::io::Error::other(e);
    let c =
        Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    let mut stmt = c
        .prepare(
            "SELECT k.name, r.archive, r.m, r.n, r.lost, r.sum, r.min, r.max, r.sk FROM row r
             JOIN key k ON k.id = r.key ORDER BY k.name, r.archive, r.m",
        )
        .map_err(err)?;
    let mut rows = stmt.query([]).map_err(err)?;
    let mut w = std::io::BufWriter::new(std::fs::File::create(out)?);
    writeln!(
        w,
        "series,archive,start_utc,unix,samples,lost,loss,avg,min,max,p50,p95,p99"
    )?;
    let mut count = 0;
    while let Some(r) = rows.next().map_err(err)? {
        let m = store::row(r, 3).map_err(err)?;
        let archive = ["A1", "A2", "A3"]
            .get(r.get::<_, i64>(1).map_err(err)? as usize)
            .copied()
            .unwrap_or("?");
        let at = r.get::<_, i64>(2).map_err(err)? as u64;
        let name: String = r.get(0).map_err(err)?;
        let v = |a| m.value(a).map(|v| format!("{v}")).unwrap_or_default();
        let bound = |v: f64| {
            if m.n > 0 {
                format!("{v}")
            } else {
                String::new()
            }
        };
        writeln!(
            w,
            "{},{archive},{},{at},{},{},{},{},{},{},{},{},{}",
            csv_field(&name),
            utc(at),
            m.n,
            m.lost,
            m.loss(),
            v(Aggregate::Avg),
            bound(m.min),
            bound(m.max),
            v(Aggregate::P50),
            v(Aggregate::P95),
            v(Aggregate::P99),
        )?;
        count += 1;
    }
    w.flush()?;
    Ok(count)
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// ISO 8601 UTC, from Howard Hinnant's days-to-civil algorithm (https://howardhinnant.github.io/date_algorithms.html).
fn utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:00Z",
        rem / 3600,
        rem % 3600 / 60
    )
}

/// The X-th percentile of `v`: sorted, the element at `⌊(n − 1) × X / 100⌋` in integer
/// arithmetic, the one rank every percentile uses (spec `rules/probe-stats.md`).
fn percentile(mut v: Vec<f32>, x: usize) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    Some(v[(v.len() - 1) * x / 100])
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Protocol, Target};

    fn limits() -> Limits {
        Limits {
            raw_s: 7200,
            archives: [(300, 288), (3600, 168), (86_400, 365)],
            hop_window: 60,
            recent_window_s: 30,
            grace_s: 2,
        }
    }

    fn probe() -> Probe {
        Probe {
            target: Target::Addr("1.1.1.1".parse().unwrap()),
            protocol: Protocol::Icmp,
        }
    }

    fn key() -> Key {
        Key::Probe(probe())
    }

    fn feed(h: &mut History, from: u64, to: u64, f: impl Fn(u64) -> Option<f32>) {
        for t in from..to {
            h.record_probe(&ProbeResult {
                probe: probe(),
                at: t,
                rtt_ms: f(t),
            });
        }
    }

    /// A temporary database path, removed when dropped.
    struct TempDb(std::path::PathBuf);

    impl TempDb {
        fn new(name: &str) -> TempDb {
            let p = std::env::temp_dir().join(format!("bg-{name}-{}.db", std::process::id()));
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", p.display()));
            }
            TempDb(p)
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", self.0.display()));
            }
        }
    }

    /// An hour before now, on a day boundary's 5-minute grid, so reopening keeps it in Raw.
    fn recent_start() -> u64 {
        (model::now() - 3600) / 300 * 300
    }

    #[test]
    fn aggregate_over_window() {
        let mut h = History::open(None, limits());
        feed(&mut h, 1000, 1300, |t| {
            (t % 10 != 0).then_some((t % 100) as f32)
        });
        let a = |agg, w| h.aggregate(&probe(), 1299, w, agg).value;
        assert_eq!(a(Aggregate::Max, 300), Some(99.0));
        assert_eq!(a(Aggregate::Max, 20), Some(99.0));
        assert_eq!(a(Aggregate::Max, 5), Some(99.0));
        assert_eq!(a(Aggregate::P50, 300), Some(49.0));
        assert_eq!(a(Aggregate::P95, 300), Some(95.0));
        assert_eq!(a(Aggregate::Avg, 10), Some(95.0));
        let loss = h.aggregate(&probe(), 1299, 300, Aggregate::P50).loss;
        assert!((loss - 0.1).abs() < 1e-6);
    }

    #[test]
    fn stat_reports_loss_and_baseline() {
        let mut h = History::open(None, limits());
        feed(&mut h, 0, 600, |_| Some(10.0));
        feed(&mut h, 600, 630, |t| (t % 2 == 0).then_some(80.0));
        let s = h.now(&probe(), 629).unwrap();
        assert!((s.loss - 0.5).abs() < 0.01, "{s:?}");
        assert_eq!(s.rtt_ms, Some(80.0));
        assert_eq!(s.baseline_ms, Some(10.0));
    }

    #[test]
    fn raw_keeps_two_hours() {
        let mut h = History::open(None, limits());
        feed(&mut h, 0, 3 * 3600, |_| Some(1.0));
        let kept = h.recent(&key(), usize::MAX).len() as u64;
        // Two hours, plus the A1 step not yet written and its grace.
        assert!((7200..7200 + 300 + 2).contains(&kept), "{kept}");
    }

    #[test]
    fn closed_points_never_change() {
        let mut h = History::open(None, limits());
        feed(&mut h, 0, 3600, |t| Some((t % 97) as f32));
        let before = h.series(&key(), Range::Day, 3600, Aggregate::P95);
        feed(&mut h, 3600, 4800, |t| Some((t % 89) as f32));
        let after = h.series(&key(), Range::Day, 4800, Aggregate::P95);
        let closed: Vec<_> = before.iter().filter(|p| !p.provisional).collect();
        assert_eq!(closed.len(), 11);
        for p in closed {
            assert_eq!(after.iter().find(|q| q.at == p.at), Some(p));
        }
    }

    #[test]
    fn archives_build_up_and_a3_serves_all() {
        let mut h = History::open(None, limits());
        // Every 10 s for three days.
        for t in (0..3 * 86_400).step_by(10) {
            h.record_probe(&ProbeResult {
                probe: probe(),
                at: t,
                rtt_ms: Some(20.0),
            });
        }
        let week = h.series(&key(), Range::Week, 3 * 86_400, Aggregate::Max);
        assert_eq!(week.iter().filter(|p| !p.provisional).count(), 71);
        let all = h.series(&key(), Range::All, 3 * 86_400, Aggregate::Max);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].value, Some(20.0));
        assert!(all[2].provisional);
        assert_eq!(h.oldest(), Some(0));
    }

    #[test]
    fn raw_and_archives_survive_a_kill() {
        let db = TempDb::new("kill");
        let t0 = recent_start();
        {
            let mut h = History::open(Some(&db.0), limits());
            feed(&mut h, t0, t0 + 900, |_| Some(12.0));
            // Dropped without close, as a kill or hibernate leaves it.
        }
        let h = History::open(Some(&db.0), limits());
        // Raw reached disk within RAW_WRITE_S of the last result.
        let kept = h.recent(&key(), usize::MAX).len() as u64;
        assert!(kept + RAW_WRITE_S >= 900, "{kept}");
        let day = h.series(&key(), Range::Day, t0 + 900, Aggregate::Max);
        assert_eq!(day.iter().filter(|p| !p.provisional).count(), 2);
        assert!(day.iter().all(|p| p.value == Some(12.0)));
    }

    #[test]
    fn a_restart_writes_the_steps_it_missed() {
        let db = TempDb::new("restart");
        let t0 = recent_start();
        {
            let mut h = History::open(Some(&db.0), limits());
            feed(&mut h, t0, t0 + 250, |_| Some(5.0));
            h.close(t0 + 250);
        }
        let mut h = History::open(Some(&db.0), limits());
        feed(&mut h, t0 + 700, t0 + 701, |_| Some(5.0));
        let day = h.series(&key(), Range::Day, t0 + 701, Aggregate::Max);
        let first = day.first().unwrap();
        assert_eq!((first.at, first.provisional), (t0, false));
    }

    #[test]
    fn an_older_database_starts_empty() {
        let db = TempDb::new("old");
        {
            let c = Connection::open(&db.0).unwrap();
            c.execute_batch("CREATE TABLE minute (key INTEGER, minute INTEGER);")
                .unwrap();
        }
        let _h = History::open(Some(&db.0), limits());
        let c = Connection::open(&db.0).unwrap();
        let minute: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'minute'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(minute, 0);
    }

    #[test]
    fn storage_is_measured_and_estimated() {
        let db = TempDb::new("size");
        let mut h = History::open(Some(&db.0), limits());
        let t0 = recent_start();
        feed(&mut h, t0, t0 + 610, |t| Some((t % 7) as f32));
        let s = h.storage().unwrap();
        assert_eq!(s.rows, 2);
        assert!(s.bytes > 0);
        let rows = 288 + 168 + 7;
        assert_eq!(
            estimate_bytes(20, &limits(), 7, Some(s)),
            (20.0 * rows as f64 * 94.0) as u64
        );
        let big = Storage {
            bytes: 2_000_000,
            rows: 20_000,
        };
        assert_eq!(estimate_bytes(10, &limits(), 1, Some(big)), 457 * 10 * 100);
    }

    #[test]
    fn utc_formats_known_dates() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_800_000_000), "2027-01-15T08:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn export_writes_every_row() {
        let db = TempDb::new("export");
        let out = db.0.with_extension("csv");
        let mut h = History::open(Some(&db.0), limits());
        let t0 = recent_start();
        feed(&mut h, t0, t0 + 610, |t| {
            (t % 30 != 0).then_some((t % 7) as f32)
        });
        assert_eq!(export_csv(&db.0, &out).unwrap(), 2);
        let csv = std::fs::read_to_string(&out).unwrap();
        let first = csv.lines().nth(1).unwrap();
        assert!(first.starts_with("probe:icmp 1.1.1.1,A1,"), "{first}");
        assert!(first.contains(",290,10,"), "{first}");
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn hops_truncate_at_destination() {
        let mut h = History::open(None, limits());
        for ttl in 1..=5u8 {
            h.record_hop(&HopResult {
                trace: probe(),
                at: 1,
                ttl,
                addr: Some(format!("10.0.0.{ttl}").parse().unwrap()),
                rtt_ms: (ttl != 2).then_some(ttl as f32),
                reached: false,
            });
        }
        h.record_hop(&HopResult {
            trace: probe(),
            at: 2,
            ttl: 3,
            addr: None,
            rtt_ms: Some(3.0),
            reached: true,
        });
        let hops = h.hops(&probe());
        assert_eq!(hops.len(), 3);
        assert_eq!(hops[1].loss, 1.0);
        assert_eq!(hops[2].addr, Some("10.0.0.3".parse().unwrap()));
    }
}
