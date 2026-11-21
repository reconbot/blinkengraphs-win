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
