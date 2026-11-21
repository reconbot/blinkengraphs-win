mod settings;
pub mod theme;

use config::Config;
use egui::{Align, Color32, Layout, RichText, Ui, vec2};
use egui_plot::{Line, Plot, PlotPoints};
use history::{History, Key, Point};
use model::{Metric, Probe, ProcessRow, Section, Unit};
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
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
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            tab: Tab::Overview,
            range: Range::Min1,
        }
    }
}

impl Panel {
    pub fn show(&mut self, ui: &mut Ui, view: &View, config: &mut Config) -> Vec<Action> {
        let mut actions = Vec::new();
        egui::CentralPanel::default()
            .frame(theme::window_frame())
            .show(ui, |ui| {
                self.header(ui, view, &mut actions);
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| match self.tab {
                        Tab::Overview => self.overview(ui, view),
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
                    ui.selectable_value(&mut self.tab, tab, label);
                }
            });
        });
        ui.add_space(6.0);
        verdict_banner(ui, view.findings);
        if let Some(e) = view.config_error {
            ui.colored_label(theme::WARN, format!("Config not loaded: {e}"));
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

    fn overview(&self, ui: &mut Ui, view: &View) {
        let s = view.history.last_sample();
        for row in [
            [Section::Cpu, Section::Memory],
            [Section::Network, Section::Disk],
        ] {
            ui.columns(2, |cols| {
                for (col, section) in cols.iter_mut().zip(row) {
                    card(col, section, |ui| self.section_card(ui, view, section, s));
                }
            });
            ui.add_space(8.0);
        }
        card(ui, Section::Latency, |ui| {
            self.latency_card(ui, view, false)
        });
        ui.add_space(8.0);
        processes(ui, view.processes);
    }

    fn section_card(&self, ui: &mut Ui, view: &View, section: Section, s: Option<&model::Sample>) {
        let metrics: Vec<Metric> = Metric::ALL
            .into_iter()
            .filter(|m| m.section() == section)
            .collect();
        let primary = metrics[0];
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(section.label())
                    .color(accent(section))
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let v = s.map(|s| s.get(primary)).unwrap_or(0.0);
                let text = if section == Section::Network {
                    let up = s.map(|s| s.get(Metric::NetUp)).unwrap_or(0.0);
                    format!("↓{}/s ↑{}/s", bytes(v), bytes(up))
                } else {
                    format_value(primary, v)
                };
                let size = if section == Section::Network {
                    14.0
                } else {
                    18.0
                };
                ui.label(RichText::new(text).size(size).strong());
            });
        });
        let mut lines = vec![(primary, accent(section))];
        if section == Section::Network {
            lines.push((Metric::NetUp, theme::UPLOAD));
        }
        let series: Vec<_> = lines
            .iter()
            .map(|(m, c)| (m.label(), *c, self.points(view, &Key::Metric(*m))))
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
        );
        ui.horizontal_wrapped(|ui| {
            for m in metrics
                .iter()
                .skip(1)
                .filter(|m| !(section == Section::Network && **m == Metric::NetUp))
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

    fn latency_card(&self, ui: &mut Ui, view: &View, all: bool) {
        let probes = latency_probes(view.history, all);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Latency")
                    .color(accent(Section::Latency))
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(p) = probes.first() {
                    let st = view.history.stat(p, history::RECENT_SECS);
                    let rtt = st
                        .rtt_ms
                        .map(|r| format!("{r:.0} ms"))
                        .unwrap_or("—".into());
                    ui.label(
                        RichText::new(format!("{rtt}  {:.0}% loss", st.loss * 100.0))
                            .size(18.0)
                            .strong(),
                    );
                }
            });
        });
        let series: Vec<_> = probes
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    p.to_string(),
                    theme::series(i),
                    self.points(view, &Key::Probe(p.clone())),
                )
            })
            .collect();
        let names: Vec<(String, Color32, Vec<Point>)> = series;
        multiplot(ui, "latency", &names, if all { 180.0 } else { 110.0 });
        ui.horizontal_wrapped(|ui| {
            for (i, p) in probes.iter().enumerate() {
                let st = view.history.stat(p, history::RECENT_SECS);
                let text = format!(
                    "{p}  {}  {:.0}%{}",
                    st.rtt_ms
                        .map(|r| format!("{r:.1} ms"))
                        .unwrap_or("—".into()),
                    st.loss * 100.0,
                    st.jitter_ms
                        .map(|j| format!("  ±{j:.1}"))
                        .unwrap_or_default()
                );
                dot(ui, theme::series(i), 3.5);
                ui.label(RichText::new(text).small().color(theme::series(i)));
            }
        });
    }

    fn network(&self, ui: &mut Ui, view: &View, config: &Config) {
        card(ui, Section::Latency, |ui| self.latency_card(ui, view, true));
        for trace in config
            .groups
            .iter()
            .filter(|g| g.enabled)
            .flat_map(|g| &g.traces)
        {
            ui.add_space(8.0);
            card(ui, Section::Latency, |ui| {
                hop_table(ui, view.history, trace)
            });
        }
    }

    fn points(&self, view: &View, key: &Key) -> Vec<Point> {
        view.history
            .series(key, view.now.saturating_sub(self.range.secs()), view.now)
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
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), egui::Sense::hover());
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
}

fn processes(ui: &mut Ui, rows: &[ProcessRow]) {
    egui::Frame::new()
        .fill(theme::CARD)
        .corner_radius(10.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Top processes").strong());
            if rows.is_empty() {
                ui.label(RichText::new("Sampling…").weak());
                return;
            }
            egui::Grid::new("procs")
                .num_columns(4)
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for h in ["Process", "CPU", "Memory", "Disk"] {
                        ui.label(RichText::new(h).small().weak());
                    }
                    ui.end_row();
                    for r in rows.iter().take(8) {
                        ui.label(&r.name);
                        ui.label(
                            RichText::new(format!("{:.1}%", r.cpu_pct))
                                .color(theme::level(Section::Cpu, r.cpu_pct / 100.0)),
                        );
                        ui.label(bytes(r.memory_bytes as f64));
                        ui.label(format!("{}/s", bytes(r.disk_bps)));
                        ui.end_row();
                    }
                });
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
    egui::Grid::new(("hops", trace.to_string()))
        .num_columns(7)
        .striped(true)
        .spacing([12.0, 3.0])
        .show(ui, |ui| {
            for c in ["#", "Address", "Loss", "Last", "Avg", "Best", "Worst"] {
                ui.label(RichText::new(c).small().weak());
            }
            ui.end_row();
            for hop in hops {
                ui.label(RichText::new(hop.ttl.to_string()).weak());
                ui.label(hop.addr.map(|a| a.to_string()).unwrap_or("???".into()));
                let loss_color = if hop.loss >= 0.2 {
                    theme::CRIT
                } else if hop.loss > 0.0 {
                    theme::WARN
                } else {
                    theme::TEXT_WEAK
                };
                ui.label(RichText::new(format!("{:.0}%", hop.loss * 100.0)).color(loss_color));
                ui.label(ms(hop.last_ms));
                ui.label(ms(hop.avg_ms));
                ui.label(ms(hop.best_ms));
                ui.label(ms(hop.worst_ms));
                ui.end_row();
            }
        });
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

fn to_line(name: impl Into<String>, color: Color32, pts: &[Point], fill: bool) -> Line<'static> {
    let newest = pts.last().map(|p| p.at).unwrap_or(0) as f64;
    let xy: Vec<[f64; 2]> = pts
        .iter()
        .filter(|p| p.mean.is_finite())
        .map(|p| [p.at as f64 - newest, p.mean as f64])
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
) {
    let mut plot = base_plot(id, height)
        .include_y(0.0)
        .y_axis_formatter(move |m, _| fmt(m.value));
    if let Some(m) = max {
        plot = plot.include_y(m);
    }
    plot.show(ui, |pu| {
        for (name, color, pts) in &series {
            pu.line(to_line(*name, *color, pts, true));
        }
    });
}

fn multiplot(ui: &mut Ui, id: &str, series: &[(String, Color32, Vec<Point>)], height: f32) {
    base_plot(id, height)
        .include_y(0.0)
        .y_axis_formatter(|m, _| format!("{:.0} ms", m.value))
        .show(ui, |pu| {
            for (name, color, pts) in series {
                pu.line(to_line(name.clone(), *color, pts, false));
                let newest = pts.last().map(|p| p.at).unwrap_or(0) as f64;
                let lost: Vec<[f64; 2]> = pts
                    .iter()
                    .filter(|p| p.loss > 0.0)
                    .map(|p| [p.at as f64 - newest, 0.0])
                    .collect();
                if !lost.is_empty() {
                    pu.points(
                        egui_plot::Points::new(format!("{name} loss"), lost)
                            .color(theme::CRIT)
                            .radius(2.5),
                    );
                }
            }
        });
}

fn base_plot(id: &str, height: f32) -> Plot<'static> {
    Plot::new(id.to_owned())
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
