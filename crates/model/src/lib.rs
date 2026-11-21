use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
pub enum Target {
    Gateway,
    SystemResolver,
    Addr(IpAddr),
    Host(String),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Gateway => f.write_str("gateway"),
            Target::SystemResolver => f.write_str("system DNS"),
            Target::Addr(a) => write!(f, "{a}"),
            Target::Host(h) => f.write_str(h),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Protocol {
    Icmp,
    Tcp(u16),
    /// UDP DNS query on port 53.
    Dns,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::Icmp => f.write_str("ICMP"),
            Protocol::Tcp(p) => write!(f, "TCP {p}"),
            Protocol::Dns => f.write_str("DNS"),
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

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
