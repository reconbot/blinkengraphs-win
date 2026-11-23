mod settings;
pub mod theme;

use config::{Config, SectionConfig};
use egui::{Align, Color32, Layout, RichText, Ui, vec2};
use egui_plot::{Line, Plot, PlotPoints};
use history::{GraphPoint as Point, History, Key, State};
use model::{Link, LinkId, Metric, Ping, Probe, ProcessRow, Readout, Section, Unit};
use theme::{accent, card, severity_color};
use verdict::{Finding, Severity, bytes};

pub struct View<'a> {
    pub history: &'a History,
    pub findings: &'a [Finding],
    pub processes: &'a [ProcessRow],
    pub adapters: &'a [String],
    /// The Links being measured now.
    pub links: &'a [Link],
    /// The Public Address, IPv4 then IPv6; `None` before the first lookup.
    pub public: Option<&'a [probe::public::Reading; 2]>,
    pub now: u64,
    pub elevated: bool,
    pub config_error: Option<&'a str>,
    /// Status of the last history export.
    pub notice: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ConfigChanged,
    /// Write all stored history to a CSV file.
    ExportHistory,
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Tab {
    #[default]
    Overview,
    Network,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Range {
    Min1,
    Min15,
    Hour,
    Day,
    Week,
    /// From the oldest daily Row to now.
    All,
}

impl Range {
    const ALL: [Range; 6] = [
        Range::Min1,
        Range::Min15,
        Range::Hour,
        Range::Day,
        Range::Week,
        Range::All,
    ];

    fn secs(self, h: &History, now: u64) -> u64 {
        match self {
            Range::Min1 => 60,
            Range::Min15 => 900,
            Range::Hour => 3600,
            Range::Day => 86_400,
            Range::Week => 604_800,
            Range::All => h.oldest().map_or(60, |t| now.saturating_sub(t).max(60)),
        }
    }

    fn history(self) -> history::Range {
        match self {
            Range::Min1 => history::Range::Min1,
            Range::Min15 => history::Range::Min15,
            Range::Hour => history::Range::Hour,
            Range::Day => history::Range::Day,
            Range::Week => history::Range::Week,
            Range::All => history::Range::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Range::Min1 => "1m",
            Range::Min15 => "15m",
            Range::Hour => "1h",
            Range::Day => "24h",
            Range::Week => "7d",
            Range::All => "All",
        }
    }
}

pub struct Panel {
    pub tab: Tab,
    pub range: Range,
    pub sort: ProcessSort,
    scroll_to_top: bool,
    graph_aggregate: model::Aggregate,
    /// One Link to graph, or every Link (`None`, the default: spec `rules/history.md`).
    pub link: Option<LinkId>,
}

/// Impact ranks by CPU and memory together, each as a share of the busiest process listed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ProcessSort {
    #[default]
    Impact,
    Cpu,
    Memory,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            tab: Tab::Overview,
            range: Range::Min1,
            sort: ProcessSort::Impact,
            scroll_to_top: false,
            graph_aggregate: config::defaults::get().graph_aggregate,
            link: None,
        }
    }
}

impl Panel {
    /// Clicking the open tab again scrolls it to the top.
    pub fn select(&mut self, tab: Tab) {
        self.scroll_to_top = self.tab == tab;
        self.tab = tab;
    }

    pub fn show(&mut self, ui: &mut Ui, view: &View, config: &mut Config) -> Vec<Action> {
        let mut actions = Vec::new();
        theme::apply(ui.ctx(), &config.appearance);
        self.graph_aggregate = config.graph_aggregate;
        egui::CentralPanel::default()
            .frame(theme::window_frame())
            .show(ui, |ui| {
                self.header(ui, view, &mut actions);
                ui.add_space(8.0);
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt(self.tab)
                    .auto_shrink(false);
                if std::mem::take(&mut self.scroll_to_top) {
                    scroll = scroll.vertical_scroll_offset(0.0);
                }
                scroll.show(ui, |ui| match self.tab {
                    Tab::Overview => self.overview(ui, view, config),
                    Tab::Network => self.network(ui, view, config),
                    Tab::Settings => {
                        let mut export = false;
                        if settings::show(ui, config, view, &mut export) {
                            actions.push(Action::ConfigChanged);
                        }
                        if export {
                            actions.push(Action::ExportHistory);
                        }
                    }
                });
            });
        actions
    }

    fn header(&mut self, ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("blinkengraphs").strong().size(15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Quit").clicked() {
                    actions.push(Action::Quit);
                }
                for (tab, label) in [
                    (Tab::Settings, "Settings"),
                    (Tab::Network, "Network"),
                    (Tab::Overview, "Overview"),
                ] {
                    if ui.selectable_label(self.tab == tab, label).clicked() {
                        self.select(tab);
                    }
                }
            });
        });
        ui.add_space(6.0);
        verdict_banner(ui, view.findings);
        if let Some(e) = view.config_error {
            ui.colored_label(theme::warn(), format!("Config not loaded: {e}"));
        }
        if self.tab != Tab::Settings {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for r in Range::ALL {
                    ui.selectable_value(&mut self.range, r, r.label());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.link_picker(ui, view)
                });
            });
        }
    }

    fn overview(&mut self, ui: &mut Ui, view: &View, config: &Config) {
        let s = view.history.last_sample();
        let shown: Vec<(Section, Vec<Metric>)> = [
            Section::Cpu,
            Section::Memory,
            Section::Network,
            Section::Disk,
        ]
        .into_iter()
        .map(|section| {
            (
                section,
                SectionConfig::metrics(&config.section(section).panel),
            )
        })
        .filter(|(_, m)| !m.is_empty())
        .collect();
        // Two independent columns, so a short card doesn't leave a gap beside a tall one.
        if !shown.is_empty() {
            ui.columns(2, |cols| {
                for (i, (section, plotted)) in shown.iter().enumerate() {
                    let col = &mut cols[i % 2];
                    card(col, *section, |ui| {
                        self.section_card(ui, view, *section, plotted, s)
                    });
                    col.add_space(8.0);
                }
            });
        }
        let ping = &config.section(Section::Latency).panel;
        if !ping.is_empty() {
            card(ui, Section::Latency, |ui| {
                self.latency_card(ui, view, config, false, ping)
            });
            ui.add_space(8.0);
        }
        processes(ui, view.processes, &mut self.sort);
    }

    fn section_card(
        &self,
        ui: &mut Ui,
        view: &View,
        section: Section,
        plotted: &[Metric],
        s: Option<&model::Sample>,
    ) {
        let primary = plotted[0];
        let duplex = plotted.contains(&Metric::NetDown) && plotted.contains(&Metric::NetUp);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(section.label())
                    .color(accent(section))
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let v = s.map(|s| s.get(primary)).unwrap_or(0.0);
                let text = if duplex {
                    let down = s.map(|s| s.get(Metric::NetDown)).unwrap_or(0.0);
                    let up = s.map(|s| s.get(Metric::NetUp)).unwrap_or(0.0);
                    format!("↓{}/s ↑{}/s", bytes(down), bytes(up))
                } else {
                    format_value(primary, v)
                };
                let size = if duplex { 14.0 } else { 18.0 };
                ui.label(RichText::new(text).size(size).strong());
            });
        });
        let series: Vec<_> = plotted
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let color = match (i, m) {
                    (_, Metric::NetUp) => theme::upload(),
                    (0, _) => accent(section),
                    _ => theme::series(i),
                };
                (m.label(), color, self.points(view, &self.metric_key(*m)))
            })
            .collect();
        let fmt = move |v: f64| match primary.unit() {
            Unit::BytesPerSec => bytes(v),
            Unit::Percent => format!("{v:.0}%"),
            _ => format!("{v:.0}"),
        };
        sparkplot(
            ui,
            &format!("{section:?}"),
            series,
            matches!(primary.unit(), Unit::Percent).then_some(100.0),
            70.0,
            fmt,
            self.window(view),
        );
        ui.horizontal_wrapped(|ui| {
            for m in Metric::ALL
                .iter()
                .filter(|m| m.section() == section && !plotted.contains(m))
            {
                let v = s.map(|s| s.get(*m)).unwrap_or(0.0);
                ui.label(
                    RichText::new(format!("{} {}", m.label(), format_value(*m, v)))
                        .small()
                        .weak(),
                );
            }
        });
        if section == Section::Cpu
            && let Some(s) = s
        {
            cores(ui, &s.per_core);
        }
    }

    fn latency_card(
        &self,
        ui: &mut Ui,
        view: &View,
        config: &Config,
        all: bool,
        shown: &[Readout],
    ) {
        let (latency, loss) = (
            shown.contains(&Readout::Ping(Ping::Latency)),
            shown.contains(&Readout::Ping(Ping::Loss)),
        );
        let probes = latency_probes(view.history, all);
        let lines = self.latency_lines(view.history, all);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Latency")
                    .color(accent(Section::Latency))
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(p) = probes.first() {
                    let v = config.probe_view(p);
                    let st = view
                        .history
                        .aggregate(p, view.now, v.window_secs, v.aggregate);
                    let rtt = st
                        .value
                        .map(|r| model::rtt_text(r as f64))
                        .unwrap_or("—".into());
                    let text = match (latency, loss) {
                        (true, true) => format!("{rtt}  {:.0}% loss", st.loss * 100.0),
                        (true, false) => rtt,
                        _ => format!("{:.0}% loss", st.loss * 100.0),
                    };
                    ui.label(RichText::new(text).size(18.0).strong());
                }
            });
        });
        let series: Vec<(String, Color32, Vec<Point>)> = lines
            .iter()
            .enumerate()
            .map(|(i, (l, p))| {
                (
                    line_name(p, l.as_ref(), view.links),
                    theme::probe(p, i),
                    self.points(view, &Key::Probe(l.clone(), p.clone())),
                )
            })
            .collect();
        let markers = self.link_markers(view);
        multiplot(
            ui,
            "latency",
            &series,
            &markers,
            if all { 180.0 } else { 110.0 },
            self.window(view),
        );
        ui.horizontal_wrapped(|ui| {
            for (i, (l, p)) in lines.iter().enumerate() {
                let v = config.probe_view(p);
                let sum =
                    view.history
                        .aggregate_on(l.as_ref(), p, view.now, v.window_secs, v.aggregate);
                let st = view.history.now_on(l.as_ref(), p, view.now);
                // A p50 readout is the plain RTT, so it goes unnamed (spec `rules/probe-stats.md`).
                let named = match v.aggregate {
                    model::Aggregate::P50 => String::new(),
                    a => format!("{} ", a.label()),
                };
                let text = format!(
                    "{}  {named}{}  {:.0}%{}",
                    line_name(p, l.as_ref(), view.links),
                    sum.value
                        .map(|r| model::rtt_text(r as f64))
                        .unwrap_or("—".into()),
                    sum.loss * 100.0,
                    st.and_then(|s| s.jitter_ms)
                        .map(|j| format!("  ±{j:.0}"))
                        .unwrap_or_default()
                );
                dot(ui, theme::probe(p, i), 3.5);
                ui.label(RichText::new(text).small().color(theme::probe(p, i)));
            }
        });
    }

    /// Throughput is the picked Link's adapter when one is picked, else the Followed Adapter's;
    /// every other Metric belongs to the machine.
    fn metric_key(&self, m: Metric) -> Key {
        match (&self.link, m) {
            (Some(l), Metric::NetDown | Metric::NetUp) => Key::LinkMetric(l.clone(), m),
            _ => Key::Metric(m),
        }
    }

    /// Every (Link, Probe) the latency graph draws: one line per Link by default, or the
    /// picked Link's.
    fn latency_lines(&self, h: &History, all: bool) -> Vec<(Option<LinkId>, Probe)> {
        let mut v: Vec<(Option<LinkId>, Probe)> = h
            .series_keys()
            .into_iter()
            .filter(|(l, _)| self.link.is_none() || *l == self.link)
            .filter(|(_, p)| {
                if all {
                    true
                } else {
                    p.protocol == model::Protocol::Icmp
                }
            })
            .collect();
        v.sort_by_key(|(l, p)| {
            (
                p.target.to_string(),
                p.protocol.to_string(),
                l.as_ref() != h.followed(),
                l.clone(),
            )
        });
        v
    }

    /// Where the followed Link changed, on ranges drawn from Raw (spec `rules/history.md`).
    fn link_markers(&self, view: &View) -> Vec<f64> {
        if self.range.history().source() != history::range::Source::Raw {
            return vec![];
        }
        let changes = view.history.link_changes();
        // The first entry is the Link History started on, not a change.
        changes
            .iter()
            .skip(1)
            .map(|(at, _)| *at as f64 - view.now as f64)
            .collect()
    }

    /// Lets the user graph one Link, once more than one has results.
    fn link_picker(&mut self, ui: &mut Ui, view: &View) {
        let mut links: Vec<Option<LinkId>> = view
            .history
            .series_keys()
            .into_iter()
            .map(|(l, _)| l)
            .collect();
        links.sort();
        links.dedup();
        if links.len() < 2 {
            self.link = None;
            return;
        }
        let name = |l: &Option<LinkId>| match l {
            None => "Unknown Link".to_string(),
            Some(id) => link_alias(id, view.links),
        };
        let selected = self
            .link
            .clone()
            .map_or("Every Link".to_string(), |l| name(&Some(l)));
        egui::ComboBox::from_id_salt("link")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.link, None, "Every Link");
                for l in links.into_iter().flatten() {
                    let text = name(&Some(l.clone()));
                    ui.selectable_value(&mut self.link, Some(l), text);
                }
            });
    }

    fn network(&self, ui: &mut Ui, view: &View, config: &Config) {
        let all = [Readout::Ping(Ping::Latency), Readout::Ping(Ping::Loss)];
        card(ui, Section::Latency, |ui| {
            self.latency_card(ui, view, config, true, &all)
        });
        for trace in config
            .groups
            .iter()
            .filter(|g| g.enabled)
            .flat_map(|g| g.traces())
        {
            ui.add_space(8.0);
            card(ui, Section::Latency, |ui| {
                hop_table(ui, view.history, &trace)
            });
        }
        if let Some(public) = view.public {
            ui.add_space(8.0);
            card(ui, Section::Network, |ui| public_card(ui, public));
        }
    }

    fn window(&self, view: &View) -> Window {
        Window {
            now: view.now as f64,
            secs: self.range.secs(view.history, view.now) as f64,
            aggregate: self.graph_aggregate,
        }
    }

    fn points(&self, view: &View, key: &Key) -> Vec<Point> {
        view.history
            .series(key, self.range.history(), view.now, self.graph_aggregate)
    }
}

/// The Public Address per IP version, shown as the spec says; it never feeds the Verdict.
fn public_card(ui: &mut Ui, readings: &[probe::public::Reading; 2]) {
    ui.label(
        RichText::new("Public Address")
            .color(accent(Section::Network))
            .strong(),
    );
    for (v6, r) in [false, true].into_iter().zip(readings) {
        let (text, flagged) = public_line(v6, r);
        ui.horizontal(|ui| {
            ui.label(RichText::new(if v6 { "IPv6" } else { "IPv4" }).weak());
            let t = RichText::new(text);
            ui.label(if flagged { t.color(theme::warn()) } else { t });
        });
    }
}

/// One IP version's reading as text, and whether it is flagged (spec
/// `rules/public-address.md`, Reading the answers).
fn public_line(v6: bool, r: &probe::public::Reading) -> (String, bool) {
    use probe::public::{Reading, Source};
    let asn = |a: &Option<String>| a.as_ref().map(|a| format!(" · AS{a}")).unwrap_or_default();
    match r {
        Reading::Agree { address, asn: a } => (format!("{address}{}", asn(a)), false),
        Reading::Differ { dns, https, .. } => (
            format!("DNS {dns}, HTTPS {https}: different paths out"),
            true,
        ),
        Reading::Nat64(_) => ("IPv6-only network with NAT64".into(), false),
        Reading::None if v6 => ("No IPv6 internet".into(), false),
        Reading::None => ("No IPv4 internet".into(), false),
        Reading::Single {
            address,
            source,
            asn: a,
        } => {
            let from = match source {
                Source::Dns => "DNS",
                Source::Https => "HTTPS",
            };
            (format!("{address}{} ({from} only)", asn(a)), false)
        }
    }
}

/// "icmp 1.1.1.1 · Wi-Fi": the Probe, and its Link's adapter when known.
fn line_name(p: &Probe, link: Option<&LinkId>, links: &[Link]) -> String {
    match link {
        Some(l) => format!("{p} · {}", link_alias(l, links)),
        None => p.to_string(),
    }
}

/// The adapter's name, or the Link's id once it is gone.
/// The adapter's name. A Link no longer up, such as the same adapter under the identity it had
/// before, is named after its adapter and marked earlier, rather than shown as a raw id.
fn link_alias(id: &LinkId, links: &[Link]) -> String {
    if let Some(l) = links.iter().find(|l| l.id == *id) {
        return l.alias.clone();
    }
    match links.iter().find(|l| l.id.luid == id.luid) {
        Some(l) => format!("{} (earlier)", l.alias),
        None => "earlier network".to_string(),
    }
}

fn latency_probes(h: &History, all: bool) -> Vec<Probe> {
    let mut v: Vec<Probe> = h
        .probes()
        .into_iter()
        .filter(|p| all || p.protocol != model::Protocol::Dns)
        .collect();
    v.sort_by_key(|p| (p.target.to_string(), p.protocol.to_string()));
    if !all {
        v.retain(|p| p.protocol == model::Protocol::Icmp);
    }
    v
}

fn verdict_banner(ui: &mut Ui, findings: &[Finding]) {
    let Some(top) = findings.first() else { return };
    let color = severity_color(top.severity);
    egui::Frame::new()
        .fill(color.gamma_multiply(0.15))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(10.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                dot(ui, color, 5.0);
                ui.add(egui::Label::new(RichText::new(&top.headline).size(15.0).strong()).wrap());
            });
            for f in &findings[1..] {
                ui.label(
                    RichText::new(format!("   {}", f.headline))
                        .small()
                        .color(severity_color(f.severity)),
                );
            }
        });
}

fn dot(ui: &mut Ui, color: Color32, r: f32) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(r * 2.0 + 2.0, r * 2.0 + 2.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), r, color);
}

fn cores(ui: &mut Ui, per_core: &[f32]) {
    ui.label(RichText::new("Cores").small().weak());
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), 10.0), egui::Sense::hover());
    let n = per_core.len().max(1) as f32;
    let w = rect.width() / n;
    for (i, v) in per_core.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            rect.min + vec2(i as f32 * w, 0.0),
            vec2(w - 2.0, rect.height()),
        );
        let c = theme::level(Section::Cpu, v / 100.0);
        ui.painter()
            .rect_filled(r, 2.0, c.gamma_multiply(0.25 + 0.75 * v / 100.0));
    }
    if let Some(pos) = resp.hover_pos() {
        let i = (((pos.x - rect.left()) / w) as usize).min(per_core.len().saturating_sub(1));
        if let Some(v) = per_core.get(i) {
            resp.on_hover_text_at_pointer(format!("Core {i}: {v:.0}%"));
        }
    }
}

fn processes(ui: &mut Ui, rows: &[ProcessRow], sort: &mut ProcessSort) {
    egui::Frame::new()
        .fill(theme::card_fill())
        .corner_radius(10.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Top processes").strong());
            if rows.is_empty() {
                ui.label(RichText::new("Sampling…").weak());
                return;
            }
            let max_cpu = rows.iter().map(|r| r.cpu_pct).fold(1.0, f32::max);
            let max_mem = rows
                .iter()
                .map(|r| r.memory_bytes)
                .max()
                .unwrap_or(1)
                .max(1) as f32;
            let impact =
                |r: &ProcessRow| (r.cpu_pct / max_cpu + r.memory_bytes as f32 / max_mem) / 2.0;
            let mut sorted: Vec<&ProcessRow> = rows.iter().collect();
            match sort {
                ProcessSort::Impact => sorted.sort_by(|a, b| impact(b).total_cmp(&impact(a))),
                ProcessSort::Cpu => sorted.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct)),
                ProcessSort::Memory => sorted.sort_by_key(|r| std::cmp::Reverse(r.memory_bytes)),
            }

            let widths = [0.0, 52.0, 56.0, 72.0, 72.0];
            table_cells(ui, &widths, false, |i, ui| {
                let (label, key) = match i {
                    0 => ("Process", None),
                    1 => ("Impact", Some(ProcessSort::Impact)),
                    2 => ("CPU", Some(ProcessSort::Cpu)),
                    3 => ("Memory", Some(ProcessSort::Memory)),
                    _ => ("Disk", None),
                };
                match key {
                    Some(k) => {
                        let active = *sort == k;
                        let text = if active {
                            format!("{label} ↓")
                        } else {
                            label.to_string()
                        };
                        let rt = RichText::new(text).small();
                        let rt = if active {
                            rt.color(theme::text())
                        } else {
                            rt.weak()
                        };
                        if ui
                            .add(egui::Button::new(rt).frame(false))
                            .on_hover_text(format!("Sort by {label}"))
                            .clicked()
                        {
                            *sort = k;
                        }
                    }
                    None => {
                        ui.label(RichText::new(label).small().weak());
                    }
                }
            });
            for (row, r) in sorted.into_iter().take(8).enumerate() {
                let score = impact(r);
                table_cells(ui, &widths, row % 2 == 1, |i, ui| match i {
                    0 => {
                        ui.add(egui::Label::new(RichText::new(&r.name)).truncate());
                    }
                    1 => impact_bar(ui, score),
                    2 => {
                        ui.label(
                            RichText::new(format!("{:.1}%", r.cpu_pct))
                                .color(theme::level(Section::Cpu, r.cpu_pct / 100.0)),
                        );
                    }
                    3 => {
                        ui.label(bytes(r.memory_bytes as f64));
                    }
                    _ => {
                        ui.label(format!("{}/s", bytes(r.disk_bps)));
                    }
                });
            }
        });
}

fn impact_bar(ui: &mut Ui, score: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(44.0, 8.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, theme::faint());
    let mut fill = rect;
    fill.set_width(rect.width() * score.clamp(0.0, 1.0));
    ui.painter()
        .rect_filled(fill, 3.0, theme::level(Section::Cpu, score));
}

/// Fixed-width columns so values don't shift as rows change. Width 0 takes the remaining space,
/// left-aligned; every other column is right-aligned.
pub(crate) fn table_cells(
    ui: &mut Ui,
    widths: &[f32],
    stripe: bool,
    mut cell: impl FnMut(usize, &mut Ui),
) {
    let fixed: f32 = widths.iter().sum();
    let gap = 8.0;
    let flex = (ui.available_width() - fixed - gap * widths.len() as f32).max(40.0);
    let fill = if stripe {
        theme::faint()
    } else {
        Color32::TRANSPARENT
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(4, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (i, w) in widths.iter().enumerate() {
                    let (width, layout) = if *w == 0.0 {
                        (flex, Layout::left_to_right(Align::Center))
                    } else {
                        (*w, Layout::right_to_left(Align::Center))
                    };
                    ui.allocate_ui_with_layout(vec2(width, 18.0), layout, |ui| {
                        ui.set_width(width);
                        cell(i, ui);
                    });
                }
            });
        });
}

fn table_row(ui: &mut Ui, cells: &[(f32, RichText)], stripe: bool) {
    let widths: Vec<f32> = cells.iter().map(|(w, _)| *w).collect();
    table_cells(ui, &widths, stripe, |i, ui| {
        ui.add(egui::Label::new(cells[i].1.clone()).truncate());
    });
}

fn hop_table(ui: &mut Ui, h: &History, trace: &Probe) {
    ui.label(
        RichText::new(format!("Hop trace · {trace}"))
            .color(accent(Section::Latency))
            .strong(),
    );
    let hops = h.hops(trace);
    if hops.is_empty() {
        ui.label(RichText::new("Waiting for replies…").weak());
        return;
    }
    let ms = |v: Option<f32>| {
        v.map(|v| {
            if v < 0.5 {
                "<1".into()
            } else {
                format!("{v:.0}")
            }
        })
        .unwrap_or("—".into())
    };
    let cols: [(f32, &str); 7] = [
        (22.0, "#"),
        (0.0, "Host"),
        (40.0, "Loss"),
        (44.0, "Last"),
        (44.0, "Avg"),
        (44.0, "Best"),
        (44.0, "Worst"),
    ];
    table_row(
        ui,
        &cols.map(|(w, h)| (w, RichText::new(h).small().weak())),
        false,
    );
    for (i, hop) in hops.iter().enumerate() {
        let loss_color = if hop.loss >= 0.2 {
            theme::crit()
        } else if hop.loss > 0.0 {
            theme::warn()
        } else {
            theme::text_weak()
        };
        let cells = [
            (22.0, RichText::new(hop.ttl.to_string()).weak()),
            (0.0, RichText::new(host(hop.addr))),
            (
                40.0,
                RichText::new(format!("{:.0}%", hop.loss * 100.0)).color(loss_color),
            ),
            (44.0, RichText::new(ms(hop.last_ms))),
            (44.0, RichText::new(ms(hop.avg_ms))),
            (44.0, RichText::new(ms(hop.best_ms))),
            (44.0, RichText::new(ms(hop.worst_ms))),
        ];
        table_row(ui, &cells, i % 2 == 1);
    }
}

/// The Hop's reverse-DNS name with its address, like mtr's `-b`, or the address alone.
fn host(addr: Option<std::net::IpAddr>) -> String {
    static REVERSE_DNS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let Some(a) = addr else {
        return "???".into();
    };
    let lookup = *REVERSE_DNS.get_or_init(|| config::defaults::get().hop_trace.reverse_dns);
    match lookup.then(|| probe::names::name(a)).flatten() {
        Some(name) => format!("{name} ({a})"),
        None => a.to_string(),
    }
}

fn format_value(m: Metric, v: f64) -> String {
    match m.unit() {
        Unit::Percent => format!("{v:.0}%"),
        Unit::BytesPerSec => format!("{}/s", bytes(v)),
        Unit::PerSec => format!("{v:.0}/s"),
        Unit::Millis => format!("{v:.1} ms"),
        Unit::Count => format!("{v:.1}"),
    }
}

/// The plotted time range: x runs from -`secs` to 0, where 0 is `now`.
#[derive(Clone, Copy)]
struct Window {
    now: f64,
    secs: f64,
    /// What each point shows when it covers several samples.
    aggregate: model::Aggregate,
}

/// Names the graph aggregate in the plot's top-right corner (spec `rules/graph-aggregate.md`).
/// `top` must come from the data: a Text item counts toward the auto-fitted bounds, so placing
/// it at the plot's own top edge grows the axis by the margin every frame, without limit.
fn aggregate_tag(pu: &mut egui_plot::PlotUi, w: Window, top: f64) {
    pu.text(
        egui_plot::Text::new(
            "",
            egui_plot::PlotPoint::new(0.0, top),
            RichText::new(w.aggregate.label()).small(),
        )
        .anchor(egui::Align2::RIGHT_TOP)
        .color(theme::text_weak()),
    );
}

/// The highest value the plot must show: its points and any fixed maximum, never less than 0.
fn data_top<'a>(series: impl Iterator<Item = &'a [Point]>, max: Option<f64>) -> f64 {
    series
        .flatten()
        .filter_map(|p| p.value)
        .chain(max)
        .fold(0.0, f64::max)
}

/// One line per run of adjacent buckets with values: a missing, lost or Paused bucket breaks
/// the line rather than joining across it (spec `rules/history.md`, Drawing).
fn to_lines(
    name: &str,
    color: Color32,
    pts: &[Point],
    fill: bool,
    w: Window,
) -> Vec<Line<'static>> {
    let mut runs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut prev: Option<&Point> = None;
    for p in pts {
        let joins = prev.is_some_and(|q| q.value.is_some() && p.at <= q.at + q.w);
        if let Some(v) = p.value {
            if !joins {
                runs.push(Vec::new());
            }
            runs.last_mut().unwrap().push([p.at as f64 - w.now, v]);
        }
        prev = Some(p);
    }
    runs.into_iter()
        .map(|mut xy| {
            // A lone bucket draws as a level segment across its width; one point draws nothing.
            if let [[x, y]] = xy[..] {
                let width = pts
                    .iter()
                    .find(|p| p.at as f64 - w.now == x)
                    .map_or(1.0, |p| p.w as f64);
                xy.push([x + width, y]);
            }
            let line = Line::new(name.to_string(), PlotPoints::from(xy))
                .color(color)
                .width(1.5);
            if fill {
                line.fill(0.0).fill_alpha(0.18)
            } else {
                line
            }
        })
        .collect()
}

/// Shades Paused buckets as "not measuring", and rings the provisional edge, which can still
/// change (spec `rules/history.md`, Drawing).
fn states<'a>(pu: &mut egui_plot::PlotUi, w: Window, series: impl Iterator<Item = &'a [Point]>) {
    let t = theme::text();
    let mut provisional: Vec<[f64; 2]> = Vec::new();
    let mut paused: Vec<(f64, f64)> = Vec::new();
    for pts in series {
        for p in pts {
            let x = p.at as f64 - w.now;
            if p.state == State::Paused {
                match paused.last_mut() {
                    Some((_, end)) if *end >= x => *end = x + p.w as f64,
                    _ => paused.push((x, x + p.w as f64)),
                }
            } else if p.provisional
                && let Some(v) = p.value
            {
                provisional.push([x, v]);
            }
        }
    }
    for (from, to) in paused {
        pu.span(
            egui_plot::Span::new("", from..=to)
                .fill(gray(t))
                .border_width(0.0),
        );
    }
    if !provisional.is_empty() {
        pu.points(
            egui_plot::Points::new("provisional", provisional)
                .color(theme::text_weak())
                .filled(false)
                .radius(2.5),
        );
    }
}

fn sparkplot(
    ui: &mut Ui,
    id: &str,
    series: Vec<(&str, Color32, Vec<Point>)>,
    max: Option<f64>,
    height: f32,
    fmt: impl Fn(f64) -> String + Clone + 'static,
    w: Window,
) {
    let axis = fmt.clone();
    let mut plot = base_plot(id, height, w)
        .include_y(0.0)
        .y_axis_formatter(move |m, _| axis(m.value));
    if let Some(m) = max {
        plot = plot.include_y(m);
    }
    let shown = plot.show(ui, |pu| {
        unrecorded(pu, w, series.iter().map(|s| s.2.as_slice()));
        states(pu, w, series.iter().map(|s| s.2.as_slice()));
        aggregate_tag(pu, w, data_top(series.iter().map(|s| s.2.as_slice()), max));
        for (name, color, pts) in &series {
            for l in to_lines(name, *color, pts, true, w) {
                pu.line(l);
            }
        }
        pu.pointer_coordinate().map(|p| p.x)
    });
    let lines: Vec<_> = series
        .iter()
        .map(|(n, c, p)| (n.to_string(), *c, p.as_slice()))
        .collect();
    hover_values(shown.response, shown.inner, &lines, w, &fmt);
}

/// The value of every line where the pointer is, in a tooltip: what was drawn there, not the
/// pointer's coordinates.
fn hover_values(
    response: egui::Response,
    x: Option<f64>,
    lines: &[(String, Color32, &[Point])],
    w: Window,
    fmt: &dyn Fn(f64) -> String,
) {
    let Some(x) = x.filter(|_| response.hovered()) else {
        return;
    };
    let rows = values_at(lines, x, w.now);
    response.on_hover_ui_at_pointer(|ui| {
        ui.label(RichText::new(ago(-x)).small().weak());
        for (name, color, v) in rows {
            let text = match v {
                Some(Shown::Value(v)) => fmt(v),
                Some(Shown::Lost) => "lost".to_string(),
                Some(Shown::Paused) => "not measuring".to_string(),
                None => "—".to_string(),
            };
            ui.label(RichText::new(format!("{name}  {text}")).color(color));
        }
    });
}

/// What one line shows in the bucket under `x` (seconds before now).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shown {
    Value(f64),
    Lost,
    Paused,
}

/// Each line's bucket covering `x`, seconds relative to `now`; `None` where the line has none.
fn values_at(
    lines: &[(String, Color32, &[Point])],
    x: f64,
    now: f64,
) -> Vec<(String, Color32, Option<Shown>)> {
    lines
        .iter()
        .map(|(name, color, pts)| {
            let p = pts.iter().find(|p| {
                let from = p.at as f64 - now;
                from <= x && x < from + p.w as f64
            });
            let shown = p.map(|p| match (p.state, p.value) {
                (State::Paused, _) => Shown::Paused,
                (_, Some(v)) => Shown::Value(v),
                _ => Shown::Lost,
            });
            (name.clone(), *color, shown)
        })
        .collect()
}

/// "45 s ago", "12 min ago", "3 h ago", "2 d ago".
fn ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        s if s < 120 => format!("{s} s ago"),
        s if s < 7200 => format!("{} min ago", s / 60),
        s if s < 172_800 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

fn multiplot(
    ui: &mut Ui,
    id: &str,
    series: &[(String, Color32, Vec<Point>)],
    markers: &[f64],
    height: f32,
    w: Window,
) {
    let shown = base_plot(id, height, w)
        .include_y(0.0)
        .y_axis_formatter(|m, _| format!("{:.0} ms", m.value))
        .show(ui, |pu| {
            unrecorded(pu, w, series.iter().map(|s| s.2.as_slice()));
            states(pu, w, series.iter().map(|s| s.2.as_slice()));
            aggregate_tag(pu, w, data_top(series.iter().map(|s| s.2.as_slice()), None));
            for x in markers {
                pu.vline(
                    egui_plot::VLine::new("Link changed", *x)
                        .color(theme::text_weak())
                        .width(1.0),
                );
            }
            for (name, color, pts) in series {
                for l in to_lines(name, *color, pts, false, w) {
                    pu.line(l);
                }
                let lost: Vec<[f64; 2]> = pts
                    .iter()
                    .filter(|p| p.loss.is_some_and(|l| l > 0.0))
                    .map(|p| [p.at as f64 - w.now, 0.0])
                    .collect();
                if !lost.is_empty() {
                    pu.points(
                        egui_plot::Points::new(format!("{name} loss"), lost)
                            .color(theme::crit())
                            .radius(2.5),
                    );
                }
            }
            pu.pointer_coordinate().map(|p| p.x)
        });
    let lines: Vec<_> = series
        .iter()
        .map(|(n, c, p)| (n.clone(), *c, p.as_slice()))
        .collect();
    hover_values(shown.response, shown.inner, &lines, w, &|v| {
        model::rtt_text(v)
    });
}

/// Shades the part of the window before the first recorded point, so a 24h view of 20 minutes of data reads as such.
fn unrecorded<'a>(
    pu: &mut egui_plot::PlotUi,
    w: Window,
    series: impl Iterator<Item = &'a [Point]>,
) {
    let first = series
        .filter_map(|p| p.first().map(|p| p.at as f64 - w.now))
        .reduce(f64::min)
        .unwrap_or(0.0);
    if first > -w.secs + w.secs * 0.01 {
        pu.span(
            egui_plot::Span::new("", -w.secs..=first)
                .fill(gray(theme::text()))
                .border_width(0.0),
        );
    }
}

/// Time with nothing to draw, not measured or before History began: plain gray, unlabelled.
fn gray(t: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(t.r(), t.g(), t.b(), 14)
}

/// One id per plot name, so its memory (bounds) can be found again.
fn plot_id(name: &str) -> egui::Id {
    egui::Id::unique(("plot", name))
}

fn base_plot(id: &str, height: f32, w: Window) -> Plot<'static> {
    Plot::new(id.to_owned())
        .id(plot_id(id))
        .include_x(-w.secs)
        .include_x(0.0)
        .height(height)
        .show_axes([false, true])
        .show_grid([false, true])
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .show_background(false)
        .y_axis_min_width(44.0)
        .set_margin_fraction(vec2(0.0, 0.08))
        // The hover tooltip gives each line's value instead (`hover_values`).
        .show_crosshair(false)
        .show_x(false)
        .show_y(false)
}

pub fn severity_dot(s: Severity) -> Color32 {
    severity_color(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plot_keeps_its_axis_however_many_frames_it_draws() {
        let mut h = History::open(None, config::Config::default().history_limits());
        let now = 1_800_000_000;
        fixtures::populate(&mut h, &fixtures::all().remove(2), now);
        let mut config = config::Config::default();
        let mut panel = Panel::default();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(460.0, 1080.0))
            .allow_missing_glyphs()
            .with_accessibility_check(false)
            .build_ui(|ui| {
                let view = View {
                    history: &h,
                    findings: &[],
                    processes: &[],
                    adapters: &[],
                    links: &[],
                    public: None,
                    now,
                    elevated: false,
                    config_error: None,
                    notice: None,
                };
                panel.show(ui, &view, &mut config);
            });
        harness.run_steps(300);
        let top = |ctx: &egui::Context| {
            egui_plot::PlotMemory::load(ctx, plot_id("latency")).map(|m| m.bounds().max()[1])
        };
        let ctx = harness.ctx.clone();
        let y = top(&ctx).expect("the latency plot drew");
        // The fixture's slowest reply is about 130 ms; 8% margin on top of that is all.
        assert!(y < 1000.0, "latency axis grew to {y}");
    }

    #[test]
    fn hovering_reads_each_line_where_the_pointer_is() {
        let p = |at, state, value| Point {
            at,
            w: 15,
            state,
            value,
            loss: Some(0.0),
            provisional: false,
        };
        let a = [
            p(970, State::Value, Some(12.0)),
            p(985, State::Paused, None),
        ];
        let b = [p(970, State::Lost, None)];
        let lines = [
            ("a".to_string(), Color32::RED, &a[..]),
            ("b".to_string(), Color32::BLUE, &b[..]),
        ];
        let at = |x| {
            values_at(&lines, x, 1000.0)
                .into_iter()
                .map(|r| r.2)
                .collect::<Vec<_>>()
        };
        assert_eq!(at(-25.0), [Some(Shown::Value(12.0)), Some(Shown::Lost)]);
        assert_eq!(at(-10.0), [Some(Shown::Paused), None]);
        assert_eq!(ago(25.4), "25 s ago");
        assert_eq!(ago(3600.0), "60 min ago");
    }

    #[test]
    fn a_link_no_longer_up_is_named_after_its_adapter() {
        let up = Link {
            id: LinkId {
                luid: 7,
                gateway: Some("192.168.1.1".parse().unwrap()),
                gateway_mac: Some([1, 2, 3, 4, 5, 6]),
            },
            alias: "Wi-Fi".into(),
            kind: model::LinkKind::WiFi,
            index_v4: 1,
            index_v6: 1,
            source_v4: None,
            source_v6: None,
            default_route: true,
        };
        let before = LinkId {
            gateway_mac: None,
            ..up.id.clone()
        };
        let gone = LinkId {
            luid: 9,
            ..before.clone()
        };
        let links = [up.clone()];
        assert_eq!(link_alias(&up.id, &links), "Wi-Fi");
        assert_eq!(link_alias(&before, &links), "Wi-Fi (earlier)");
        assert_eq!(link_alias(&gone, &links), "earlier network");
    }

    #[test]
    fn a_picked_link_s_throughput_is_its_own() {
        let l = LinkId {
            luid: 2,
            gateway: None,
            gateway_mac: None,
        };
        let mut p = Panel::default();
        assert_eq!(p.metric_key(Metric::NetDown), Key::Metric(Metric::NetDown));
        p.link = Some(l.clone());
        assert_eq!(
            p.metric_key(Metric::NetUp),
            Key::LinkMetric(l, Metric::NetUp)
        );
        assert_eq!(p.metric_key(Metric::CpuBusy), Key::Metric(Metric::CpuBusy));
    }

    #[test]
    fn public_address_lines_follow_the_spec() {
        use probe::public::{Reading, Source};
        let a = |s: &str| s.parse().unwrap();
        let agree = Reading::Agree {
            address: a("198.51.100.7"),
            asn: Some("64500".into()),
        };
        assert_eq!(
            public_line(false, &agree),
            ("198.51.100.7 · AS64500".into(), false)
        );
        let differ = Reading::Differ {
            dns: a("198.51.100.7"),
            https: a("203.0.113.9"),
            asn: None,
        };
        assert_eq!(
            public_line(false, &differ),
            (
                "DNS 198.51.100.7, HTTPS 203.0.113.9: different paths out".into(),
                true
            )
        );
        assert_eq!(
            public_line(true, &Reading::Nat64(a("198.51.100.7"))),
            ("IPv6-only network with NAT64".into(), false)
        );
        assert_eq!(
            public_line(true, &Reading::None),
            ("No IPv6 internet".into(), false)
        );
        let single = Reading::Single {
            address: a("2001:db8::1"),
            source: Source::Dns,
            asn: None,
        };
        assert_eq!(
            public_line(true, &single),
            ("2001:db8::1 (DNS only)".into(), false)
        );
    }

    #[test]
    fn reselecting_the_open_tab_scrolls_to_top() {
        let mut p = Panel::default();
        p.select(Tab::Network);
        assert!(!p.scroll_to_top);
        p.select(Tab::Network);
        assert!(p.scroll_to_top);
    }
}
