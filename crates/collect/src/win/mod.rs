mod net;
mod pdh;
mod procs;

use crate::calc::NetTracker;
use crate::{AdapterChoice, AdapterInfo};
use model::{Metric, ProcessRow, Sample};
use std::time::Instant;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

pub struct Collector {
    pdh: pdh::Query,
    adapter: AdapterChoice,
    net: NetTracker,
    last_sample: Instant,
    procs: procs::Snapshotter,
}

impl Collector {
    pub fn new(adapter: AdapterChoice) -> std::io::Result<Self> {
        let mut c = Self {
            pdh: pdh::Query::open()?,
            adapter,
            net: NetTracker::default(),
            last_sample: Instant::now(),
            procs: procs::Snapshotter::default(),
        };
        // PDH rate counters and network deltas need a previous reading, so the caller's first sample has one.
        c.sample();
        Ok(c)
    }

    pub fn set_adapter(&mut self, adapter: AdapterChoice) {
        self.adapter = adapter;
        self.net.reset();
    }

    pub fn adapters(&self) -> Vec<AdapterInfo> {
        net::adapters()
    }

    pub fn sample(&mut self) -> Sample {
        let now = Instant::now();
        let secs = now.duration_since(self.last_sample).as_secs_f64();
        self.last_sample = now;

        let mut s = Sample {
            at: model::now(),
            ..Sample::default()
        };
        self.pdh.read(&mut s);
        s.set(Metric::MemPhysical, physical_in_use());
        let rates = self.net.update(net::followed(&self.adapter), secs);
        s.set(Metric::NetDown, rates.down);
        s.set(Metric::NetUp, rates.up);
        s.set(Metric::NetErrors, rates.errors);
        s
    }

    pub fn processes(&mut self, limit: usize) -> Vec<ProcessRow> {
        self.procs.top(limit)
    }
}

fn physical_in_use() -> f64 {
    // dwLength must be set or the call fails.
    let mut m = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 || m.ullTotalPhys == 0 {
        return 0.0;
    }
    m.ullTotalPhys.saturating_sub(m.ullAvailPhys) as f64 / m.ullTotalPhys as f64 * 100.0
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn from_wide(s: &[u16]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

/// # Safety
/// `p` is null or points to a nul-terminated UTF-16 string.
unsafe fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}
