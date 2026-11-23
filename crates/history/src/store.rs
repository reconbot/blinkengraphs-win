//! Where History lives on disk: Raw results and Pauses, the Archives' Rows and Paused seconds,
//! and how far each Archive is written. Without a database the same calls keep it in memory.

use crate::archive::Row;
use crate::pause::Pause;
use crate::sketch::Sketch;
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Bumped when the tables change; an older file is dropped rather than migrated (#40: History
/// starts empty when it moves to Raw and Archives).
const SCHEMA: i64 = 2;

pub(crate) struct Store {
    db: Option<Connection>,
    /// In-memory Rows, `(archive, key) → m → Row`, when there is no database.
    mem: HashMap<(usize, String), BTreeMap<u64, Row>>,
    mem_paused: [BTreeMap<u64, f64>; 3],
    /// Arrival order of Raw results that share an `at`.
    seq: i64,
}

impl Store {
    pub fn open(path: Option<&Path>) -> Store {
        let db = path.and_then(|p| {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).ok()?;
            }
            let c = Connection::open(p).ok()?;
            let version: i64 = c
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .ok()?;
            if version < SCHEMA {
                c.execute_batch(
                    "DROP TABLE IF EXISTS minute; DROP TABLE IF EXISTS recent;
                     DROP TABLE IF EXISTS rollup; DROP TABLE IF EXISTS key;",
                )
                .ok()?;
            }
            // NORMAL: a commit survives a crash or kill; power loss costs at most the last few
            // (research 07 §5).
            c.execute_batch(&format!(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 CREATE TABLE IF NOT EXISTS key (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
                 CREATE TABLE IF NOT EXISTS raw (
                   key INTEGER NOT NULL, at INTEGER NOT NULL, seq INTEGER NOT NULL, v REAL,
                   PRIMARY KEY (key, at, seq)) WITHOUT ROWID;
                 CREATE TABLE IF NOT EXISTS pause (begin REAL PRIMARY KEY, end REAL NOT NULL);
                 CREATE TABLE IF NOT EXISTS row (
                   archive INTEGER NOT NULL, key INTEGER NOT NULL, m INTEGER NOT NULL,
                   n INTEGER NOT NULL, lost INTEGER NOT NULL,
                   sum REAL, min REAL, max REAL, sk BLOB NOT NULL,
                   PRIMARY KEY (archive, key, m)) WITHOUT ROWID;
                 CREATE TABLE IF NOT EXISTS paused (
                   archive INTEGER NOT NULL, m INTEGER NOT NULL, secs REAL NOT NULL,
                   PRIMARY KEY (archive, m)) WITHOUT ROWID;
                 CREATE TABLE IF NOT EXISTS cursor (archive INTEGER PRIMARY KEY, m INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS mark (one INTEGER PRIMARY KEY CHECK (one = 1), at INTEGER NOT NULL);
                 PRAGMA user_version = {SCHEMA};"
            ))
            .ok()?;
            Some(c)
        });
        let seq = db
            .as_ref()
            .and_then(|c| {
                c.query_row("SELECT COALESCE(MAX(seq), 0) FROM raw", [], |r| r.get(0))
                    .ok()
            })
            .unwrap_or(0);
        Store {
            db,
            mem: HashMap::new(),
            mem_paused: Default::default(),
            seq,
        }
    }

    pub fn persistent(&self) -> bool {
        self.db.is_some()
    }

    /// Raw results with `at > since`, by key, in `at` then arrival order.
    pub fn load_raw(&self, since: u64) -> Vec<(String, u64, Option<f32>)> {
        let Some(c) = &self.db else { return vec![] };
        c.prepare(
            "SELECT k.name, r.at, r.v FROM raw r JOIN key k ON k.id = r.key
             WHERE r.at > ?1 ORDER BY r.at, r.seq",
        )
        .and_then(|mut s| {
            s.query_map(params![since as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, Option<f64>>(2)?.map(|v| v as f32),
                ))
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default()
    }

    pub fn load_pauses(&self, since: f64) -> Vec<Pause> {
        let Some(c) = &self.db else { return vec![] };
        c.prepare("SELECT begin, end FROM pause WHERE end > ?1 ORDER BY begin")
            .and_then(|mut s| {
                s.query_map(params![since], |r| {
                    Ok(Pause {
                        begin: r.get(0)?,
                        end: r.get(1)?,
                    })
                })
                .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
    }

    /// Appends Raw results and records Pauses in one transaction, and drops Raw and Pauses
    /// older than `oldest`.
    /// Also records a mark at `now`: measuring went on, though nothing else may be in this
    /// write (spec `rules/history.md`, Raw). Only the newest mark matters, so one row holds it.
    pub fn write_raw(
        &mut self,
        results: &[(String, u64, Option<f32>)],
        pauses: &[Pause],
        oldest: u64,
        now: u64,
    ) {
        let Some(c) = &mut self.db else { return };
        let Ok(tx) = c.transaction() else { return };
        let _ = tx.execute(
            "INSERT OR REPLACE INTO mark VALUES (1, ?1)",
            params![now as i64],
        );
        for (key, at, v) in results {
            self.seq += 1;
            let _ = tx.execute("INSERT OR IGNORE INTO key (name) VALUES (?1)", params![key]);
            let _ = tx.execute(
                "INSERT OR IGNORE INTO raw SELECT id, ?2, ?3, ?4 FROM key WHERE name = ?1",
                params![key, *at as i64, self.seq, v.map(f64::from)],
            );
        }
        for p in pauses {
            let _ = tx.execute(
                "INSERT OR REPLACE INTO pause VALUES (?1, ?2)",
                params![p.begin, p.end],
            );
        }
        let _ = tx.execute("DELETE FROM raw WHERE at <= ?1", params![oldest as i64]);
        let _ = tx.execute("DELETE FROM pause WHERE end <= ?1", params![oldest as f64]);
        let _ = tx.commit();
    }

    /// The newest mark: when measuring was last known to go on.
    pub fn alive(&self) -> Option<u64> {
        let c = self.db.as_ref()?;
        c.query_row("SELECT at FROM mark WHERE one = 1", [], |r| {
            r.get::<_, i64>(0)
        })
        .ok()
        .map(|at| at as u64)
    }

    pub fn cursor(&self, archive: usize) -> Option<u64> {
        let c = self.db.as_ref()?;
        c.query_row(
            "SELECT m FROM cursor WHERE archive = ?1",
            params![archive as i64],
            |r| r.get::<_, i64>(0),
        )
        .ok()
        .map(|m| m as u64)
    }

    /// Writes one step of one Archive: its Rows, its Paused seconds, and the cursor past it.
    pub fn write_step(
        &mut self,
        archive: usize,
        m: u64,
        rows: &[(String, Row)],
        paused: f64,
        next: u64,
    ) {
        if let Some(c) = &mut self.db {
            let Ok(tx) = c.transaction() else { return };
            for (key, r) in rows {
                let _ = tx.execute("INSERT OR IGNORE INTO key (name) VALUES (?1)", params![key]);
                let some = |v: f64| (r.n > 0).then_some(v);
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO row SELECT ?1, id, ?3, ?4, ?5, ?6, ?7, ?8, ?9
                     FROM key WHERE name = ?2",
                    params![
                        archive as i64,
                        key,
                        m as i64,
                        r.n as i64,
                        r.lost as i64,
                        some(r.sum),
                        some(r.min),
                        some(r.max),
                        r.sketch.encode()
                    ],
                );
            }
            if paused > 0.0 {
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO paused VALUES (?1, ?2, ?3)",
                    params![archive as i64, m as i64, paused],
                );
            }
            let _ = tx.execute(
                "INSERT OR REPLACE INTO cursor VALUES (?1, ?2)",
                params![archive as i64, next as i64],
            );
            let _ = tx.commit();
        } else {
            for (key, r) in rows {
                self.mem
                    .entry((archive, key.clone()))
                    .or_default()
                    .insert(m, r.clone());
            }
            if paused > 0.0 {
                self.mem_paused[archive].insert(m, paused);
            }
        }
    }

    /// One Archive's Rows in `[from, to)`, by key, each ascending.
    pub fn rows_in(&self, archive: usize, from: u64, to: u64) -> BTreeMap<String, Vec<(u64, Row)>> {
        let mut out: BTreeMap<String, Vec<(u64, Row)>> = BTreeMap::new();
        match &self.db {
            Some(c) => {
                let rows = c
                    .prepare_cached(
                        "SELECT k.name, r.m, r.n, r.lost, r.sum, r.min, r.max, r.sk FROM row r
                         JOIN key k ON k.id = r.key
                         WHERE r.archive = ?1 AND r.m >= ?2 AND r.m < ?3 ORDER BY r.m",
                    )
                    .and_then(|mut s| {
                        s.query_map(params![archive as i64, from as i64, to as i64], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, i64>(1)? as u64,
                                row(r, 2)?,
                            ))
                        })
                        .map(|rows| rows.flatten().collect::<Vec<_>>())
                    })
                    .unwrap_or_default();
                for (k, m, r) in rows {
                    out.entry(k).or_default().push((m, r));
                }
            }
            None => {
                for ((a, k), rows) in &self.mem {
                    if *a == archive {
                        let v: Vec<_> =
                            rows.range(from..to).map(|(m, r)| (*m, r.clone())).collect();
                        if !v.is_empty() {
                            out.insert(k.clone(), v);
                        }
                    }
                }
            }
        }
        out
    }

    /// One key's Rows in one Archive from `from` on, ascending.
    pub fn rows(&self, archive: usize, key: &str, from: u64) -> Vec<(u64, Row)> {
        match &self.db {
            Some(c) => c
                .prepare_cached(
                    "SELECT r.m, r.n, r.lost, r.sum, r.min, r.max, r.sk FROM row r
                     JOIN key k ON k.id = r.key
                     WHERE r.archive = ?1 AND k.name = ?2 AND r.m >= ?3 ORDER BY r.m",
                )
                .and_then(|mut s| {
                    s.query_map(params![archive as i64, key, from as i64], |r| {
                        Ok((r.get::<_, i64>(0)? as u64, row(r, 1)?))
                    })
                    .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default(),
            None => self
                .mem
                .get(&(archive, key.to_string()))
                .map(|rows| rows.range(from..).map(|(m, r)| (*m, r.clone())).collect())
                .unwrap_or_default(),
        }
    }

    /// One Archive's Paused seconds per step from `from` on, ascending.
    pub fn paused(&self, archive: usize, from: u64) -> Vec<(u64, f64)> {
        match &self.db {
            Some(c) => c
                .prepare_cached(
                    "SELECT m, secs FROM paused WHERE archive = ?1 AND m >= ?2 ORDER BY m",
                )
                .and_then(|mut s| {
                    s.query_map(params![archive as i64, from as i64], |r| {
                        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, f64>(1)?))
                    })
                    .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default(),
            None => self.mem_paused[archive]
                .range(from..)
                .map(|(m, s)| (*m, *s))
                .collect(),
        }
    }

    /// The first step at or after `from` that has a Row or Paused seconds in `archive`.
    pub fn next_step(&self, archive: usize, from: u64) -> Option<u64> {
        match &self.db {
            Some(c) => c
                .query_row(
                    "SELECT MIN(m) FROM (SELECT MIN(m) AS m FROM row WHERE archive = ?1 AND m >= ?2
                     UNION ALL SELECT MIN(m) FROM paused WHERE archive = ?1 AND m >= ?2)",
                    params![archive as i64, from as i64],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .ok()
                .flatten()
                .map(|m| m as u64),
            None => {
                let rows = self
                    .mem
                    .iter()
                    .filter(|((a, _), _)| *a == archive)
                    .filter_map(|(_, rows)| rows.range(from..).next().map(|r| *r.0));
                let paused = self.mem_paused[archive].range(from..).next().map(|p| *p.0);
                rows.chain(paused).min()
            }
        }
    }

    /// Drops an Archive's Rows and Paused seconds that start before `before`.
    pub fn retain(&mut self, archive: usize, before: u64) {
        match &self.db {
            Some(c) => {
                let _ = c.execute(
                    "DELETE FROM row WHERE archive = ?1 AND m < ?2",
                    params![archive as i64, before as i64],
                );
                let _ = c.execute(
                    "DELETE FROM paused WHERE archive = ?1 AND m < ?2",
                    params![archive as i64, before as i64],
                );
            }
            None => {
                for ((a, _), rows) in self.mem.iter_mut() {
                    if *a == archive {
                        *rows = rows.split_off(&before);
                    }
                }
                self.mem_paused[archive] = self.mem_paused[archive].split_off(&before);
            }
        }
    }

    /// The oldest step any of an Archive's Rows starts at.
    pub fn oldest(&self, archive: usize) -> Option<u64> {
        match &self.db {
            Some(c) => c
                .query_row(
                    "SELECT MIN(m) FROM row WHERE archive = ?1",
                    params![archive as i64],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .ok()
                .flatten()
                .map(|m| m as u64),
            None => self
                .mem
                .iter()
                .filter(|((a, _), _)| *a == archive)
                .filter_map(|(_, rows)| rows.keys().next().copied())
                .min(),
        }
    }

    /// Bytes in use and Rows held, across every Archive.
    pub fn measure(&self) -> Option<(u64, u64)> {
        let c = self.db.as_ref()?;
        let one = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64;
        Some((
            one("PRAGMA page_count").saturating_sub(one("PRAGMA freelist_count"))
                * one("PRAGMA page_size"),
            one("SELECT COUNT(*) FROM row"),
        ))
    }
}

/// A Row from columns `n, lost, sum, min, max, sk` starting at `i`.
pub(crate) fn row(r: &rusqlite::Row, i: usize) -> rusqlite::Result<Row> {
    let sk: Vec<u8> = r.get(i + 5)?;
    Ok(Row {
        n: r.get::<_, i64>(i)? as u64,
        lost: r.get::<_, i64>(i + 1)? as u64,
        sum: r.get::<_, Option<f64>>(i + 2)?.unwrap_or(0.0),
        min: r.get::<_, Option<f64>>(i + 3)?.unwrap_or(f64::INFINITY),
        max: r.get::<_, Option<f64>>(i + 4)?.unwrap_or(f64::NEG_INFINITY),
        sketch: Sketch::decode(&sk).unwrap_or_default(),
    })
}
