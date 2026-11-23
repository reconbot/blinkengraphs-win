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
        let mut s = self.inner.sample();
        s.pressure = model::Pressure::from_commit(s.get(model::Metric::MemCommit));
        s
    }

    /// Top `limit` processes by CPU plus top `limit` by memory; only called while the Panel is open (ADR 0003).
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
        let cpu = s.get(Metric::CpuBusy);
        let commit = s.get(Metric::MemCommit);
        assert!((0.0..=100.0).contains(&cpu), "CpuBusy {cpu}");
        assert!((0.0..=100.0).contains(&commit), "MemCommit {commit}");
        assert!(commit > 0.0, "MemCommit {commit}");
        assert!(!s.per_core.is_empty());
        let procs = c.processes(5);
        assert!(!procs.is_empty());
        assert!(procs.len() <= 10);
        assert!(!c.adapters().is_empty());
        eprintln!("{s:?}\n{procs:#?}\n{:#?}", c.adapters());
    }
}

/// Seconds the machine has spent asleep since boot: interrupt time counts sleep and the
/// unbiased one doesn't (research 07 §1). A jump between two readings is a suspend that sent
/// no notice, such as a critical one. 0 off Windows.
pub fn slept_s() -> f64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::WindowsProgramming::{
            QueryInterruptTime, QueryUnbiasedInterruptTime,
        };
        let (mut biased, mut unbiased) = (0u64, 0u64);
        // Unbiased first: interrupt time read after it can only be later, never less asleep.
        unsafe {
            QueryUnbiasedInterruptTime(&mut unbiased);
            QueryInterruptTime(&mut biased);
        }
        // Both count 100 ns units.
        biased.saturating_sub(unbiased) as f64 / 1e7
    }
    #[cfg(not(windows))]
    0.0
}
