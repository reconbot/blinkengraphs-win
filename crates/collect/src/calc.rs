#![cfg_attr(not(windows), allow(dead_code))]

use model::{Metric, ProcessRow};
use std::collections::HashMap;

/// A decrease means the counter was reset (adapter restarted, process counters are 64-bit), so it counts as no traffic.
pub fn counter_delta(prev: u64, now: u64) -> u64 {
    now.saturating_sub(prev)
}

pub fn per_sec(delta: u64, secs: f64) -> f64 {
    if secs > 0.0 { delta as f64 / secs } else { 0.0 }
}

pub fn share_pct(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        return 0.0;
    }
    (part as f64 / whole as f64 * 100.0).clamp(0.0, 100.0) as f32
}

/// Turns a raw PDH value into the Metric's unit.
pub fn from_pdh(metric: Metric, raw: f64) -> f64 {
    if !raw.is_finite() {
        return 0.0;
    }
    match metric {
        Metric::DiskActive => (100.0 - raw).clamp(0.0, 100.0),
        Metric::DiskResponse => (raw * 1000.0).max(0.0),
        Metric::CpuTotal | Metric::CpuKernel | Metric::MemCommit => raw.clamp(0.0, 100.0),
        _ => raw.max(0.0),
    }
}

/// `\Processor(*)` instances are "0", "1", … plus "_Total"; PDH does not return them in index order.
pub fn order_cores(instances: impl IntoIterator<Item = (String, f64)>) -> Vec<f32> {
    let mut cores: Vec<(usize, f32)> = instances
        .into_iter()
        .filter_map(|(name, v)| {
            let i = name.parse().ok()?;
            Some((i, from_pdh(Metric::CpuTotal, v) as f32))
        })
        .collect();
    cores.sort_by_key(|&(i, _)| i);
    cores.into_iter().map(|(_, v)| v).collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetCounters {
    pub rx: u64,
    pub tx: u64,
    pub errors: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetRates {
    pub down: f64,
    pub up: f64,
    pub errors: f64,
}

/// Rates for the Followed Adapter; the first Sample after the adapter changes is zero.
#[derive(Default)]
pub struct NetTracker {
    last: Option<(u64, NetCounters)>,
}

impl NetTracker {
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// `now` is the adapter's LUID and counters, `None` when there is no Followed Adapter.
    pub fn update(&mut self, now: Option<(u64, NetCounters)>, secs: f64) -> NetRates {
        let rates = match (self.last, now) {
            (Some((was, p)), Some((is, c))) if was == is => NetRates {
                down: per_sec(counter_delta(p.rx, c.rx), secs),
                up: per_sec(counter_delta(p.tx, c.tx), secs),
                errors: per_sec(counter_delta(p.errors, c.errors), secs),
            },
            _ => NetRates::default(),
        };
        self.last = now;
        rates
    }
}

pub struct ProcCounters {
    pub pid: u32,
    /// Distinguishes a reused pid.
    pub created: i64,
    pub name: String,
    /// User + kernel, 100 ns units.
    pub cpu_time: u64,
    pub io_bytes: u64,
    pub memory_bytes: u64,
}

#[derive(Default)]
pub struct ProcTracker {
    prev: HashMap<(u32, i64), (u64, u64)>,
    prev_system: Option<u64>,
}

impl ProcTracker {
    /// `system_time` is CPU time of all processors including idle, 100 ns units, so CPU% matches Task Manager.
    pub fn update(
        &mut self,
        counters: Vec<ProcCounters>,
        system_time: u64,
        secs: f64,
        limit: usize,
    ) -> Vec<ProcessRow> {
        let system_delta = self
            .prev_system
            .map_or(0, |p| counter_delta(p, system_time));
        self.prev_system = Some(system_time);
        let mut next = HashMap::with_capacity(counters.len());
        let mut rows: Vec<ProcessRow> = counters
            .into_iter()
            .map(|r| {
                let key = (r.pid, r.created);
                let (cpu_pct, disk_bps) = match self.prev.get(&key) {
                    Some(&(cpu, io)) => (
                        share_pct(counter_delta(cpu, r.cpu_time), system_delta),
                        per_sec(counter_delta(io, r.io_bytes), secs),
                    ),
                    None => (0.0, 0.0),
                };
                next.insert(key, (r.cpu_time, r.io_bytes));
                ProcessRow {
                    name: r.name,
                    pid: r.pid,
                    cpu_pct,
                    memory_bytes: r.memory_bytes,
                    disk_bps,
                }
            })
            .collect();
        self.prev = next;
        rows.sort_by(|a, b| {
            b.cpu_pct
                .total_cmp(&a.cpu_pct)
                .then(b.memory_bytes.cmp(&a.memory_bytes))
        });
        // Union of the top `limit` by CPU and by memory, so a large idle process still reaches the Panel.
        let mut keep: Vec<ProcessRow> = rows.iter().take(limit).cloned().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.memory_bytes));
        for r in rows.into_iter().take(limit) {
            if !keep.iter().any(|k| k.pid == r.pid) {
                keep.push(r);
            }
        }
        keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_of_reset_counter_is_zero() {
        assert_eq!(counter_delta(100, 250), 150);
        assert_eq!(counter_delta(250, 100), 0);
        assert_eq!(counter_delta(u64::MAX - 1, u64::MAX), 1);
    }

    #[test]
    fn rate_needs_elapsed_time() {
        assert_eq!(per_sec(1000, 0.5), 2000.0);
        assert_eq!(per_sec(1000, 0.0), 0.0);
        assert_eq!(per_sec(1000, -1.0), 0.0);
    }

    #[test]
    fn share_is_clamped_percent() {
        assert_eq!(share_pct(25, 100), 25.0);
        assert_eq!(share_pct(5, 0), 0.0);
        assert_eq!(share_pct(300, 100), 100.0);
    }

    #[test]
    fn pdh_values_become_metric_units() {
        assert_eq!(from_pdh(Metric::DiskActive, 70.0), 30.0);
        assert_eq!(from_pdh(Metric::DiskActive, 104.0), 0.0);
        assert_eq!(from_pdh(Metric::DiskActive, -3.0), 100.0);
        assert_eq!(from_pdh(Metric::DiskResponse, 0.004), 4.0);
        assert_eq!(from_pdh(Metric::CpuTotal, 100.5), 100.0);
        assert_eq!(from_pdh(Metric::DiskRead, f64::NAN), 0.0);
        assert_eq!(from_pdh(Metric::CpuQueue, 3.0), 3.0);
    }

    #[test]
    fn cores_in_index_order_without_total() {
        let got = order_cores([
            ("10".to_string(), 10.0),
            ("_Total".to_string(), 50.0),
            ("2".to_string(), 2.0),
            ("0".to_string(), 0.5),
        ]);
        assert_eq!(got, vec![0.5, 2.0, 10.0]);
    }

    fn net(rx: u64, tx: u64, errors: u64) -> NetCounters {
        NetCounters { rx, tx, errors }
    }

    #[test]
    fn net_rates_over_elapsed_time() {
        let mut t = NetTracker::default();
        assert_eq!(t.update(Some((1, net(0, 0, 0))), 1.0), NetRates::default());
        let r = t.update(Some((1, net(2000, 500, 4))), 2.0);
        assert_eq!(
            r,
            NetRates {
                down: 1000.0,
                up: 250.0,
                errors: 2.0
            }
        );
    }

    #[test]
    fn net_adapter_change_and_reset_read_zero() {
        let mut t = NetTracker::default();
        t.update(Some((1, net(1000, 1000, 0))), 1.0);
        assert_eq!(
            t.update(Some((2, net(5000, 5000, 0))), 1.0),
            NetRates::default()
        );
        assert_eq!(
            t.update(Some((2, net(10, 10, 0))), 1.0),
            NetRates::default()
        );
        assert_eq!(t.update(Some((2, net(20, 30, 0))), 1.0).up, 20.0);
        assert_eq!(t.update(None, 1.0), NetRates::default());
        assert_eq!(
            t.update(Some((2, net(40, 40, 0))), 1.0),
            NetRates::default()
        );
        t.reset();
        assert_eq!(
            t.update(Some((2, net(90, 90, 0))), 1.0),
            NetRates::default()
        );
    }

    fn proc(
        pid: u32,
        created: i64,
        cpu_time: u64,
        io_bytes: u64,
        memory_bytes: u64,
    ) -> ProcCounters {
        ProcCounters {
            pid,
            created,
            name: format!("p{pid}"),
            cpu_time,
            io_bytes,
            memory_bytes,
        }
    }

    #[test]
    fn processes_first_call_has_no_cpu_and_sorts_by_memory() {
        let mut t = ProcTracker::default();
        let rows = t.update(
            vec![proc(1, 0, 500, 0, 10), proc(2, 0, 900, 0, 20)],
            1000,
            1.0,
            5,
        );
        assert_eq!(rows.iter().map(|r| r.pid).collect::<Vec<_>>(), [2, 1]);
        assert!(rows.iter().all(|r| r.cpu_pct == 0.0 && r.disk_bps == 0.0));
    }

    #[test]
    fn processes_keep_top_memory_users_even_when_idle() {
        let mut t = ProcTracker::default();
        t.update(
            vec![
                proc(1, 0, 0, 0, 10),
                proc(2, 0, 0, 0, 10),
                proc(3, 0, 0, 0, 9_000),
            ],
            0,
            1.0,
            1,
        );
        let rows = t.update(
            vec![
                proc(1, 0, 500, 0, 10),
                proc(2, 0, 100, 0, 10),
                proc(3, 0, 0, 0, 9_000),
            ],
            1000,
            1.0,
            1,
        );
        assert_eq!(rows.iter().map(|r| r.pid).collect::<Vec<_>>(), [1, 3]);
    }

    #[test]
    fn processes_cpu_share_of_all_processors() {
        let mut t = ProcTracker::default();
        t.update(vec![proc(1, 0, 0, 0, 0), proc(2, 0, 0, 100, 0)], 0, 1.0, 5);
        let rows = t.update(
            vec![
                proc(1, 0, 100, 0, 0),
                proc(2, 0, 600, 4100, 0),
                proc(3, 0, 50, 0, 0),
            ],
            1000,
            2.0,
            2,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].pid, rows[0].cpu_pct, rows[0].disk_bps),
            (2, 60.0, 2000.0)
        );
        assert_eq!((rows[1].pid, rows[1].cpu_pct), (1, 10.0));
    }

    #[test]
    fn processes_reused_pid_starts_over() {
        let mut t = ProcTracker::default();
        t.update(vec![proc(7, 1, 0, 0, 0)], 0, 1.0, 5);
        let rows = t.update(vec![proc(7, 2, 900, 900, 0)], 1000, 1.0, 5);
        assert_eq!((rows[0].cpu_pct, rows[0].disk_bps), (0.0, 0.0));
    }
}
