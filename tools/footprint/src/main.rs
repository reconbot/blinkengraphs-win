//! Usage: footprint [seconds=60] [exe]
//! Runs the engine with default config plus Meter and Strip drawing each Sample, then prints a markdown
//! report of CPU share and memory. The Panel window is excluded: it only exists while open.

use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);
    let exe = args.next();

    let config = config::Config::default();
    let engine = engine::Engine::start(config.clone(), None, || {}, Box::new(|| false));
    let start = Instant::now();
    let cpu0 = os::cpu_time();
    let mut drawn = 0u64;
    let mut last = u64::MAX;
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(100));
        let s = engine.shared.lock().unwrap();
        if s.sample_count == last {
            continue;
        }
        last = s.sample_count;
        for section in model::Section::ALL {
            let c = config.section(section);
            std::hint::black_box(meters::draw(
                section,
                &s.history,
                &config,
                &c.tray,
                24,
                render::Taskbar::Dark,
                model::now(),
            ));
        }
        std::hint::black_box(meters::strip(
            &s.history,
            &config,
            40,
            1.0,
            None,
            render::Taskbar::Dark,
            model::now(),
        ));
        drawn += 1;
    }
    let wall = start.elapsed().as_secs_f64();
    let cpu = os::cpu_time() - cpu0;
    let (ws, peak_ws, private) = os::memory();
    let mb = |b: u64| b as f64 / 1_048_576.0;

    println!("| Measure | Value |\n|---|---|");
    println!("| Run | {wall:.0} s, {drawn} Samples drawn |");
    println!("| CPU (share of one core) | {:.2}% |", cpu / wall * 100.0);
    println!(
        "| Working set (now / peak) | {:.1} MB / {:.1} MB |",
        mb(ws),
        mb(peak_ws)
    );
    println!("| Private bytes | {:.1} MB |", mb(private));
    if let Some(path) = exe
        && let Ok(m) = std::fs::metadata(&path)
    {
        println!("| `{path}` size | {:.2} MB |", mb(m.len()));
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };

    pub fn cpu_time() -> f64 {
        let z = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
        let secs =
            |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 1e7;
        secs(k) + secs(u)
    }

    pub fn memory() -> (u64, u64, u64) {
        let mut m: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        m.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                m.cb,
            )
        };
        (
            m.WorkingSetSize as u64,
            m.PeakWorkingSetSize as u64,
            m.PrivateUsage as u64,
        )
    }
}

#[cfg(not(windows))]
mod os {
    pub fn cpu_time() -> f64 {
        0.0
    }

    pub fn memory() -> (u64, u64, u64) {
        (0, 0, 0)
    }
}
