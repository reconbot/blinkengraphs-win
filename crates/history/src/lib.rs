use model::{HopResult, Metric, Probe, ProbeResult, ProbeStat, Sample};
use rusqlite::{Connection, params};
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
}

/// A raw sample has min = mean = max; `loss` is the lost fraction (0 or 1 when raw).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub at: u64,
    pub min: f32,
    pub mean: f32,
    pub max: f32,
    pub loss: f32,
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

const HOP_WINDOW: usize = 60;
/// The window "now" means for loss, RTT and the Verdict.
pub const RECENT_SECS: u64 = 30;

#[derive(Default)]
struct HopTrack {
    addr: Option<IpAddr>,
    rtts: VecDeque<Option<f32>>,
    reached: bool,
}

pub struct History {
    ram_secs: u64,
    rollup_secs: u64,
    ram: HashMap<Key, VecDeque<(u64, Option<f32>)>>,
    hops: HashMap<Probe, Vec<HopTrack>>,
    last_sample: Option<Sample>,
    db: Option<Connection>,
    flushed_minute: u64,
}

impl History {
    /// `db: None` keeps everything in RAM (tests, or a disk that refuses writes).
    pub fn open(db: Option<&Path>, ram_minutes: u32, rollup_days: u32) -> Self {
        let db = db.and_then(|p| {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).ok()?;
            }
            let c = Connection::open(p).ok()?;
            c.execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE IF NOT EXISTS rollup (
                   key TEXT NOT NULL, minute INTEGER NOT NULL,
                   min REAL, mean REAL, max REAL, loss REAL NOT NULL,
                   PRIMARY KEY (key, minute)) WITHOUT ROWID;",
            )
            .ok()?;
            Some(c)
        });
        Self {
            ram_secs: ram_minutes as u64 * 60,
            rollup_secs: rollup_days as u64 * 86_400,
            ram: HashMap::new(),
            hops: HashMap::new(),
            last_sample: None,
            db,
            flushed_minute: 0,
        }
    }

    pub fn record_sample(&mut self, s: &Sample) {
        for m in Metric::ALL {
            self.push(Key::Metric(m), s.at, Some(s.get(m) as f32));
        }
        self.last_sample = Some(s.clone());
        self.maintain(s.at);
    }

    pub fn record_probe(&mut self, r: &ProbeResult) {
        self.push(Key::Probe(r.probe.clone()), r.at, r.rtt_ms);
        self.maintain(r.at);
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
        if t.rtts.len() > HOP_WINDOW {
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
            rtt_ms: median(replies),
            baseline_ms: median(all),
            jitter_ms: jitter,
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
    pub fn series(&self, key: &Key, since: u64, until: u64) -> Vec<Point> {
        let ram = self.ram.get(key);
        let ram_start = ram
            .and_then(|q| q.front())
            .map(|(t, _)| *t)
            .unwrap_or(until + 1);
        let mut out = Vec::new();
        if since < ram_start
            && let Some(db) = &self.db
        {
            let mut stmt = db
                .prepare_cached(
                    "SELECT minute, min, mean, max, loss FROM rollup
                         WHERE key = ?1 AND minute >= ?2 AND minute < ?3 ORDER BY minute",
                )
                .expect("static SQL");
            let rows = stmt.query_map(
                params![
                    key.id(),
                    (since / 60 * 60) as i64,
                    ram_start.min(until) as i64
                ],
                |r| {
                    let f = |i| {
                        r.get::<_, Option<f64>>(i)
                            .map(|v| v.unwrap_or(f64::NAN) as f32)
                    };
                    Ok(Point {
                        at: r.get::<_, i64>(0)? as u64,
                        min: f(1)?,
                        mean: f(2)?,
                        max: f(3)?,
                        loss: r.get::<_, f64>(4)? as f32,
                    })
                },
            );
            out.extend(rows.into_iter().flatten().flatten());
        }
        if let Some(q) = ram {
            out.extend(q.iter().filter(|(t, _)| (since..=until).contains(t)).map(
                |(t, v)| match v {
                    Some(v) => Point {
                        at: *t,
                        min: *v,
                        mean: *v,
                        max: *v,
                        loss: 0.0,
                    },
                    None => Point {
                        at: *t,
                        min: f32::NAN,
                        mean: f32::NAN,
                        max: f32::NAN,
                        loss: 1.0,
                    },
                },
            ));
        }
        out
    }

    fn push(&mut self, key: Key, at: u64, v: Option<f32>) {
        let q = self.ram.entry(key).or_default();
        q.push_back((at, v));
        while q.front().is_some_and(|(t, _)| *t + self.ram_secs < at) {
            q.pop_front();
        }
    }

    /// Writes one Rollup per key for every whole minute not yet flushed.
    fn maintain(&mut self, now: u64) {
        let minute = now / 60 * 60;
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
            let mut buckets: HashMap<u64, Vec<Option<f32>>> = HashMap::new();
            for (t, v) in q.iter().filter(|(t, _)| (from..to).contains(t)) {
                buckets.entry(t / 60 * 60).or_default().push(*v);
            }
            for (m, vs) in buckets {
                let replies: Vec<f32> = vs.iter().flatten().copied().collect();
                let loss = 1.0 - replies.len() as f64 / vs.len() as f64;
                let mean = (!replies.is_empty())
                    .then(|| replies.iter().sum::<f32>() as f64 / replies.len() as f64);
                let min = replies.iter().copied().reduce(f32::min).map(f64::from);
                let max = replies.iter().copied().reduce(f32::max).map(f64::from);
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO rollup VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![key.id(), m as i64, min, mean, max, loss],
                );
            }
        }
        let _ = tx.execute(
            "DELETE FROM rollup WHERE minute < ?1",
            params![now.saturating_sub(self.rollup_secs) as i64],
        );
        let _ = tx.commit();
    }
}

fn median(mut v: Vec<f32>) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    Some(v[v.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Protocol, Target};

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
    fn stat_reports_loss_and_baseline() {
        let mut h = History::open(None, 15, 7);
        feed(&mut h, 0, 600, |_| Some(10.0));
        feed(&mut h, 600, 630, |t| (t % 2 == 0).then_some(80.0));
        let s = h.stat(&probe(), 30);
        assert!((s.loss - 0.5).abs() < 0.01, "{s:?}");
        assert_eq!(s.rtt_ms, Some(80.0));
        assert_eq!(s.baseline_ms, Some(10.0));
    }

    #[test]
    fn ram_is_bounded() {
        let mut h = History::open(None, 1, 7);
        feed(&mut h, 0, 500, |_| Some(1.0));
        assert!(h.recent(&Key::Probe(probe()), 1000).len() <= 61);
    }

    #[test]
    fn rollups_serve_ranges_older_than_ram() {
        let path = std::env::temp_dir().join(format!("bg-history-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut h = History::open(Some(&path), 1, 7);
        feed(&mut h, 6000, 6000 + 600, |t| {
            if t < 6060 {
                None
            } else {
                Some((t % 60) as f32)
            }
        });
        let pts = h.series(&Key::Probe(probe()), 6000, 6600);
        let first = pts.first().unwrap();
        assert_eq!(first.at, 6000);
        assert_eq!(first.loss, 1.0);
        let rolled = pts.iter().find(|p| p.at == 6120).unwrap();
        assert_eq!((rolled.min, rolled.max), (0.0, 59.0));
        assert!(pts.last().unwrap().at >= 6599);
        drop(h);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hops_truncate_at_destination() {
        let mut h = History::open(None, 15, 7);
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
