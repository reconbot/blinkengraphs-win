use model::{Metric, Probe, Protocol, Section, Target};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub surface: Surface,
    pub sections: Vec<SectionConfig>,
    pub adapter: Adapter,
    pub system_interval_ms: u64,
    pub process_interval_ms: u64,
    /// Drives the Latency Meter.
    pub primary: Probe,
    pub groups: Vec<ProbeGroup>,
    pub hotkey: String,
    pub history: HistoryConfig,
    pub pause_when_idle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    TrayIcons,
    TaskbarStrip,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SectionConfig {
    pub section: Section,
    pub visible: bool,
    pub meter: Vec<Metric>,
    pub strip: Vec<Metric>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Adapter {
    FollowDefaultRoute,
    Pinned(String),
}

/// A named table of Targets and what to measure on each, with one interval per collection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProbeGroup {
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub targets: Vec<TargetRow>,
    #[serde(default = "second")]
    pub latency_ms: u64,
    #[serde(default = "second")]
    pub dns_ms: u64,
    #[serde(default = "second")]
    pub trace_ms: u64,
    /// v1.0 layout, folded into `targets` on load.
    #[serde(default, skip_serializing)]
    interval_ms: Option<u64>,
    #[serde(default, skip_serializing)]
    probes: Vec<Probe>,
    #[serde(default, skip_serializing)]
    traces: Vec<Probe>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetRow {
    pub target: Target,
    #[serde(default)]
    pub icmp: bool,
    /// Port, when a TCP handshake is measured.
    #[serde(default)]
    pub tcp: Option<u16>,
    /// Port, when a UDP datagram is measured.
    #[serde(default)]
    pub udp: Option<u16>,
    #[serde(default)]
    pub dns: bool,
    #[serde(default)]
    pub trace: bool,
}

fn second() -> u64 {
    1000
}

impl TargetRow {
    pub fn new(target: Target) -> Self {
        Self {
            target,
            icmp: false,
            tcp: None,
            udp: None,
            dns: false,
            trace: false,
        }
    }

    fn set(&mut self, p: Protocol) {
        match p {
            Protocol::Icmp => self.icmp = true,
            Protocol::Tcp(port) => self.tcp = Some(port),
            Protocol::Udp(port) => self.udp = Some(port),
            Protocol::Dns => self.dns = true,
        }
    }
}

impl ProbeGroup {
    pub fn new(name: impl Into<String>, targets: Vec<TargetRow>) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            targets,
            latency_ms: 1000,
            dns_ms: 1000,
            trace_ms: 1000,
            interval_ms: None,
            probes: vec![],
            traces: vec![],
        }
    }

    fn probe(t: &TargetRow, protocol: Protocol) -> Probe {
        Probe {
            target: t.target.clone(),
            protocol,
        }
    }

    /// ICMP, TCP and UDP Probes: the collection that runs every `latency_ms`.
    pub fn latency_probes(&self) -> Vec<Probe> {
        self.targets
            .iter()
            .flat_map(|t| {
                let icmp = t.icmp.then(|| Self::probe(t, Protocol::Icmp));
                let tcp = t.tcp.map(|port| Self::probe(t, Protocol::Tcp(port)));
                let udp = t.udp.map(|port| Self::probe(t, Protocol::Udp(port)));
                [icmp, tcp, udp].into_iter().flatten()
            })
            .collect()
    }

    pub fn dns_probes(&self) -> Vec<Probe> {
        self.targets
            .iter()
            .filter(|t| t.dns)
            .map(|t| Self::probe(t, Protocol::Dns))
            .collect()
    }

    /// ICMP Hop Traces; TCP traces stay experimental (ADR 0006).
    pub fn traces(&self) -> Vec<Probe> {
        self.targets
            .iter()
            .filter(|t| t.trace)
            .map(|t| Self::probe(t, Protocol::Icmp))
            .collect()
    }

    fn row(&mut self, target: &Target) -> &mut TargetRow {
        if let Some(i) = self.targets.iter().position(|r| &r.target == target) {
            return &mut self.targets[i];
        }
        self.targets.push(TargetRow::new(target.clone()));
        self.targets.last_mut().unwrap()
    }

    fn is_legacy(&self) -> bool {
        self.targets.is_empty() && (!self.probes.is_empty() || !self.traces.is_empty())
    }

    /// Folds v1.0 `probes`/`traces` into rows; intervals become the group's old interval.
    fn absorb_legacy(&mut self, from: &ProbeGroup) {
        let ms = from.interval_ms.unwrap_or(1000);
        for p in &from.probes {
            self.row(&p.target).set(p.protocol);
            if p.protocol == Protocol::Dns {
                self.dns_ms = ms;
            } else {
                self.latency_ms = ms;
            }
        }
        for t in &from.traces {
            self.row(&t.target).trace = true;
            self.trace_ms = ms;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    pub ram_minutes: u32,
    pub rollup_days: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            ram_minutes: 15,
            rollup_days: 7,
        }
    }
}

const CLOUDFLARE: Target = Target::Addr(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
const GOOGLE: Target = Target::Addr(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));

fn probe(protocol: Protocol, target: Target) -> Probe {
    Probe { target, protocol }
}

impl Default for Config {
    fn default() -> Self {
        use Metric::*;
        let section = |section, meter: &[Metric], strip: &[Metric]| SectionConfig {
            section,
            visible: true,
            meter: meter.to_vec(),
            strip: strip.to_vec(),
        };
        let row = |target, icmp, tcp, dns, trace| TargetRow {
            target,
            icmp,
            tcp,
            udp: None,
            dns,
            trace,
        };
        Self {
            surface: Surface::TrayIcons,
            sections: vec![
                section(Section::Cpu, &[CpuTotal], &[CpuTotal]),
                section(Section::Memory, &[MemCommit], &[MemCommit]),
                section(Section::Network, &[NetDown, NetUp], &[NetDown, NetUp]),
                section(Section::Disk, &[DiskActive], &[DiskActive]),
                section(Section::Latency, &[], &[]),
            ],
            adapter: Adapter::FollowDefaultRoute,
            system_interval_ms: 1000,
            process_interval_ms: 1000,
            primary: probe(Protocol::Icmp, CLOUDFLARE),
            groups: vec![ProbeGroup::new(
                "Default",
                vec![
                    row(Target::Gateway, true, None, false, false),
                    row(CLOUDFLARE, true, Some(443), true, true),
                    row(GOOGLE, true, Some(443), false, false),
                    row(Target::SystemResolver, false, None, true, false),
                ],
            )],
            hotkey: "Ctrl+Alt+B".into(),
            history: HistoryConfig::default(),
            pause_when_idle: true,
        }
    }
}

impl Config {
    pub fn section(&self, s: Section) -> &SectionConfig {
        self.sections
            .iter()
            .find(|c| c.section == s)
            .expect("every Section configured")
    }

    pub fn section_mut(&mut self, s: Section) -> &mut SectionConfig {
        self.sections
            .iter_mut()
            .find(|c| c.section == s)
            .expect("every Section configured")
    }

    /// Restores Sections a hand-edited file dropped, so `section()` never panics.
    fn normalize(mut self) -> Self {
        // v1.0 wrote its default to every config; move unchanged ones to the new default.
        if self.hotkey == "Ctrl+Shift+Alt+M" {
            self.hotkey = Config::default().hotkey;
        }
        // v1.0 kept one free-form list per group; merge those into a single Default group.
        if !self.groups.is_empty() && self.groups.iter().all(ProbeGroup::is_legacy) {
            let mut merged = ProbeGroup::new("Default", vec![]);
            merged.enabled = self.groups.iter().any(|g| g.enabled);
            for g in &self.groups {
                merged.absorb_legacy(g);
            }
            self.groups = vec![merged];
        }
        let defaults = Config::default();
        for d in defaults.sections {
            if !self.sections.iter().any(|c| c.section == d.section) {
                self.sections.push(d);
            }
        }
        self.sections
            .sort_by_key(|c| Section::ALL.iter().position(|s| *s == c.section));
        self
    }

    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str::<Config>(text).map(Config::normalize)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("config serializes")
    }

    /// Missing file → defaults written to disk. Unparseable file → defaults plus the error, file left alone.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match fs::read_to_string(path) {
            Ok(text) => match Self::parse(&text) {
                Ok(c) => (c, None),
                Err(e) => (Self::default(), Some(format!("{}: {e}", path.display()))),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let c = Self::default();
                let err = c.save(path).err().map(|e| e.to_string());
                (c, err)
            }
            Err(e) => (Self::default(), Some(e.to_string())),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, self.to_toml())?;
        fs::rename(tmp, path)
    }
}

pub fn dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default()
        .join("blinkengraphs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips() {
        let c = Config::default();
        assert_eq!(Config::parse(&c.to_toml()).unwrap(), c);
    }

    #[test]
    fn targets_are_readable() {
        let toml = Config::default().to_toml();
        assert!(toml.contains(r#"target = "1.1.1.1""#), "{toml}");
        assert!(toml.contains(r#"target = "gateway""#));
        assert!(toml.contains("tcp = 443"));
        assert!(
            !toml.contains("probes ="),
            "legacy fields are never written"
        );
    }

    #[test]
    fn v1_groups_merge_into_one_default_group() {
        let legacy = r#"
            [[groups]]
            name = "Reachability"
            enabled = true
            interval_ms = 2000
            probes = [{ target = "gateway", protocol = "icmp" }, { target = "1.1.1.1", protocol = "tcp:443" }]
            traces = []

            [[groups]]
            name = "DNS"
            enabled = true
            interval_ms = 1000
            probes = [{ target = "system-dns", protocol = "dns" }]
            traces = []

            [[groups]]
            name = "Hop Trace"
            enabled = true
            interval_ms = 5000
            probes = []
            traces = [{ target = "1.1.1.1", protocol = "icmp" }]
        "#;
        let c = Config::parse(legacy).unwrap();
        assert_eq!(c.groups.len(), 1);
        let g = &c.groups[0];
        assert_eq!(
            (g.name.as_str(), g.latency_ms, g.dns_ms, g.trace_ms),
            ("Default", 2000, 1000, 5000)
        );
        let cf = g.targets.iter().find(|r| r.target == CLOUDFLARE).unwrap();
        assert_eq!((cf.tcp, cf.trace, cf.icmp), (Some(443), true, false));
        assert!(
            g.targets
                .iter()
                .any(|r| r.target == Target::SystemResolver && r.dns)
        );
        assert_eq!(
            Config::parse(&c.to_toml()).unwrap(),
            c,
            "migrated config round-trips"
        );
    }

    #[test]
    fn group_collections_split_by_kind() {
        let g = &Config::default().groups[0];
        assert!(
            g.latency_probes()
                .iter()
                .all(|p| p.protocol != Protocol::Dns)
        );
        assert!(g.dns_probes().iter().all(|p| p.protocol == Protocol::Dns));
        assert_eq!(g.traces(), vec![probe(Protocol::Icmp, CLOUDFLARE)]);
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let c = Config::parse("surface = \"taskbar-strip\"\nsections = []").unwrap();
        assert_eq!(c.surface, Surface::TaskbarStrip);
        assert_eq!(c.sections.len(), Section::ALL.len());
        assert_eq!(c.groups, Config::default().groups);
    }

    #[test]
    fn hostname_target() {
        let c = Config::parse("[primary]\ntarget = \"vpn.example.com\"\nprotocol = \"tcp:443\"")
            .unwrap();
        assert_eq!(c.primary.target, Target::Name("vpn.example.com".into()));
    }

    #[test]
    fn load_creates_missing_file() {
        let dir = std::env::temp_dir().join(format!("bg-config-{}", std::process::id()));
        let path = dir.join("config.toml");
        let (c, err) = Config::load(&path);
        assert!(err.is_none());
        assert_eq!(c, Config::default());
        assert!(path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
