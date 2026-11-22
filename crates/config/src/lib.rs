pub mod defaults;

use model::{Metric, Ping, Probe, Protocol, Readout, Section, Target};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub surface: Surface,
    pub menu_icon: MenuIcon,
    pub appearance: Appearance,
    /// What each point of a history graph shows when it covers several samples.
    pub graph_aggregate: model::Aggregate,
    /// Keyed by the Probe's display form, e.g. "icmp 1.1.1.1". Missing means `ProbeView::default()`.
    pub probe_views: std::collections::BTreeMap<String, ProbeView>,
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

/// How the Panel and the Latency Meter summarize one Probe. The Verdict ignores this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeView {
    pub window_secs: u64,
    pub aggregate: model::Aggregate,
}

impl ProbeView {
    /// Windows that fit in the RAM history, so every aggregate is exact.
    pub const WINDOWS: [u64; 4] = [30, 60, 300, 900];

    pub fn window_label(secs: u64) -> String {
        if secs.is_multiple_of(60) {
            format!("{} min", secs / 60)
        } else {
            format!("{secs} s")
        }
    }
}

impl Default for ProbeView {
    fn default() -> Self {
        Self {
            window_secs: 30,
            aggregate: model::Aggregate::P50,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// The Panel's light or dark mode; Meters follow the taskbar instead.
    pub mode: Mode,
    pub preset: palette::Preset,
    #[serde(flatten)]
    pub overrides: palette::Overrides,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub fn palette(&self, shade: palette::Shade) -> palette::Palette {
        palette::Palette::new(self.preset, shade, &self.overrides)
    }
}

/// The one notification-area icon left while the Strip is attached; it opens the Panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum MenuIcon {
    #[default]
    App,
    /// The app icon colored by the Verdict's worst Finding.
    Verdict,
    Meter(Section),
}

impl MenuIcon {
    pub fn label(self) -> &'static str {
        match self {
            MenuIcon::App => "App icon",
            MenuIcon::Verdict => "Verdict color",
            MenuIcon::Meter(s) => s.label(),
        }
    }
}

impl From<MenuIcon> for String {
    fn from(m: MenuIcon) -> String {
        match m {
            MenuIcon::App => "app".into(),
            MenuIcon::Verdict => "verdict".into(),
            MenuIcon::Meter(s) => s.label().to_lowercase(),
        }
    }
}

impl TryFrom<String> for MenuIcon {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        match s.as_str() {
            "app" => Ok(MenuIcon::App),
            "verdict" => Ok(MenuIcon::Verdict),
            _ => Section::ALL
                .into_iter()
                .find(|x| x.label().eq_ignore_ascii_case(&s))
                .map(MenuIcon::Meter)
                .ok_or(format!("unknown menu icon {s:?}")),
        }
    }
}

/// Which Readouts each Surface shows for one Section. An empty list hides the Section there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SectionConfig {
    pub section: Section,
    #[serde(default)]
    pub tray: Vec<Readout>,
    #[serde(default)]
    pub strip: Vec<Readout>,
    #[serde(default)]
    pub panel: Vec<Readout>,
    /// v1.0 fields; `normalize` converts them.
    #[serde(default, skip_serializing)]
    visible: Option<bool>,
    #[serde(default, skip_serializing)]
    meter: Option<Vec<Metric>>,
}

impl SectionConfig {
    pub fn new(section: Section, tray: &[Readout], strip: &[Readout], panel: &[Readout]) -> Self {
        Self {
            section,
            tray: tray.to_vec(),
            strip: strip.to_vec(),
            panel: panel.to_vec(),
            visible: None,
            meter: None,
        }
    }

    pub fn metrics(list: &[Readout]) -> Vec<Metric> {
        list.iter().filter_map(|r| r.metric()).collect()
    }

    /// v1.0 had one Visible switch per Section and no Panel choice; PING listed no Metrics.
    fn absorb_legacy(&mut self, default: &SectionConfig) {
        let Some(visible) = self.visible.take() else {
            return;
        };
        let meter = self.meter.take().unwrap_or_default();
        let ping = [Readout::Ping(Ping::Latency), Readout::Ping(Ping::Loss)];
        if self.section == Section::Latency {
            self.tray = ping.to_vec();
            self.strip = ping.to_vec();
        } else {
            self.tray = meter.into_iter().map(Readout::Metric).collect();
        }
        if !visible {
            self.tray.clear();
            self.strip.clear();
        }
        self.panel = default.panel.clone();
    }
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

impl HistoryConfig {
    pub const MAX_DAYS: u32 = 365;
}

impl Default for HistoryConfig {
    fn default() -> Self {
        let h = defaults::get().history;
        Self {
            ram_minutes: h.ram_minutes,
            rollup_days: h.rollup_days,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        use Metric::*;
        let m = |list: &[Metric]| {
            list.iter()
                .copied()
                .map(Readout::Metric)
                .collect::<Vec<_>>()
        };
        let all = |list: &[Metric]| {
            let r = m(list);
            SectionConfig::new(list[0].section(), &r, &r, &r)
        };
        let ping = [Readout::Ping(Ping::Latency), Readout::Ping(Ping::Loss)];
        let d = defaults::get();
        let g = d.probe_group;
        let mut group = ProbeGroup::new(g.name, g.targets);
        group.latency_ms = g.latency_ms;
        group.dns_ms = g.dns_ms;
        group.trace_ms = g.trace_ms;
        Self {
            surface: Surface::TaskbarStrip,
            menu_icon: MenuIcon::App,
            appearance: Appearance::default(),
            graph_aggregate: model::Aggregate::Avg,
            probe_views: Default::default(),
            sections: vec![
                all(&[CpuTotal]),
                all(&[MemCommit]),
                all(&[NetDown, NetUp]),
                SectionConfig::new(Section::Disk, &m(&[DiskActive]), &[], &m(&[DiskActive])),
                SectionConfig::new(Section::Latency, &ping, &ping, &ping),
            ],
            adapter: Adapter::FollowDefaultRoute,
            system_interval_ms: d.system_interval_ms,
            process_interval_ms: 1000,
            primary: d.primary,
            groups: vec![group],
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

    pub fn probe_view(&self, p: &Probe) -> ProbeView {
        self.probe_views
            .get(&p.to_string())
            .copied()
            .unwrap_or_default()
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
        self.history.rollup_days = self.history.rollup_days.clamp(1, HistoryConfig::MAX_DAYS);
        let ram = self.history.ram_minutes as u64 * 60;
        for v in self.probe_views.values_mut() {
            v.window_secs = v.window_secs.clamp(1, ram.max(1));
        }
        let defaults = Config::default();
        for d in defaults.sections {
            match self.sections.iter_mut().find(|c| c.section == d.section) {
                Some(c) => c.absorb_legacy(&d),
                None => self.sections.push(d),
            }
        }
        for c in &mut self.sections {
            let section = c.section;
            for list in [&mut c.tray, &mut c.strip, &mut c.panel] {
                list.retain(|r| r.section() == section);
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
    use std::net::{IpAddr, Ipv4Addr};

    const CLOUDFLARE: Target = Target::Addr(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));

    fn probe(protocol: Protocol, target: Target) -> Probe {
        Probe { target, protocol }
    }

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
    fn v1_sections_become_readout_lists() {
        let c = Config::parse(
            r#"
            [[sections]]
            section = "Cpu"
            visible = true
            meter = ["CpuTotal", "CpuKernel"]
            strip = ["CpuTotal"]
            [[sections]]
            section = "Disk"
            visible = false
            meter = ["DiskActive"]
            strip = ["DiskActive"]
            [[sections]]
            section = "Latency"
            visible = true
            meter = []
            strip = []
            "#,
        )
        .unwrap();
        let cpu = c.section(Section::Cpu);
        let ids = |l: &[Readout]| l.iter().map(|r| r.label()).collect::<Vec<_>>();
        assert_eq!(ids(&cpu.tray), ["Total", "Kernel"]);
        assert_eq!(ids(&cpu.strip), ["Total"]);
        assert_eq!(ids(&cpu.panel), ["Total"]);
        let disk = c.section(Section::Disk);
        assert!(disk.tray.is_empty() && disk.strip.is_empty());
        assert_eq!(ids(&disk.panel), ["Active time"]);
        let ping = c.section(Section::Latency);
        assert_eq!(ids(&ping.strip), ["Latency", "Loss"]);
        assert_eq!(Config::parse(&c.to_toml()).unwrap(), c, "round-trips");
    }

    #[test]
    fn menu_icon_is_a_word_in_toml() {
        for m in [
            MenuIcon::App,
            MenuIcon::Verdict,
            MenuIcon::Meter(Section::Network),
        ] {
            let c = Config {
                menu_icon: m,
                ..Config::default()
            };
            let text = c.to_toml();
            assert!(text.contains(&format!("menu_icon = \"{}\"", String::from(m))));
            assert_eq!(Config::parse(&text).unwrap().menu_icon, m);
        }
    }

    #[test]
    fn default_strip_is_cpu_mem_net_ping() {
        let c = Config::default();
        assert_eq!(c.surface, Surface::TaskbarStrip);
        let on: Vec<_> = Section::ALL
            .into_iter()
            .filter(|s| !c.section(*s).strip.is_empty())
            .collect();
        assert_eq!(
            on,
            [
                Section::Cpu,
                Section::Memory,
                Section::Network,
                Section::Latency
            ]
        );
    }

    #[test]
    fn appearance_round_trips() {
        let mut c = Config::default();
        c.appearance.mode = Mode::Light;
        c.appearance.preset = palette::Preset::ColorBlind;
        c.appearance
            .overrides
            .sections
            .insert(Section::Cpu, palette::Rgb::hex(0x123456));
        c.appearance
            .overrides
            .probes
            .insert("icmp 1.1.1.1".into(), palette::Rgb::hex(0xabcdef));
        let text = c.to_toml();
        assert!(text.contains("\"#123456\""), "{text}");
        assert_eq!(Config::parse(&text).unwrap(), c);
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
