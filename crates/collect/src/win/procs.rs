use crate::calc::{ProcCounters, ProcTracker};
use model::ProcessRow;
use std::mem::offset_of;
use std::time::Instant;
use windows_sys::Wdk::System::SystemInformation::{
    NtQuerySystemInformation, SystemProcessInformation,
};
use windows_sys::Win32::Foundation::{
    FILETIME, HANDLE, STATUS_INFO_LENGTH_MISMATCH, UNICODE_STRING,
};
use windows_sys::Win32::System::Threading::GetSystemTimes;
use windows_sys::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION;

/// SYSTEM_PROCESS_INFORMATION with the fields windows-sys leaves as Reserved, 64-bit layout.
/// https://github.com/winsiderss/phnt/blob/master/ntexapi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessInfo {
    next_entry_offset: u32,
    _number_of_threads: u32,
    /// Task Manager's "Memory" column.
    working_set_private_size: i64,
    _hard_faults_threads_cycles: [u64; 2],
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UNICODE_STRING,
    _base_priority: i32,
    unique_process_id: HANDLE,
    /// InheritedFromUniqueProcessId through PrivatePageCount.
    _middle: [usize; 15],
    _operation_counts: [i64; 3],
    read_transfer_count: i64,
    write_transfer_count: i64,
    _other_transfer_count: i64,
}

const _: () = {
    type Spi = SYSTEM_PROCESS_INFORMATION;
    assert!(size_of::<ProcessInfo>() == size_of::<Spi>());
    assert!(offset_of!(ProcessInfo, image_name) == offset_of!(Spi, ImageName));
    assert!(offset_of!(ProcessInfo, unique_process_id) == offset_of!(Spi, UniqueProcessId));
    assert!(offset_of!(ProcessInfo, _middle) + 14 * 8 == offset_of!(Spi, PrivatePageCount));
    assert!(offset_of!(ProcessInfo, _operation_counts) == offset_of!(Spi, Reserved7));
};

#[derive(Default)]
pub struct Snapshotter {
    buf: Vec<u64>,
    tracker: ProcTracker,
    last: Option<Instant>,
}

impl Snapshotter {
    pub fn top(&mut self, limit: usize) -> Vec<ProcessRow> {
        let now = Instant::now();
        let secs = self
            .last
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last = Some(now);
        let system_time = system_time();
        if !self.query() {
            return Vec::new();
        }
        let counters = self.counters();
        self.tracker.update(counters, system_time, secs, limit)
    }

    fn query(&mut self) -> bool {
        if self.buf.is_empty() {
            self.buf.resize(64 * 1024, 0);
        }
        for _ in 0..5 {
            let mut needed = 0u32;
            let status = unsafe {
                NtQuerySystemInformation(
                    SystemProcessInformation,
                    self.buf.as_mut_ptr().cast(),
                    (self.buf.len() * 8) as u32,
                    &mut needed,
                )
            };
            if status >= 0 {
                return true;
            }
            if status != STATUS_INFO_LENGTH_MISMATCH {
                return false;
            }
            // Processes can start between the size report and the retry.
            self.buf
                .resize((needed as usize + 64 * 1024).div_ceil(8), 0);
        }
        false
    }

    fn counters(&self) -> Vec<ProcCounters> {
        let base = self.buf.as_ptr().cast::<u8>();
        let len = self.buf.len() * 8;
        let mut out = Vec::with_capacity(512);
        let mut offset = 0;
        while offset + size_of::<ProcessInfo>() <= len {
            let p = unsafe { base.add(offset).cast::<ProcessInfo>().read_unaligned() };
            let pid = p.unique_process_id as usize as u32;
            // pid 0 is the Idle process: its CPU time is idle time.
            if pid != 0 {
                out.push(ProcCounters {
                    pid,
                    created: p.create_time,
                    name: image_name(&p.image_name),
                    cpu_time: p.user_time.saturating_add(p.kernel_time).max(0) as u64,
                    io_bytes: p
                        .read_transfer_count
                        .saturating_add(p.write_transfer_count)
                        .max(0) as u64,
                    memory_bytes: p.working_set_private_size.max(0) as u64,
                });
            }
            if p.next_entry_offset == 0 {
                break;
            }
            offset += p.next_entry_offset as usize;
        }
        out
    }
}

fn image_name(s: &UNICODE_STRING) -> String {
    if s.Buffer.is_null() {
        return String::new();
    }
    // Length is in bytes; the Buffer points into the snapshot buffer, which is still borrowed.
    let chars = unsafe { std::slice::from_raw_parts(s.Buffer, s.Length as usize / 2) };
    String::from_utf16_lossy(chars)
}

/// Kernel time includes idle, so this is all processors' elapsed time.
fn system_time() -> u64 {
    let (mut idle, mut kernel, mut user) = Default::default();
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
        return 0;
    }
    filetime(kernel).saturating_add(filetime(user))
}

fn filetime(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}
