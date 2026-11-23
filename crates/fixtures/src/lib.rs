/// Deterministic synthetic histories, normalized to 0..=1, one sample per second.
pub struct Scenario {
    pub name: &'static str,
    pub caption: &'static str,
    pub cpu: Vec<f32>,
    pub memory: Vec<f32>,
    pub disk: Vec<f32>,
    pub down: Vec<f32>,
    pub up: Vec<f32>,
    pub rtt: Vec<Option<f32>>,
}

pub const LEN: usize = 120;

pub fn all() -> Vec<Scenario> {
    vec![idle(), cpu_pinned(), isp_loss(), upload_saturation()]
}

fn idle() -> Scenario {
    let mut r = Lcg(1);
    Scenario {
        name: "idle",
        caption: "Idle",
        cpu: series(|_| 0.05 + 0.08 * r.next()),
        memory: series(|_| 0.42),
        disk: series(|i| if i % 37 == 0 { 0.4 } else { 0.02 }),
        down: series(|_| 0.1 + 0.1 * r.next()),
        up: series(|_| 0.05 + 0.05 * r.next()),
        rtt: series(|_| Some(0.1 + 0.03 * r.next())),
    }
}

fn cpu_pinned() -> Scenario {
    let mut r = Lcg(2);
    Scenario {
        name: "cpu-pinned",
        caption: "CPU pinned, memory pressure rising",
        cpu: series(|i| {
            if i > 60 {
                0.9 + 0.1 * r.next()
            } else {
                0.1 + 0.1 * r.next()
            }
        }),
        memory: series(|i| 0.5 + 0.4 * (i as f32 / LEN as f32)),
        disk: series(|i| if i > 70 { 0.65 + 0.2 * r.next() } else { 0.05 }),
        down: series(|_| 0.1 * r.next()),
        up: series(|_| 0.05 * r.next()),
        rtt: series(|_| Some(0.1 + 0.05 * r.next())),
    }
}

fn isp_loss() -> Scenario {
    let mut r = Lcg(3);
    Scenario {
        name: "isp-loss",
        caption: "ISP packet loss",
        cpu: series(|_| 0.1 + 0.1 * r.next()),
        memory: series(|_| 0.45),
        disk: series(|_| 0.03),
        down: series(|i| if i > 80 { 0.05 } else { 0.3 + 0.2 * r.next() }),
        up: series(|_| 0.05 * r.next()),
        rtt: series(|i| {
            let x = r.next();
            match i > 80 {
                true if x < 0.3 => None,
                true => Some(0.4 + 0.5 * x),
                false => Some(0.12 + 0.05 * x),
            }
        }),
    }
}

fn upload_saturation() -> Scenario {
    let mut r = Lcg(4);
    Scenario {
        name: "upload-saturation",
        caption: "Upload saturated, latency climbs (bufferbloat)",
        cpu: series(|_| 0.15 + 0.1 * r.next()),
        memory: series(|_| 0.5),
        disk: series(|i| if i > 50 { 0.3 + 0.1 * r.next() } else { 0.05 }),
        down: series(|_| 0.2 + 0.1 * r.next()),
        up: series(|i| if i > 50 { 0.95 } else { 0.1 * r.next() }),
        rtt: series(|i| Some(if i > 55 { 0.55 + 0.4 * r.next() } else { 0.1 })),
    }
}

fn series<T>(f: impl FnMut(usize) -> T) -> Vec<T> {
    (0..LEN).map(f).collect()
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

const LINK_BPS: f64 = 12.5e6;
const RTT_SCALE_MS: f32 = 120.0;

/// Fills `h` with `s` converted to real units, ending at `now`, plus probes and a hop trace.
pub fn populate(h: &mut history::History, s: &Scenario, now: u64) {
    use model::{HopResult, Metric, Probe, ProbeResult, Protocol, Sample, Target};
    let start = now - LEN as u64 + 1;
    let ip = |a: &str| Target::Addr(a.parse().unwrap());
    let probe = |protocol, target| Probe { protocol, target };
    let mut r = Lcg(99);
    for i in 0..LEN {
        let at = start + i as u64;
        let mut x = Sample {
            at,
            per_core: (0..8)
                .map(|c| (s.cpu[i] * 100.0 * (0.6 + 0.1 * c as f32)).min(100.0))
                .collect(),
            ..Default::default()
        };
        x.set(Metric::CpuBusy, s.cpu[i] as f64 * 100.0);
        x.set(Metric::CpuKernel, s.cpu[i] as f64 * 18.0);
        x.set(Metric::CpuQueue, (s.cpu[i] * 6.0).floor() as f64);
        x.set(Metric::MemCommit, s.memory[i] as f64 * 100.0);
        x.set(Metric::MemPhysical, s.memory[i] as f64 * 90.0);
        // Bytes read in from disk: 1400 or 20 pages/s of 4 KiB.
        x.set(
            Metric::MemPageIns,
            if s.memory[i] > 0.8 {
                5_734_400.0
            } else {
                81_920.0
            },
        );
        x.set(Metric::NetDown, (s.down[i] as f64).powi(3) * LINK_BPS);
        x.set(Metric::NetUp, (s.up[i] as f64).powi(3) * LINK_BPS);
        x.set(Metric::DiskBusy, s.disk[i] as f64 * 100.0);
        x.set(Metric::DiskRead, s.disk[i] as f64 * 80e6);
        x.set(Metric::DiskWrite, s.disk[i] as f64 * 30e6);
        x.set(Metric::DiskResponse, s.disk[i] as f64 * 60.0);
        x.set(Metric::DiskQueue, s.disk[i] as f64 * 4.0);
        h.record_sample(&x);

        let rtt = s.rtt[i].map(|v| v * RTT_SCALE_MS);
        let jitter = |base: f32, r: &mut Lcg| base + r.next() * 3.0;
        let results = [
            (
                probe(Protocol::Icmp, Target::Gateway),
                Some(jitter(1.5, &mut r)),
            ),
            (probe(Protocol::Icmp, ip("1.1.1.1")), rtt),
            (
                probe(Protocol::Tcp(443), ip("1.1.1.1")),
                rtt.map(|v| v + 1.2),
            ),
            (probe(Protocol::Icmp, ip("8.8.8.8")), rtt.map(|v| v + 2.5)),
            (
                probe(Protocol::Tcp(443), ip("8.8.8.8")),
                rtt.map(|v| v + 3.1),
            ),
            (probe(Protocol::Dns, ip("1.1.1.1")), rtt.map(|v| v + 0.8)),
            (
                probe(Protocol::Dns, Target::SystemResolver),
                rtt.map(|v| v + 6.0),
            ),
        ];
        for (p, rtt_ms) in results {
            h.record_probe(&ProbeResult {
                probe: p,
                at,
                rtt_ms,
            });
        }
        let trace = probe(Protocol::Icmp, ip("1.1.1.1"));
        let path = [
            "192.168.1.1",
            "100.64.0.1",
            "68.86.92.25",
            "96.110.40.17",
            "173.245.63.1",
            "1.1.1.1",
        ];
        for (n, addr) in path.iter().enumerate() {
            let share = (n + 1) as f32 / path.len() as f32;
            let lost = n >= 2 && rtt.is_none();
            h.record_hop(&HopResult {
                trace: trace.clone(),
                at,
                ttl: n as u8 + 1,
                addr: Some(addr.parse().unwrap()),
                rtt_ms: (!lost).then(|| rtt.unwrap_or(30.0) * share + r.next()),
                reached: n + 1 == path.len(),
            });
        }
    }
}

pub fn processes() -> Vec<model::ProcessRow> {
    [
        ("MsMpEng.exe", 38.2, 412e6, 21e6),
        ("OneDrive.exe", 12.4, 180e6, 3.2e6),
        ("chrome.exe", 9.8, 1.6e9, 0.4e6),
        ("Teams.exe", 6.1, 820e6, 0.1e6),
        ("code.exe", 3.3, 640e6, 0.2e6),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, cpu, mem, disk))| model::ProcessRow {
        name: name.into(),
        pid: 1000 + i as u32,
        cpu_pct: cpu,
        memory_bytes: mem as u64,
        disk_bps: disk,
    })
    .collect()
}
