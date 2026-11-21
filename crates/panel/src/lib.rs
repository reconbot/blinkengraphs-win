mod settings;
pub mod theme;

use config::{Config, SectionConfig};
use egui::{Align, Color32, Layout, RichText, Ui, vec2};
use egui_plot::{Line, Plot, PlotPoints};
use history::{History, Key, Point};
use model::{Metric, Ping, Probe, ProcessRow, Readout, Section, Unit};
use theme::{accent, card, severity_color};
use verdict::{Finding, Severity, bytes};

pub struct View<'a> {
    pub history: &'a History,
    pub findings: &'a [Finding],
    pub processes: &'a [ProcessRow],
    pub adapters: &'a [String],
    pub now: u64,
    pub elevated: bool,
    pub config_error: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ConfigChanged,
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
}

impl Range {
    const ALL: [Range; 5] = [
        Range::Min1,
        Range::Min15,
        Range::Hour,
        Range::Day,
        Range::Week,
    ];

    fn secs(self) -> u64 {
        match self {
            Range::Min1 => 60,
            Range::Min15 => 900,
            Range::Hour => 3600,
            Range::Day => 86_400,
            Range::Week => 604_800,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Range::Min1 => "1m",
            Range::Min15 => "15m",
            Range::Hour => "1h",
            Range::Day => "24h",
            Range::Week => "7d",
        }
    }
}

pub struct Panel {
    pub tab: Tab,
    pub range: Range,
    pub sort: ProcessSort,
    scroll_to_top: bool,
    graph_aggregate: model::Aggregate,
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
            graph_aggregate: model::Aggregate::Avg,
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
                        if settings::show(ui, config, view) {
                            actions.push(Action::ConfigChanged);
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
                (m.label(), color, self.points(view, &Key::Metric(*m)))
            })
            .collect();
        let fmt = move |v: f64| match primary.unit() {
            Unit::BytesPerSec => bytes(v),
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
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Latency")
                    .color(accent(Section::Latency))
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(p) = probes.first() {
                    let v = config.probe_view(p);
                    let st = view.history.aggregate(p, v.window_secs, v.aggregate);
                    let rtt = st.value.map(|r| format!("{r:.0} ms")).unwrap_or("—".into());
                    let text = match (latency, loss) {
                        (true, true) => format!("{rtt}  {:.0}% loss", st.loss * 100.0),
                        (true, false) => rtt,
                        _ => format!("{:.0}% loss", st.loss * 100.0),
                    };
                    ui.label(RichText::new(text).size(18.0).strong());
                }
            });
        });
        let series: Vec<_> = probes
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    p.to_string(),
                    theme::probe(p, i),
                    self.points(view, &Key::Probe(p.clone())),
                )
            })
            .collect();
        let names: Vec<(String, Color32, Vec<Point>)> = series;
        multiplot(
            ui,
            "latency",
            &names,
            if all { 180.0 } else { 110.0 },
            self.window(view),
        );
        ui.horizontal_wrapped(|ui| {
            for (i, p) in probes.iter().enumerate() {
                let v = config.probe_view(p);
                let sum = view.history.aggregate(p, v.window_secs, v.aggregate);
                let st = view.history.stat(p, history::RECENT_SECS);
                let text = format!(
                    "{p}  {} {}  {:.0}%{}",
                    v.aggregate.label(),
                    sum.value
                        .map(|r| format!("{r:.1} ms"))
                        .unwrap_or("—".into()),
                    sum.loss * 100.0,
                    st.jitter_ms
                        .map(|j| format!("  ±{j:.1}"))
                        .unwrap_or_default()
                );
                dot(ui, theme::probe(p, i), 3.5);
                ui.label(RichText::new(text).small().color(theme::probe(p, i)));
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
    }

    fn window(&self, view: &View) -> Window {
        Window {
            now: view.now as f64,
            secs: self.range.secs() as f64,
        }
    }

    fn points(&self, view: &View, key: &Key) -> Vec<Point> {
        let secs = self.range.secs();
        view.history.series(
            key,
            view.now.saturating_sub(secs),
            view.now,
            (secs / GRAPH_POINTS).max(1),
            self.graph_aggregate,
        )
    }
}

/// Buckets per graph: about one per pixel of a half-width card.
const GRAPH_POINTS: u64 = 240;

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
    let ms = |v: Option<f32>| v.map(|v| format!("{v:.1}")).unwrap_or("—".into());
    let cols: [(f32, &str); 7] = [
        (22.0, "#"),
        (0.0, "Address"),
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
            (
                0.0,
                RichText::new(hop.addr.map(|a| a.to_string()).unwrap_or("???".into())),
            ),
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
}

fn to_line(
    name: impl Into<String>,
    color: Color32,
    pts: &[Point],
    fill: bool,
    w: Window,
) -> Line<'static> {
    let xy: Vec<[f64; 2]> = pts
        .iter()
        .filter(|p| p.value.is_finite())
        .map(|p| [p.at as f64 - w.now, p.value as f64])
        .collect();
    let line = Line::new(name, PlotPoints::from(xy))
        .color(color)
        .width(1.5);
    if fill {
        line.fill(0.0).fill_alpha(0.18)
    } else {
        line
    }
}

fn sparkplot(
    ui: &mut Ui,
    id: &str,
    series: Vec<(&str, Color32, Vec<Point>)>,
    max: Option<f64>,
    height: f32,
    fmt: impl Fn(f64) -> String + 'static,
    w: Window,
) {
    let mut plot = base_plot(id, height, w)
        .include_y(0.0)
        .y_axis_formatter(move |m, _| fmt(m.value));
    if let Some(m) = max {
        plot = plot.include_y(m);
    }
    plot.show(ui, |pu| {
        unrecorded(pu, w, series.iter().map(|s| s.2.as_slice()));
        for (name, color, pts) in &series {
            pu.line(to_line(*name, *color, pts, true, w));
        }
    });
}

fn multiplot(
    ui: &mut Ui,
    id: &str,
    series: &[(String, Color32, Vec<Point>)],
    height: f32,
    w: Window,
) {
    base_plot(id, height, w)
        .include_y(0.0)
        .y_axis_formatter(|m, _| format!("{:.0} ms", m.value))
        .show(ui, |pu| {
            unrecorded(pu, w, series.iter().map(|s| s.2.as_slice()));
            for (name, color, pts) in series {
                pu.line(to_line(name.clone(), *color, pts, false, w));
                let lost: Vec<[f64; 2]> = pts
                    .iter()
                    .filter(|p| p.loss > 0.0)
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
        // The label is drawn inside the span, so a narrow span gets none rather than a clipped one.
        let label = if first + w.secs > w.secs * 0.3 {
            "no data"
        } else {
            ""
        };
        let t = theme::text();
        pu.span(
            egui_plot::Span::new(label, -w.secs..=first)
                .fill(Color32::from_rgba_unmultiplied(t.r(), t.g(), t.b(), 10))
                .border_width(0.0),
        );
    }
}

fn base_plot(id: &str, height: f32, w: Window) -> Plot<'static> {
    Plot::new(id.to_owned())
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
}

pub fn severity_dot(s: Severity) -> Color32 {
    severity_color(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reselecting_the_open_tab_scrolls_to_top() {
        let mut p = Panel::default();
        p.select(Tab::Network);
        assert!(!p.scroll_to_top);
        p.select(Tab::Network);
        assert!(p.scroll_to_top);
    }
}
