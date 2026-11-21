//! Samples every Metric and takes Process Snapshots. Windows only; elsewhere every value is zero.

mod calc;

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
use stub as imp;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as imp;

use model::{ProcessRow, Sample};

/// Which adapter is the Followed Adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterChoice {
    FollowDefaultRoute,
    /// Interface alias, e.g. "Wi-Fi".
    Pinned(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterInfo {
    pub alias: String,
    pub description: String,
    pub carries_default_route: bool,
}

pub struct Collector {
    inner: imp::Collector,
}

impl Collector {
    pub fn new(adapter: AdapterChoice) -> std::io::Result<Self> {
        Ok(Self {
            inner: imp::Collector::new(adapter)?,
        })
    }

    /// The next Sample's network rates are zero.
    pub fn set_adapter(&mut self, adapter: AdapterChoice) {
        self.inner.set_adapter(adapter);
    }

    pub fn adapters(&self) -> Vec<AdapterInfo> {
        self.inner.adapters()
    }

    /// All Metric values; rates over the time since the previous call.
    pub fn sample(&mut self) -> Sample {
        self.inner.sample()
    }

    /// Top `limit` processes by CPU; only called while the Panel is open (ADR 0003).
    /// CPU and disk rates are over the time since the previous call, so the first call reports zero CPU.
    pub fn processes(&mut self, limit: usize) -> Vec<ProcessRow> {
        self.inner.processes(limit)
    }
}

#[cfg(all(test, windows))]
mod smoke {
    use super::*;
    use model::Metric;

    #[test]
    fn samples_and_processes() {
        let mut c = Collector::new(AdapterChoice::FollowDefaultRoute).unwrap();
        c.processes(5);
        c.sample();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let s = c.sample();
        let cpu = s.get(Metric::CpuTotal);
        let commit = s.get(Metric::MemCommit);
        assert!((0.0..=100.0).contains(&cpu), "CpuTotal {cpu}");
        assert!((0.0..=100.0).contains(&commit), "MemCommit {commit}");
        assert!(commit > 0.0, "MemCommit {commit}");
        assert!(!s.per_core.is_empty());
        let procs = c.processes(5);
        assert!(!procs.is_empty());
        assert!(procs.len() <= 5);
        assert!(!c.adapters().is_empty());
        eprintln!("{s:?}\n{procs:#?}\n{:#?}", c.adapters());
    }
}
