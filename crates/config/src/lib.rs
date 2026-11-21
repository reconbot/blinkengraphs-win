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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProbeGroup {
    pub name: String,
    pub enabled: bool,
    pub interval_ms: u64,
    pub probes: Vec<Probe>,
    pub traces: Vec<Probe>,
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
        let reach = [Target::Gateway, CLOUDFLARE, GOOGLE]
            .into_iter()
            .flat_map(|t| {
                [
                    probe(Protocol::Icmp, t.clone()),
                    probe(Protocol::Tcp(443), t),
                ]
            })
            .filter(|p| p.target != Target::Gateway || p.protocol == Protocol::Icmp)
            .collect();
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
            groups: vec![
                ProbeGroup {
                    name: "Reachability".into(),
                    enabled: true,
                    interval_ms: 1000,
                    probes: reach,
                    traces: vec![],
                },
                ProbeGroup {
                    name: "DNS".into(),
                    enabled: true,
                    interval_ms: 1000,
                    probes: vec![
                        probe(Protocol::Dns, CLOUDFLARE),
                        probe(Protocol::Dns, Target::SystemResolver),
                    ],
                    traces: vec![],
                },
                ProbeGroup {
                    name: "Hop Trace".into(),
                    enabled: true,
                    interval_ms: 1000,
                    probes: vec![],
                    traces: vec![probe(Protocol::Icmp, CLOUDFLARE)],
                },
            ],
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
    fn probes_are_readable_strings() {
        let toml = Config::default().to_toml();
        assert!(toml.contains(r#"protocol = "tcp:443""#), "{toml}");
        assert!(toml.contains(r#"target = "1.1.1.1""#));
        assert!(toml.contains(r#"target = "gateway""#));
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
