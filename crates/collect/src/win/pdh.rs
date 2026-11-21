use super::{from_wide_ptr, to_wide};
use crate::calc;
use model::{Metric, Sample};
use std::io;
use std::ptr::{null, null_mut};
use windows_sys::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW,
};

// English paths so they resolve on every display language.
const COUNTERS: [(Metric, &str); 10] = [
    (Metric::CpuTotal, r"\Processor(_Total)\% Processor Time"),
    (Metric::CpuKernel, r"\Processor(_Total)\% Privileged Time"),
    (Metric::CpuQueue, r"\System\Processor Queue Length"),
    (Metric::MemCommit, r"\Memory\% Committed Bytes In Use"),
    (Metric::MemHardFaults, r"\Memory\Pages Input/sec"),
    (Metric::DiskActive, r"\PhysicalDisk(_Total)\% Idle Time"),
    (
        Metric::DiskRead,
        r"\PhysicalDisk(_Total)\Disk Read Bytes/sec",
    ),
    (
        Metric::DiskWrite,
        r"\PhysicalDisk(_Total)\Disk Write Bytes/sec",
    ),
    (
        Metric::DiskResponse,
        r"\PhysicalDisk(_Total)\Avg. Disk sec/Transfer",
    ),
    (
        Metric::DiskQueue,
        r"\PhysicalDisk(_Total)\Current Disk Queue Length",
    ),
];
const PER_CORE: &str = r"\Processor(*)\% Processor Time";

pub struct Query {
    query: PDH_HQUERY,
    counters: Vec<(Metric, PDH_HCOUNTER)>,
    per_core: Option<PDH_HCOUNTER>,
    items: Vec<u64>,
}

// PDH handles are not bound to the thread that opened them.
unsafe impl Send for Query {}

impl Query {
    pub fn open() -> io::Result<Self> {
        let mut query = null_mut();
        check(
            unsafe { PdhOpenQueryW(null(), 0, &mut query) },
            "PdhOpenQueryW",
        )?;
        let mut q = Self {
            query,
            counters: Vec::with_capacity(COUNTERS.len()),
            per_core: None,
            items: Vec::new(),
        };
        for (metric, path) in COUNTERS {
            match q.add(path) {
                Ok(h) => q.counters.push((metric, h)),
                // Without CPU total the counter set is broken (`lodctr /r` repairs it); report it.
                Err(e) if metric == Metric::CpuTotal => return Err(e),
                // Disk counters can be switched off (`diskperf -n`); that Metric reads zero.
                Err(_) => {}
            }
        }
        q.per_core = q.add(PER_CORE).ok();
        Ok(q)
    }

    fn add(&self, path: &str) -> io::Result<PDH_HCOUNTER> {
        let wide = to_wide(path);
        let mut h = null_mut();
        check(
            unsafe { PdhAddEnglishCounterW(self.query, wide.as_ptr(), 0, &mut h) },
            path,
        )?;
        Ok(h)
    }

    pub fn read(&mut self, s: &mut Sample) {
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return;
        }
        for &(metric, h) in &self.counters {
            s.set(metric, calc::from_pdh(metric, value(h)));
        }
        if let Some(h) = self.per_core {
            s.per_core = self.cores(h);
        }
    }

    fn cores(&mut self, h: PDH_HCOUNTER) -> Vec<f32> {
        // Item names point into the same buffer, after the items.
        let mut bytes = (self.items.len() * 8) as u32;
        for _ in 0..3 {
            let mut count = 0;
            let buf = if self.items.is_empty() {
                null_mut()
            } else {
                self.items.as_mut_ptr().cast()
            };
            match unsafe {
                PdhGetFormattedCounterArrayW(h, PDH_FMT_DOUBLE, &mut bytes, &mut count, buf)
            } {
                0 => {
                    let items = unsafe {
                        std::slice::from_raw_parts(
                            self.items.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                            count as usize,
                        )
                    };
                    return calc::order_cores(items.iter().map(|i| {
                        let v = if valid(i.FmtValue.CStatus) {
                            unsafe { i.FmtValue.Anonymous.doubleValue }
                        } else {
                            0.0
                        };
                        (unsafe { from_wide_ptr(i.szName) }, v)
                    }));
                }
                PDH_MORE_DATA => self.items.resize((bytes as usize).div_ceil(8), 0),
                _ => return Vec::new(),
            }
        }
        Vec::new()
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

fn value(h: PDH_HCOUNTER) -> f64 {
    let mut v = PDH_FMT_COUNTERVALUE::default();
    let status = unsafe { PdhGetFormattedCounterValue(h, PDH_FMT_DOUBLE, null_mut(), &mut v) };
    if status != 0 || !valid(v.CStatus) {
        return 0.0;
    }
    unsafe { v.Anonymous.doubleValue }
}

fn valid(cstatus: u32) -> bool {
    cstatus == PDH_CSTATUS_VALID_DATA || cstatus == PDH_CSTATUS_NEW_DATA
}

fn check(status: u32, what: &str) -> io::Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{what}: PDH status 0x{status:08X}"
        )))
    }
}
