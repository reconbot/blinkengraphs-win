use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Section {
    Cpu,
    Memory,
    Network,
    Disk,
    Latency,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Cpu,
        Section::Memory,
        Section::Network,
        Section::Disk,
        Section::Latency,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Cpu => "CPU",
            Section::Memory => "MEM",
            Section::Network => "NET",
            Section::Disk => "HD",
            Section::Latency => "PING",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Metric {
    CpuTotal,
    CpuKernel,
    CpuQueue,
    MemCommit,
    MemPhysical,
    MemHardFaults,
    NetDown,
    NetUp,
    NetErrors,
    DiskActive,
    DiskRead,
    DiskWrite,
    DiskResponse,
    DiskQueue,
}

pub enum Unit {
    Percent,
    BytesPerSec,
    PerSec,
    Millis,
    Count,
}

impl Metric {
    pub const ALL: [Metric; 14] = [
        Metric::CpuTotal,
        Metric::CpuKernel,
        Metric::CpuQueue,
        Metric::MemCommit,
        Metric::MemPhysical,
        Metric::MemHardFaults,
        Metric::NetDown,
        Metric::NetUp,
        Metric::NetErrors,
        Metric::DiskActive,
        Metric::DiskRead,
        Metric::DiskWrite,
        Metric::DiskResponse,
        Metric::DiskQueue,
    ];

    pub fn section(self) -> Section {
        use Metric::*;
        match self {
            CpuTotal | CpuKernel | CpuQueue => Section::Cpu,
            MemCommit | MemPhysical | MemHardFaults => Section::Memory,
            NetDown | NetUp | NetErrors => Section::Network,
            DiskActive | DiskRead | DiskWrite | DiskResponse | DiskQueue => Section::Disk,
        }
    }

    pub fn label(self) -> &'static str {
        use Metric::*;
        match self {
            CpuTotal => "Total",
            CpuKernel => "Kernel",
            CpuQueue => "Run queue",
            MemCommit => "Commit charge",
            MemPhysical => "Physical in use",
            MemHardFaults => "Hard faults",
            NetDown => "Download",
            NetUp => "Upload",
            NetErrors => "Errors + discards",
            DiskActive => "Active time",
            DiskRead => "Read",
            DiskWrite => "Write",
            DiskResponse => "Response time",
            DiskQueue => "Queue length",
        }
    }

    pub fn unit(self) -> Unit {
        use Metric::*;
        match self {
            CpuTotal | CpuKernel | MemCommit | MemPhysical | DiskActive => Unit::Percent,
            NetDown | NetUp | DiskRead | DiskWrite => Unit::BytesPerSec,
            MemHardFaults | NetErrors => Unit::PerSec,
            DiskResponse => Unit::Millis,
            CpuQueue | DiskQueue => Unit::Count,
        }
    }
}

/// How a window of values is summed up into one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Aggregate {
    #[default]
    P50,
    P95,
    P99,
    Avg,
    Max,
}

impl Aggregate {
    pub const ALL: [Aggregate; 5] = [
        Aggregate::P50,
        Aggregate::P95,
        Aggregate::P99,
        Aggregate::Avg,
        Aggregate::Max,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Aggregate::P50 => "p50",
            Aggregate::P95 => "p95",
            Aggregate::P99 => "p99",
            Aggregate::Avg => "avg",
            Aggregate::Max => "max",
        }
    }
}

/// What a Surface can show: a Metric, or a property of the Latency Meter's Probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Readout {
    Metric(Metric),
    Ping(Ping),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ping {
    Latency,
    Loss,
}

impl Readout {
    pub const ALL: [Readout; 16] = {
        let mut all = [Readout::Ping(Ping::Latency); 16];
        let mut i = 0;
        while i < Metric::ALL.len() {
            all[i] = Readout::Metric(Metric::ALL[i]);
            i += 1;
        }
        all[15] = Readout::Ping(Ping::Loss);
        all
    };

    pub fn section(self) -> Section {
        match self {
            Readout::Metric(m) => m.section(),
            Readout::Ping(_) => Section::Latency,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Readout::Metric(m) => m.label(),
            Readout::Ping(Ping::Latency) => "Latency",
            Readout::Ping(Ping::Loss) => "Loss",
        }
    }

    pub fn metric(self) -> Option<Metric> {
        match self {
            Readout::Metric(m) => Some(m),
            Readout::Ping(_) => None,
        }
    }
}

/// Values of every Metric at one instant. Rates are per second over the previous interval.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Unix seconds.
    pub at: u64,
    pub values: [f64; Metric::ALL.len()],
    pub per_core: Vec<f32>,
}

impl Sample {
    pub fn get(&self, m: Metric) -> f64 {
        self.values[m as usize]
    }

    pub fn set(&mut self, m: Metric, v: f64) {
        self.values[m as usize] = v;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessRow {
    pub name: String,
    pub pid: u32,
    pub cpu_pct: f32,
    pub memory_bytes: u64,
    pub disk_bps: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Target {
    Gateway,
    SystemResolver,
    Addr(IpAddr),
    Name(String),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Gateway => f.write_str("gateway"),
            Target::SystemResolver => f.write_str("system-dns"),
            Target::Addr(a) => write!(f, "{a}"),
            Target::Name(h) => f.write_str(h),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Protocol {
    Icmp,
    Tcp(u16),
    /// A datagram to a port; a reply or an ICMP port-unreachable both prove the host answered.
    Udp(u16),
    /// UDP DNS query on port 53.
    Dns,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::Icmp => f.write_str("icmp"),
            Protocol::Tcp(p) => write!(f, "tcp:{p}"),
            Protocol::Udp(p) => write!(f, "udp:{p}"),
            Protocol::Dns => f.write_str("dns"),
        }
    }
}

impl From<Target> for String {
    fn from(t: Target) -> String {
        t.to_string()
    }
}

impl From<String> for Target {
    fn from(s: String) -> Self {
        match s.as_str() {
            "gateway" => Target::Gateway,
            "system-dns" => Target::SystemResolver,
            _ => s.parse().map(Target::Addr).unwrap_or(Target::Name(s)),
        }
    }
}

impl From<Protocol> for String {
    fn from(p: Protocol) -> String {
        p.to_string()
    }
}

impl TryFrom<String> for Protocol {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "icmp" => Ok(Protocol::Icmp),
            "dns" => Ok(Protocol::Dns),
            _ => s
                .strip_prefix("tcp:")
                .and_then(|p| p.parse().ok())
                .map(Protocol::Tcp)
                .ok_or(format!(
                    "unknown protocol {s:?}; expected icmp, dns or tcp:<port>"
                )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Probe {
    pub target: Target,
    pub protocol: Protocol,
}

impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.protocol, self.target)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub probe: Probe,
    pub at: u64,
    /// `None` is a loss.
    pub rtt_ms: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HopResult {
    pub trace: Probe,
    pub at: u64,
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub rtt_ms: Option<f32>,
    pub reached: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProbeStat {
    pub loss: f32,
    pub rtt_ms: Option<f32>,
    /// 10th-percentile RTT over everything in RAM; what "normal" looks like.
    pub baseline_ms: Option<f32>,
    pub jitter_ms: Option<f32>,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An RTT or RTT statistic for display: whole milliseconds, and "< 1 ms" for a reply recorded
/// as 0 (spec `rules/probe-stats.md`).
pub fn rtt_text(ms: f64) -> String {
    if ms < 0.5 {
        "< 1 ms".into()
    } else {
        format!("{ms:.0} ms")
    }
}
