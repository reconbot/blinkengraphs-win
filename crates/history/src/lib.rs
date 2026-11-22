mod sketch;
pub use sketch::{Sketch, index as sketch_index, rep as sketch_rep};

use model::{Aggregate, HopResult, Metric, Probe, ProbeResult, ProbeStat, Sample};
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    path::Path,
};

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

/// One graph bucket starting at `at`: the chosen aggregate (NaN when nothing replied) and the lost fraction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// The bucket's start.
    pub at: u64,
    /// The bucket's width: whole minutes from storage, or the RAM bucket width.
    pub secs: u64,
    pub value: f32,
    pub loss: f32,
}

/// Everything kept about one key's minute, or a merge of several (spec `rules/graph-aggregate.md`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Minute {
    pub n: u64,
    pub lost: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub sketch: Sketch,
}

impl Minute {
    /// `values` in arrival order, so `sum` is reproducible.
    pub fn of(values: &[Option<f32>]) -> Self {
        let mut m = Minute {
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            ..Minute::default()
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

    /// Merge minutes in ascending time order; f64 addition is not associative.
    pub fn merge(&mut self, o: &Minute) {
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

/// Raw values in one bucket, aggregated exactly with the same rank rule as the sketch.
fn exact(values: &[Option<f32>], agg: Aggregate) -> Option<f64> {
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
/// (spec `defaults.json` `history` and `hop_trace`), plus the user's retention choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub ram_minutes: u32,
    pub rollup_days: u32,
    /// Replies kept per Hop.
    pub hop_window: usize,
    /// The window "now" means for loss, RTT and the Verdict.
    pub recent_window_s: u64,
    /// A minute is written once a result `60 + minute_grace_s` past its start arrives, so a
    /// result that arrives a timeout after its `at` still counts (spec `rules/history.md`).
    pub minute_grace_s: u64,
}

#[derive(Default)]
struct HopTrack {
    addr: Option<IpAddr>,
    rtts: VecDeque<Option<f32>>,
    reached: bool,
}

pub struct History {
    hop_window: usize,
    recent_secs: u64,
    minute_grace: u64,
    ram_secs: u64,
    rollup_secs: u64,
    ram: HashMap<Key, VecDeque<(u64, Option<f32>)>>,
    hops: HashMap<Probe, Vec<HopTrack>>,
    last_sample: Option<Sample>,
    db: Option<Connection>,
    flushed_minute: u64,
    storage: Option<Storage>,
    oldest_minute: Option<u64>,
}

/// What the history database holds now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Storage {
    pub bytes: u64,
    pub minutes: u64,
}

/// Bytes per stored Minute when the database is too young to measure (research 06, synthetic data).
const BYTES_PER_MINUTE_ROW: f64 = 94.0;

/// Disk needed to keep `days` of history for `series` keys, from the measured row size once there is enough data.
pub fn estimate_bytes(series: usize, days: u32, measured: Option<Storage>) -> u64 {
    let per_row = measured
        .filter(|s| s.minutes >= 1000)
        .map_or(BYTES_PER_MINUTE_ROW, |s| s.bytes as f64 / s.minutes as f64);
    (series as f64 * days as f64 * 1440.0 * per_row) as u64
}

impl History {
    /// `db: None` keeps everything in RAM (tests, or a disk that refuses writes).
    pub fn open(db: Option<&Path>, limits: Limits) -> Self {
        let db = db.and_then(|p| {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).ok()?;
            }
            let c = Connection::open(p).ok()?;
            c.execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE IF NOT EXISTS key (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
                 CREATE TABLE IF NOT EXISTS minute (
                   key INTEGER NOT NULL, minute INTEGER NOT NULL,
                   n INTEGER NOT NULL, lost INTEGER NOT NULL,
                   sum REAL, min REAL, max REAL, sk BLOB NOT NULL,
                   PRIMARY KEY (key, minute)) WITHOUT ROWID;
                 CREATE TABLE IF NOT EXISTS recent (
                   key TEXT NOT NULL, at INTEGER NOT NULL, v REAL,
                   PRIMARY KEY (key, at)) WITHOUT ROWID;",
            )
            .ok()?;
            migrate_v1(&c);
            Some(c)
        });
        let mut h = Self {
            hop_window: limits.hop_window,
            recent_secs: limits.recent_window_s,
            minute_grace: limits.minute_grace_s,
            // A minute is written g s after it ends, from RAM, so RAM must outlast that.
            ram_secs: (limits.ram_minutes as u64 * 60).max(61 + limits.minute_grace_s),
            rollup_secs: limits.rollup_days as u64 * 86_400,
            ram: HashMap::new(),
            hops: HashMap::new(),
            last_sample: None,
            db,
            flushed_minute: 0,
            storage: None,
            oldest_minute: None,
        };
        h.restore();
        h
    }

    // Due minutes are written before pushing evicts their oldest results.
    pub fn record_sample(&mut self, s: &Sample) {
        self.maintain(s.at);
        for m in Metric::ALL {
            self.push(Key::Metric(m), s.at, Some(s.get(m) as f32));
        }
        self.last_sample = Some(s.clone());
    }

    pub fn record_probe(&mut self, r: &ProbeResult) {
        self.maintain(r.at);
        self.push(Key::Probe(r.probe.clone()), r.at, r.rtt_ms);
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
        if t.rtts.len() > self.hop_window {
            t.rtts.pop_front();
        }
        if h.reached {
            track.truncate(i + 1);
        }
    }

    pub fn probes(&self) -> Vec<Probe> {
        self.ram
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
        self.ram
            .get(key)
            .map(|q| {
                q.iter()
                    .skip(q.len().saturating_sub(n))
                    .map(|(_, v)| *v)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The Probe's stats over the recent window (`Limits::recent_window_s`).
    pub fn now(&self, probe: &Probe) -> ProbeStat {
        self.stat(probe, self.recent_secs)
    }

    pub fn stat(&self, probe: &Probe, window_secs: u64) -> ProbeStat {
        let Some(q) = self.ram.get(&Key::Probe(probe.clone())) else {
            return ProbeStat::default();
        };
        let newest = q.back().map(|(t, _)| *t).unwrap_or(0);
        let window: Vec<Option<f32>> = q
            .iter()
            .filter(|(t, _)| *t + window_secs > newest)
            .map(|(_, v)| *v)
            .collect();
        let replies: Vec<f32> = window.iter().flatten().copied().collect();
        let all: Vec<f32> = q.iter().filter_map(|(_, v)| *v).collect();
        let jitter = (replies.len() > 1).then(|| {
            replies.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>()
                / (replies.len() - 1) as f32
        });
        ProbeStat {
            loss: if window.is_empty() {
                0.0
            } else {
                1.0 - replies.len() as f32 / window.len() as f32
            },
            rtt_ms: percentile(replies, 50),
            // Lowest decile: what the path does when nothing is queued, so a long congestion
            // episode can't become "normal".
            baseline_ms: percentile(all, 10),
            jitter_ms: jitter,
        }
    }

    /// One value for `probe` over the last `window_secs`, with the share of Probes lost.
    pub fn aggregate(&self, probe: &Probe, window_secs: u64, agg: Aggregate) -> Summary {
        let Some(q) = self.ram.get(&Key::Probe(probe.clone())) else {
            return Summary::default();
        };
        let newest = q.back().map(|(t, _)| *t).unwrap_or(0);
        let window: Vec<Option<f32>> = q
            .iter()
            .filter(|(t, _)| *t + window_secs > newest)
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

    /// Rollups for the part of `[since, until]` older than RAM, then raw RAM points.
    /// `agg` per bucket of `bucket_secs` over `[since, until]`: Minutes from SQLite for the part older
    /// than RAM (buckets rounded up to whole minutes there), exact raw values from RAM after that.
    /// Graph points for `[since, until]` in buckets of `bucket_secs` (spec `rules/history.md`).
    pub fn series(
        &self,
        key: &Key,
        since: u64,
        until: u64,
        bucket_secs: u64,
        agg: Aggregate,
    ) -> Vec<Point> {
        let empty = VecDeque::new();
        let ram = self.ram.get(key).unwrap_or(&empty);
        // Without a database nothing is written, so RAM serves everything.
        let c = if self.db.is_some() {
            self.flushed_minute
        } else {
            0
        };
        let stored = if since < c {
            self.minutes(key, since / 60 * 60, c)
        } else {
            vec![]
        };
        buckets(
            since,
            until,
            bucket_secs,
            c,
            ram.iter().copied(),
            &stored,
            agg,
        )
    }

    /// Saves what a restart would otherwise lose: the unfinished minute's rollup and the raw RAM samples.
    pub fn close(&mut self, now: u64) {
        let from = self.flushed_minute;
        let Some(db) = &mut self.db else { return };
        let Ok(tx) = db.transaction() else { return };
        let _ = tx.execute("DELETE FROM recent", []);
        for (key, q) in &self.ram {
            let id = key.id();
            let mut minute: Vec<Option<f32>> = Vec::new();
            let mut at = None;
            for (t, v) in q {
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO recent VALUES (?1, ?2, ?3)",
                    params![id, *t as i64, v.map(f64::from)],
                );
                if *t >= from && *t <= now {
                    if at.is_some_and(|a| a != t / 60 * 60) {
                        flush_one(&tx, &id, at.unwrap(), &minute);
                        minute.clear();
                    }
                    at = Some(t / 60 * 60);
                    minute.push(*v);
                }
            }
            if let Some(a) = at {
                flush_one(&tx, &id, a, &minute);
            }
        }
        let _ = tx.commit();
    }

    /// Reloads the raw samples `close` saved, then forgets them so they're read once.
    fn restore(&mut self) {
        let Some(db) = &self.db else { return };
        let rows: Vec<(String, i64, Option<f64>)> = db
            .prepare("SELECT key, at, v FROM recent ORDER BY at")
            .and_then(|mut s| {
                s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default();
        let _ = db.execute("DELETE FROM recent", []);
        let oldest = model::now().saturating_sub(self.ram_secs) as i64;
        for (id, at, v) in rows.into_iter().filter(|r| r.1 >= oldest) {
            if let Some(key) = Key::from_id(&id) {
                self.push(key, at as u64, v.map(|v| v as f32));
            }
        }
    }

    /// Measured at each minute's flush; `None` without a database.
    pub fn storage(&self) -> Option<Storage> {
        self.storage
    }

    /// Unix seconds of the oldest data held anywhere: SQLite as of the last flush, else RAM.
    pub fn oldest(&self) -> Option<u64> {
        let ram = self
            .ram
            .values()
            .filter_map(|q| q.front().map(|(t, _)| *t))
            .min();
        match (self.oldest_minute, ram) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn set_rollup_days(&mut self, days: u32) {
        self.rollup_secs = days.max(1) as u64 * 86_400;
    }

    /// Stored Minutes for `key` in `[from, to)`, ascending.
    pub fn minutes(&self, key: &Key, from: u64, to: u64) -> Vec<(u64, Minute)> {
        let Some(db) = &self.db else { return vec![] };
        let mut stmt = db
            .prepare_cached(
                "SELECT m.minute, m.n, m.lost, m.sum, m.min, m.max, m.sk FROM minute m
                 JOIN key k ON k.id = m.key
                 WHERE k.name = ?1 AND m.minute >= ?2 AND m.minute < ?3 ORDER BY m.minute",
            )
            .expect("static SQL");
        let rows = stmt.query_map(params![key.id(), from as i64, to as i64], |r| {
            let sk: Vec<u8> = r.get(6)?;
            Ok((
                r.get::<_, i64>(0)? as u64,
                Minute {
                    n: r.get::<_, i64>(1)? as u64,
                    lost: r.get::<_, i64>(2)? as u64,
                    sum: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                    min: r.get::<_, Option<f64>>(4)?.unwrap_or(f64::INFINITY),
                    max: r.get::<_, Option<f64>>(5)?.unwrap_or(f64::NEG_INFINITY),
                    sketch: Sketch::decode(&sk).unwrap_or_default(),
                },
            ))
        });
        rows.into_iter().flatten().flatten().collect()
    }

    fn push(&mut self, key: Key, at: u64, v: Option<f32>) {
        let q = self.ram.entry(key).or_default();
        // Rounds of different Probe Groups can report out of order; keep RAM in time order.
        let i = q.iter().rposition(|(t, _)| *t <= at).map_or(0, |i| i + 1);
        q.insert(i, (at, v));
        // Keep at + ram > newest: exactly `ram` back is gone (spec `rules/history.md`, RAM).
        while q.front().is_some_and(|(t, _)| *t + self.ram_secs <= at) {
            q.pop_front();
        }
    }

    /// Writes one Rollup per key for every minute that can get no more results.
    fn maintain(&mut self, now: u64) {
        let minute = now.saturating_sub(self.minute_grace) / 60 * 60;
        if self.flushed_minute == 0 {
            self.flushed_minute = minute;
            return;
        }
        if minute <= self.flushed_minute {
            return;
        }
        let (from, to) = (self.flushed_minute, minute);
        self.flushed_minute = minute;
        let Some(db) = &mut self.db else { return };
        let Ok(tx) = db.transaction() else { return };
        for (key, q) in &self.ram {
            let mut buckets: BTreeMap<u64, Vec<Option<f32>>> = BTreeMap::new();
            for (t, v) in q.iter().filter(|(t, _)| (from..to).contains(t)) {
                buckets.entry(t / 60 * 60).or_default().push(*v);
            }
            if buckets.is_empty() {
                continue;
            }
            let _ = tx.execute(
                "INSERT OR IGNORE INTO key (name) VALUES (?1)",
                params![key.id()],
            );
            for (at, vs) in buckets {
                write_minute(&tx, &key.id(), at, &Minute::of(&vs));
            }
        }
        let _ = tx.execute(
            "DELETE FROM minute WHERE minute < ?1",
            params![now.saturating_sub(self.rollup_secs) as i64],
        );
        let _ = tx.commit();
        let one = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64;
        self.storage = Some(Storage {
            bytes: one("PRAGMA page_count").saturating_sub(one("PRAGMA freelist_count"))
                * one("PRAGMA page_size"),
            minutes: one("SELECT COUNT(*) FROM minute"),
        });
        self.oldest_minute = db
            .query_row("SELECT MIN(minute) FROM minute", [], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .ok()
            .flatten()
            .map(|m| m as u64);
    }
}

/// Writes every stored Minute of every key to `out` as CSV, reading `db` through its own connection
/// so the engine keeps sampling. Returns the number of rows.
pub fn export_csv(db: &Path, out: &Path) -> std::io::Result<u64> {
    use std::io::Write;
    let err = |e: rusqlite::Error| std::io::Error::other(e);
    let c =
        Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    let mut stmt = c
        .prepare(
            "SELECT k.name, m.minute, m.n, m.lost, m.sum, m.min, m.max, m.sk FROM minute m
             JOIN key k ON k.id = m.key ORDER BY k.name, m.minute",
        )
        .map_err(err)?;
    let mut rows = stmt.query([]).map_err(err)?;
    let mut w = std::io::BufWriter::new(std::fs::File::create(out)?);
    writeln!(
        w,
        "series,minute_utc,unix,samples,lost,loss,avg,min,max,p50,p95,p99"
    )?;
    let mut count = 0;
    while let Some(r) = rows.next().map_err(err)? {
        let sk: Vec<u8> = r.get(7).map_err(err)?;
        let m = Minute {
            n: r.get::<_, i64>(2).map_err(err)? as u64,
            lost: r.get::<_, i64>(3).map_err(err)? as u64,
            sum: r.get::<_, Option<f64>>(4).map_err(err)?.unwrap_or(0.0),
            min: r
                .get::<_, Option<f64>>(5)
                .map_err(err)?
                .unwrap_or(f64::INFINITY),
            max: r
                .get::<_, Option<f64>>(6)
                .map_err(err)?
                .unwrap_or(f64::NEG_INFINITY),
            sketch: Sketch::decode(&sk).unwrap_or_default(),
        };
        let at = r.get::<_, i64>(1).map_err(err)? as u64;
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
            "{},{},{at},{},{},{},{},{},{},{},{},{}",
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

fn flush_one(c: &Connection, key: &str, at: u64, values: &[Option<f32>]) {
    let _ = c.execute("INSERT OR IGNORE INTO key (name) VALUES (?1)", params![key]);
    write_minute(c, key, at, &Minute::of(values));
}

/// A graph's RAM bucket width for a span shown in up to 240 points (spec `rules/history.md`).
pub fn bucket_width(span_secs: u64) -> u64 {
    (span_secs / 240).max(1)
}

/// Graph points (spec `rules/history.md`, "Graph buckets"). `written` is where storage hands
/// over to RAM: every minute before it is written, none from it on. Stored Minutes before it go
/// in whole-minute buckets and RAM results from it on in `w`-second buckets, both aligned to
/// the epoch, so nothing is counted twice or left out.
pub fn buckets(
    since: u64,
    until: u64,
    w: u64,
    written: u64,
    ram: impl Iterator<Item = (u64, Option<f32>)>,
    stored: &[(u64, Minute)],
    agg: Aggregate,
) -> Vec<Point> {
    let w = w.max(1);
    let c = written;
    let mut out = Vec::new();
    let big = w.div_ceil(60) * 60;
    let mut cur: Option<(u64, Minute)> = None;
    for (m, minute) in stored.iter().filter(|(m, _)| since <= m + 59 && *m < c) {
        let start = m / big * big;
        match &mut cur {
            Some((s, acc)) if *s == start => acc.merge(minute),
            _ => {
                if let Some((s, acc)) = cur.take() {
                    out.push(point(s, big, &acc, agg));
                }
                cur = Some((start, minute.clone()));
            }
        }
    }
    if let Some((s, acc)) = cur {
        out.push(point(s, big, &acc, agg));
    }
    let mut cur: Option<(u64, Vec<Option<f32>>)> = None;
    let mut flush = |c: Option<(u64, Vec<Option<f32>>)>| {
        if let Some((s, vs)) = c {
            let lost = vs.iter().filter(|v| v.is_none()).count() as f32 / vs.len() as f32;
            out.push(Point {
                at: s,
                secs: w,
                value: exact(&vs, agg).map_or(f32::NAN, |v| v as f32),
                loss: lost,
            });
        }
    };
    for (t, v) in ram.filter(|(t, _)| (since.max(c)..=until).contains(t)) {
        let start = t / w * w;
        match &mut cur {
            Some((s, vs)) if *s == start => vs.push(v),
            _ => {
                flush(cur.take());
                cur = Some((start, vec![v]));
            }
        }
    }
    flush(cur);
    out
}

fn point(at: u64, secs: u64, m: &Minute, agg: Aggregate) -> Point {
    Point {
        at,
        secs,
        value: m.value(agg).map_or(f32::NAN, |v| v as f32),
        loss: m.loss(),
    }
}

fn write_minute(c: &Connection, key: &str, at: u64, m: &Minute) {
    let some = |v: f64| (m.n > 0).then_some(v);
    let _ = c.execute(
        "INSERT OR REPLACE INTO minute
         SELECT id, ?2, ?3, ?4, ?5, ?6, ?7, ?8 FROM key WHERE name = ?1",
        params![
            key,
            at as i64,
            m.n as i64,
            m.lost as i64,
            some(m.sum),
            some(m.min),
            some(m.max),
            m.sketch.encode()
        ],
    );
}

/// v1.x stored min/mean/max/loss per minute. Each becomes a Minute holding its mean once, so old
/// history still graphs (percentiles of it read as the mean) until it ages out of the rollup window.
/// key, minute, min, mean, max, loss
type V1Row = (String, i64, Option<f64>, Option<f64>, Option<f64>, f64);

fn migrate_v1(c: &Connection) {
    let old: Vec<V1Row> = c
        .prepare("SELECT key, minute, min, mean, max, loss FROM rollup")
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();
    if old.is_empty() {
        let _ = c.execute_batch("DROP TABLE IF EXISTS rollup;");
        return;
    }
    let Ok(tx) = c.unchecked_transaction() else {
        return;
    };
    for (key, at, min, mean, max, loss) in old {
        let _ = tx.execute("INSERT OR IGNORE INTO key (name) VALUES (?1)", params![key]);
        let m = match mean {
            Some(mean) => {
                let mut sketch = Sketch::default();
                sketch.add(mean);
                Minute {
                    n: 1,
                    lost: (loss / (1.0 - loss).max(1e-9)).round() as u64,
                    sum: mean,
                    min: min.unwrap_or(mean),
                    max: max.unwrap_or(mean),
                    sketch,
                }
            }
            None => Minute {
                lost: 1,
                ..Minute::default()
            },
        };
        write_minute(&tx, &key, at as u64, &m);
    }
    let _ = tx.execute_batch("DROP TABLE rollup;");
    let _ = tx.commit();
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

    fn limits(ram_minutes: u32, rollup_days: u32) -> Limits {
        Limits {
            ram_minutes,
            rollup_days,
            hop_window: 60,
            recent_window_s: 30,
            minute_grace_s: 2,
        }
    }

    fn probe() -> Probe {
        Probe {
            target: Target::Addr("1.1.1.1".parse().unwrap()),
            protocol: Protocol::Icmp,
        }
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

    #[test]
    fn aggregate_over_window() {
        let mut h = History::open(None, limits(15, 7));
        feed(&mut h, 1000, 1300, |t| {
            (t % 10 != 0).then_some((t % 100) as f32)
        });
        let a = |agg, w| h.aggregate(&probe(), w, agg).value;
        assert_eq!(a(Aggregate::Max, 300), Some(99.0));
        assert_eq!(a(Aggregate::Max, 20), Some(99.0));
        assert_eq!(a(Aggregate::Max, 5), Some(99.0));
        assert_eq!(a(Aggregate::P50, 300), Some(49.0));
        assert_eq!(a(Aggregate::P95, 300), Some(95.0));
        assert_eq!(a(Aggregate::Avg, 10), Some(95.0));
        let loss = h.aggregate(&probe(), 300, Aggregate::P50).loss;
        assert!((loss - 0.1).abs() < 1e-6);
    }

    #[test]
    fn stat_reports_loss_and_baseline() {
        let mut h = History::open(None, limits(15, 7));
        feed(&mut h, 0, 600, |_| Some(10.0));
        feed(&mut h, 600, 630, |t| (t % 2 == 0).then_some(80.0));
        let s = h.stat(&probe(), 30);
        assert!((s.loss - 0.5).abs() < 0.01, "{s:?}");
        assert_eq!(s.rtt_ms, Some(80.0));
        assert_eq!(s.baseline_ms, Some(10.0));
    }

    #[test]
    fn ram_is_bounded() {
        let mut h = History::open(None, limits(1, 7));
        feed(&mut h, 0, 500, |_| Some(1.0));
        // One minute is raised to 61 + grace s, so minutes are written before eviction.
        assert_eq!(h.recent(&Key::Probe(probe()), 1000).len(), 63);
    }

    #[test]
    fn rollups_serve_ranges_older_than_ram() {
        let path = std::env::temp_dir().join(format!("bg-history-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut h = History::open(Some(&path), limits(1, 7));
        feed(&mut h, 6000, 6000 + 600, |t| {
            if t < 6060 {
                None
            } else {
                Some((t % 60) as f32)
            }
        });
        let key = Key::Probe(probe());
        let pts = h.series(&key, 6000, 6600, 60, Aggregate::Max);
        let first = pts.first().unwrap();
        assert_eq!(first.at, 6000);
        assert_eq!(first.loss, 1.0);
        assert!(first.value.is_nan());
        let rolled = pts.iter().find(|p| p.at == 6120).unwrap();
        assert_eq!(rolled.value, 59.0);
        assert!(pts.last().unwrap().at >= 6540);
        let two = h.series(&key, 6060, 6180, 120, Aggregate::Avg);
        assert_eq!(two[0].at, 6000);
        assert_eq!(two[1].at, 6120);
        assert_eq!(two[1].value, 29.5);
        let p95 = h.series(&key, 6120, 6180, 60, Aggregate::P95)[0].value;
        assert!((p95 - 56.0).abs() / 56.0 <= 1.0 / 65.0, "{p95}");
        drop(h);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn storage_is_measured_and_estimated() {
        let path = std::env::temp_dir().join(format!("bg-history-size-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut h = History::open(Some(&path), limits(1, 7));
        assert_eq!(h.storage(), None);
        feed(&mut h, 6000, 6000 + 300, |t| Some((t % 7) as f32));
        let s = h.storage().unwrap();
        assert_eq!(s.minutes, 4);
        assert!(s.bytes > 0);
        assert_eq!(
            estimate_bytes(20, 7, Some(s)),
            (20.0 * 7.0 * 1440.0 * 94.0) as u64
        );
        let big = Storage {
            bytes: 2_000_000,
            minutes: 20_000,
        };
        assert_eq!(estimate_bytes(10, 1, Some(big)), 1_440_000);
        drop(h);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn utc_formats_known_dates() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_800_000_000), "2027-01-15T08:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn export_writes_every_minute() {
        let dir = std::env::temp_dir().join(format!("bg-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("h.db");
        let out = dir.join("h.csv");
        let mut h = History::open(Some(&db), limits(1, 7));
        feed(&mut h, 6000, 6000 + 300, |t| {
            (t % 30 != 0).then_some((t % 7) as f32)
        });
        assert_eq!(h.oldest(), Some(6000));
        let rows = export_csv(&db, &out).unwrap();
        assert_eq!(rows, 4);
        let text = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("series,minute_utc,unix,"));
        assert!(
            lines[1].starts_with("probe:icmp 1.1.1.1,1970-01-01T01:40:00Z,6000,58,2,"),
            "{}",
            lines[1]
        );
        drop(h);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn close_keeps_recent_samples_and_the_unfinished_minute() {
        let path = std::env::temp_dir().join(format!("bg-history-close-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let key = Key::Probe(probe());
        let t0 = (model::now() - 200) / 60 * 60;
        {
            let mut h = History::open(Some(&path), limits(15, 7));
            feed(&mut h, t0, t0 + 150, |t| {
                (!(t - t0).is_multiple_of(50)).then_some((t % 9) as f32)
            });
            h.close(t0 + 150);
        }
        let h = History::open(Some(&path), limits(15, 7));
        let raw = h.recent(&key, 1000);
        assert_eq!(raw.len(), 150, "1 s samples survive a restart");
        assert_eq!(raw[0], None);
        let minutes = h.minutes(&key, t0, t0 + 200);
        assert_eq!(
            minutes.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            [t0, t0 + 60, t0 + 120]
        );
        assert_eq!(
            minutes[2].1.n + minutes[2].1.lost,
            30,
            "the unfinished minute is stored"
        );
        drop(h);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ram_buckets_are_exact() {
        let mut h = History::open(None, limits(15, 7));
        feed(&mut h, 1000, 1010, |t| Some((t - 1000) as f32));
        let pts = h.series(&Key::Probe(probe()), 1000, 1009, 5, Aggregate::P50);
        assert_eq!(pts.len(), 2);
        assert_eq!((pts[0].at, pts[0].value), (1000, 2.0));
        assert_eq!((pts[1].at, pts[1].value), (1005, 7.0));
    }

    #[test]
    fn v1_rollups_migrate() {
        let path = std::env::temp_dir().join(format!("bg-history-v1-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE rollup (key TEXT NOT NULL, minute INTEGER NOT NULL,
                   min REAL, mean REAL, max REAL, loss REAL NOT NULL,
                   PRIMARY KEY (key, minute)) WITHOUT ROWID;
                 INSERT INTO rollup VALUES ('probe:icmp 1.1.1.1', 6000, 10, 12, 20, 0.5);",
            )
            .unwrap();
        }
        let h = History::open(Some(&path), limits(1, 7));
        let m = h.minutes(&Key::Probe(probe()), 6000, 6060);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].1.n, m[0].1.lost, m[0].1.max), (1, 1, 20.0));
        assert_eq!(m[0].1.value(Aggregate::Avg), Some(12.0));
        drop(h);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hops_truncate_at_destination() {
        let mut h = History::open(None, limits(15, 7));
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
