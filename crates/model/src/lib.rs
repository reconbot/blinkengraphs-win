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
    /// The non-idle share of all logical CPUs (spec `CONTEXT.md`, Metric).
    #[serde(alias = "CpuTotal")]
    CpuBusy,
    CpuKernel,
    CpuQueue,
    MemCommit,
    MemPhysical,
    /// Bytes read from disk into memory per second.
    #[serde(alias = "MemHardFaults")]
    MemPageIns,
    NetDown,
    NetUp,
    NetErrors,
    /// The share of time the disks had a request in flight.
    #[serde(alias = "DiskActive")]
    DiskBusy,
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
        Metric::CpuBusy,
        Metric::CpuKernel,
        Metric::CpuQueue,
        Metric::MemCommit,
        Metric::MemPhysical,
        Metric::MemPageIns,
        Metric::NetDown,
        Metric::NetUp,
        Metric::NetErrors,
        Metric::DiskBusy,
        Metric::DiskRead,
        Metric::DiskWrite,
        Metric::DiskResponse,
        Metric::DiskQueue,
    ];

    pub fn section(self) -> Section {
        use Metric::*;
        match self {
            CpuBusy | CpuKernel | CpuQueue => Section::Cpu,
            MemCommit | MemPhysical | MemPageIns => Section::Memory,
            NetDown | NetUp | NetErrors => Section::Network,
            DiskBusy | DiskRead | DiskWrite | DiskResponse | DiskQueue => Section::Disk,
        }
    }

    pub fn label(self) -> &'static str {
        use Metric::*;
        match self {
            CpuBusy => "Busy",
            CpuKernel => "Kernel",
            CpuQueue => "Run queue",
            MemCommit => "Commit charge",
            MemPhysical => "Physical in use",
            MemPageIns => "Paging in",
            NetDown => "Download",
            NetUp => "Upload",
            NetErrors => "Errors + discards",
            DiskBusy => "Busy",
            DiskRead => "Read",
            DiskWrite => "Write",
            DiskResponse => "Response time",
            DiskQueue => "Queue length",
        }
    }

    pub fn unit(self) -> Unit {
        use Metric::*;
        match self {
            CpuBusy | CpuKernel | MemCommit | MemPhysical | DiskBusy => Unit::Percent,
            NetDown | NetUp | DiskRead | DiskWrite | MemPageIns => Unit::BytesPerSec,
            NetErrors => Unit::PerSec,
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
    /// Each physical disk, for the Verdict's disk rule (spec `rules/verdict.md`, Local rules).
    #[serde(default)]
    pub disks: Vec<Disk>,
    #[serde(default)]
    pub pressure: Pressure,
    /// Throughput of each Link's adapter (spec `docs/history-redesign.md`, Links).
    #[serde(default)]
    pub link_rates: Vec<LinkRate>,
}

/// One Link adapter's throughput over the Sample's interval, bytes/s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LinkRate {
    /// The adapter's NET_LUID, as in its `LinkId`.
    pub luid: u64,
    pub down: f64,
    pub up: f64,
}

/// One physical disk's load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Disk {
    /// Share of time with at least one request in flight, %.
    #[serde(rename = "DiskBusy")]
    pub busy: f64,
    /// Mean time per completed request, ms.
    #[serde(rename = "DiskResponse")]
    pub response_ms: f64,
}

/// The Memory Pressure level, as the OS judges it (spec `CONTEXT.md`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pressure {
    #[default]
    Normal,
    Warn,
    Critical,
}

impl Pressure {
    /// Windows reports no level, so it is derived from commit charge: at 90% of the commit
    /// limit processes start failing to commit memory (spec `rules/verdict.md`).
    pub fn from_commit(commit_pct: f64) -> Pressure {
        if commit_pct >= 90.0 {
            Pressure::Critical
        } else {
            Pressure::Normal
        }
    }
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

/// Names a Link: its interface (the adapter's NET_LUID, which survives a restart) and its
/// Gateway, the IPv4 one, else the IPv6 one, else none, with the Gateway's MAC address when it
/// can be read (spec `rules/history.md`, Links). The MAC keeps apart two networks behind the
/// same Gateway address.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LinkId {
    pub luid: u64,
    pub gateway: Option<std::net::IpAddr>,
    #[serde(default)]
    pub gateway_mac: Option<[u8; 6]>,
}

/// `{luid}@{gateway}/{mac}`, each part only when known.
impl std::fmt::Display for LinkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.luid)?;
        if let Some(g) = self.gateway {
            write!(f, "@{g}")?;
        }
        if let Some(m) = self.gateway_mac {
            let hex: Vec<String> = m.iter().map(|b| format!("{b:02x}")).collect();
            write!(f, "/{}", hex.join("-"))?;
        }
        Ok(())
    }
}

impl std::str::FromStr for LinkId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let err = |e: &dyn std::fmt::Display| e.to_string();
        let (rest, gateway_mac) = match s.split_once('/') {
            Some((r, m)) => {
                let bytes: Vec<u8> = m
                    .split('-')
                    .map(|b| u8::from_str_radix(b, 16))
                    .collect::<Result<_, _>>()
                    .map_err(|e| err(&e))?;
                let mac: [u8; 6] = bytes.try_into().map_err(|_| format!("bad MAC {m:?}"))?;
                (r, Some(mac))
            }
            None => (s, None),
        };
        let (luid, gateway) = match rest.split_once('@') {
            Some((l, g)) => (l, Some(g.parse().map_err(|e| err(&e))?)),
            None => (rest, None),
        };
        Ok(LinkId {
            luid: u64::from_str_radix(luid, 16).map_err(|e| err(&e))?,
            gateway,
            gateway_mac,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LinkKind {
    Ethernet,
    WiFi,
    Vpn,
    Other,
}

/// One adapter's current attachment to a network, and what it takes to send from it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Link {
    pub id: LinkId,
    /// The adapter's name, such as "Wi-Fi".
    pub alias: String,
    pub kind: LinkKind,
    /// Interface indexes for IPv4 and IPv6; 0 where that version isn't enabled.
    pub index_v4: u32,
    pub index_v6: u32,
    /// The addresses Probes leave from, which pick the interface for ICMP.
    pub source_v4: Option<std::net::Ipv4Addr>,
    pub source_v6: Option<std::net::Ipv6Addr>,
    /// Whether this Link carries the default route.
    pub default_route: bool,
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

/// Unix time in fractional seconds, for Round and Pause bounds (spec `rules/history.md`).
pub fn now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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
