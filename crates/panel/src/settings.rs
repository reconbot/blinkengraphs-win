use crate::{View, theme};
use config::{Adapter, Config, MenuIcon, Mode, ProbeGroup, ProbeView, Surface, TargetRow};
use egui::{RichText, Ui};
use model::Aggregate;
use model::{Probe, Readout, Section, Target};
use palette::{Palette, Preset, Rgb, Shade};

/// Returns true when `config` changed. Sets `export` when the user asks for a CSV.
pub fn show(ui: &mut Ui, config: &mut Config, view: &View, export: &mut bool) -> bool {
    let before = config.clone();

    group(ui, "General", |ui| {
        ui.horizontal(|ui| {
            ui.label("Taskbar display");
            ui.radio_value(&mut config.surface, Surface::TaskbarStrip, "Strip");
            ui.radio_value(&mut config.surface, Surface::TrayIcons, "Tray Icons");
        });
        ui.add_enabled_ui(config.surface == Surface::TaskbarStrip, |ui| {
            ui.horizontal(|ui| {
                ui.label("Tray icon");
                egui::ComboBox::from_id_salt("menu_icon")
                    .selected_text(config.menu_icon.label())
                    .show_ui(ui, |ui| {
                        let choices = [MenuIcon::App, MenuIcon::Verdict]
                            .into_iter()
                            .chain(Section::ALL.map(MenuIcon::Meter));
                        for m in choices {
                            ui.selectable_value(&mut config.menu_icon, m, m.label());
                        }
                    });
            })
            .response
            .on_disabled_hover_text("With Tray Icons, each Section has its own icon");
        });
        ui.horizontal(|ui| {
            ui.label("Theme");
            let mode = &mut config.appearance.mode;
            ui.radio_value(mode, Mode::System, "System");
            ui.radio_value(mode, Mode::Light, "Light");
            ui.radio_value(mode, Mode::Dark, "Dark");
        });
        ui.horizontal(|ui| {
            ui.label("Colors");
            let preset = &mut config.appearance.preset;
            egui::ComboBox::from_id_salt("preset")
                .selected_text(preset.label())
                .show_ui(ui, |ui| {
                    for p in Preset::ALL {
                        ui.selectable_value(preset, p, p.label());
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Graph values");
            egui::ComboBox::from_id_salt("graph_aggregate")
                .selected_text(config.graph_aggregate.label())
                .show_ui(ui, |ui| {
                    for a in Aggregate::ALL {
                        ui.selectable_value(&mut config.graph_aggregate, a, a.label());
                    }
                });
        })
        .response
        .on_hover_text("How each point of a history graph sums up the samples it covers");
        ui.horizontal(|ui| {
            ui.label("Panel hotkey");
            ui.text_edit_singleline(&mut config.hotkey);
        });
        ui.checkbox(
            &mut config.pause_when_idle,
            "Pause while locked or screen saver running",
        );
    });

    for section in Section::ALL {
        ui.push_id(("monitor", section), |ui| monitor(ui, config, section));
    }

    group(ui, "History", |ui| {
        ui.horizontal(|ui| {
            ui.label("Keep history for");
            ui.add(
                egui::DragValue::new(&mut config.history.rollup_days)
                    .range(1..=config::HistoryConfig::MAX_DAYS)
                    .suffix(" days"),
            );
        });
        let series = model::Metric::ALL.len()
            + config
                .groups
                .iter()
                .filter(|g| g.enabled)
                .map(|g| g.latency_probes().len() + g.dns_probes().len())
                .sum::<usize>();
        let measured = view.history.storage();
        let estimate = history::estimate_bytes(series, config.history.rollup_days, measured);
        let mut text = format!(
            "About {} on disk for {series} series at one row per minute.",
            verdict::bytes(estimate as f64)
        );
        if let Some(s) = measured {
            text.push_str(&format!(" Using {} now.", verdict::bytes(s.bytes as f64)));
        }
        ui.label(RichText::new(text).small().weak());
        ui.horizontal(|ui| {
            if ui
                .button("Export all to CSV")
                .on_hover_text(
                    "Every stored minute of every series, written to your Downloads folder",
                )
                .clicked()
            {
                *export = true;
            }
            if let Some(n) = view.notice {
                ui.label(RichText::new(n).small().weak());
            }
        });
    });

    group(ui, "Sampling", |ui| {
        interval(ui, "System metrics", &mut config.system_interval_ms);
        interval(
            ui,
            "Process list (Panel open)",
            &mut config.process_interval_ms,
        );
        ui.horizontal(|ui| {
            ui.label("Network adapter");
            let current = match &config.adapter {
                Adapter::FollowDefaultRoute => "Follow default route".to_string(),
                Adapter::Pinned(a) => a.clone(),
            };
            egui::ComboBox::from_id_salt("adapter")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut config.adapter,
                        Adapter::FollowDefaultRoute,
                        "Follow default route",
                    );
                    for a in view.adapters {
                        ui.selectable_value(&mut config.adapter, Adapter::Pinned(a.clone()), a);
                    }
                });
        });
    });

    let mut remove_group = None;
    for (gi, g) in config.groups.iter_mut().enumerate() {
        ui.push_id(("group", gi), |ui| {
            group(ui, "Probe Group", |ui| {
                probe_group(ui, g, &mut remove_group, gi)
            });
        });
    }
    if let Some(i) = remove_group {
        config.groups.remove(i);
    }
    if ui.button("Add Probe Group").clicked() {
        let n = config.groups.len() + 1;
        let mut row = TargetRow::new(Target::from("1.1.1.1".to_string()));
        row.icmp = true;
        config
            .groups
            .push(ProbeGroup::new(format!("Group {n}"), vec![row]));
    }

    *config != before
}

const MONITOR_WIDTHS: [f32; 4] = [0.0, 44.0, 44.0, 44.0];

/// One card per Section: which Readouts each Surface shows.
fn monitor(ui: &mut Ui, config: &mut Config, section: Section) {
    let title = RichText::new(section.label())
        .color(theme::accent(section))
        .strong();
    group(ui, title, |ui| {
        let shade = if ui.visuals().dark_mode {
            Shade::Dark
        } else {
            Shade::Light
        };
        let base =
            Palette::new(config.appearance.preset, shade, &Default::default()).section(section);
        color_row(
            ui,
            "Color",
            &mut config.appearance.overrides.sections,
            section,
            base,
        );
        if section == Section::Latency {
            ui.horizontal(|ui| {
                ui.label("Probe");
                let all: Vec<Probe> = config
                    .groups
                    .iter()
                    .flat_map(|g| g.latency_probes())
                    .collect();
                egui::ComboBox::from_id_salt("primary")
                    .selected_text(config.primary.to_string())
                    .show_ui(ui, |ui| {
                        for p in all {
                            let label = p.to_string();
                            ui.selectable_value(&mut config.primary, p, label);
                        }
                    });
            });
        }
        crate::table_cells(ui, &MONITOR_WIDTHS, false, |i, ui| {
            let h = ["", "Tray", "Strip", "Panel"][i];
            ui.label(RichText::new(h).small().weak());
        });
        let c = config.section_mut(section);
        for (ri, r) in Readout::ALL
            .into_iter()
            .filter(|r| r.section() == section)
            .enumerate()
        {
            crate::table_cells(ui, &MONITOR_WIDTHS, ri % 2 == 1, |i, ui| match i {
                0 => {
                    ui.label(r.label());
                }
                1 => toggle(ui, &mut c.tray, r),
                2 => toggle(ui, &mut c.strip, r),
                _ => toggle(ui, &mut c.panel, r),
            });
        }
        if section == Section::Latency {
            ui.add_space(6.0);
            probe_views(ui, config);
        }
    });
}

/// A color button; picking stores an override, Reset removes it.
fn color_row<K: Ord + Clone>(
    ui: &mut Ui,
    label: &str,
    overrides: &mut std::collections::BTreeMap<K, Rgb>,
    key: K,
    base: Rgb,
) {
    ui.horizontal(|ui| {
        if !label.is_empty() {
            ui.label(label);
        }
        let current = overrides.get(&key).copied().unwrap_or(base);
        let mut rgb = [current.0, current.1, current.2];
        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
            overrides.insert(key.clone(), Rgb(rgb[0], rgb[1], rgb[2]));
        }
        if overrides.contains_key(&key) && ui.small_button("Reset").clicked() {
            overrides.remove(&key);
        }
    });
}

const VIEW_WIDTHS: [f32; 4] = [0.0, 70.0, 64.0, 56.0];

/// Color, window and aggregate for every latency Probe, as shown in the Panel and the Latency Meter.
fn probe_views(ui: &mut Ui, config: &mut Config) {
    let probes: Vec<Probe> = config
        .groups
        .iter()
        .flat_map(|g| g.latency_probes())
        .collect();
    let shade = if ui.visuals().dark_mode {
        Shade::Dark
    } else {
        Shade::Light
    };
    let plain = Palette::new(config.appearance.preset, shade, &Default::default());
    crate::table_cells(ui, &VIEW_WIDTHS, false, |i, ui| {
        let h = ["Probe", "Color", "Window", "Value"][i];
        ui.label(RichText::new(h).small().weak());
    });
    for (i, p) in probes.iter().enumerate() {
        let key = p.to_string();
        ui.push_id(("view", i), |ui| {
            crate::table_cells(ui, &VIEW_WIDTHS, i % 2 == 1, |c, ui| match c {
                0 => {
                    ui.add(egui::Label::new(&key).truncate());
                }
                1 => color_row(
                    ui,
                    "",
                    &mut config.appearance.overrides.probes,
                    key.clone(),
                    plain.series(i),
                ),
                2 => {
                    let mut v = config.probe_view(p);
                    egui::ComboBox::from_id_salt("window")
                        .width(56.0)
                        .selected_text(ProbeView::window_label(v.window_secs))
                        .show_ui(ui, |ui| {
                            for w in ProbeView::WINDOWS {
                                ui.selectable_value(
                                    &mut v.window_secs,
                                    w,
                                    ProbeView::window_label(w),
                                );
                            }
                        });
                    store(config, &key, v);
                }
                _ => {
                    let mut v = config.probe_view(p);
                    egui::ComboBox::from_id_salt("aggregate")
                        .width(44.0)
                        .selected_text(v.aggregate.label())
                        .show_ui(ui, |ui| {
                            for a in Aggregate::ALL {
                                ui.selectable_value(&mut v.aggregate, a, a.label());
                            }
                        });
                    store(config, &key, v);
                }
            });
        });
    }
}

/// Default views are not written, so the TOML lists only Probes the user changed.
fn store(config: &mut Config, key: &str, v: ProbeView) {
    if v == ProbeView::default() {
        config.probe_views.remove(key);
    } else {
        config.probe_views.insert(key.to_string(), v);
    }
}

fn group(ui: &mut Ui, title: impl Into<RichText>, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::card_fill())
        .stroke(egui::Stroke::new(1.0, theme::card_edge()))
        .corner_radius(10.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(title.into().strong());
            ui.add_space(4.0);
            body(ui);
        });
    ui.add_space(8.0);
}

fn toggle(ui: &mut Ui, list: &mut Vec<Readout>, m: Readout) {
    let mut on = list.contains(&m);
    if ui.add(egui::Checkbox::without_text(&mut on)).changed() {
        if on {
            list.push(m);
            list.sort_by_key(|x| Readout::ALL.iter().position(|y| y == x));
        } else {
            list.retain(|x| *x != m);
        }
    }
}

fn interval(ui: &mut Ui, label: &str, ms: &mut u64) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut secs = *ms as f64 / 1000.0;
        if ui
            .add(
                egui::DragValue::new(&mut secs)
                    .range(0.25..=60.0)
                    .speed(0.25)
                    .suffix(" s"),
            )
            .changed()
        {
            *ms = (secs * 1000.0).round() as u64;
        }
    });
}

// TCP and UDP fit a checkbox and a five-digit port.
const WIDTHS: [f32; 7] = [0.0, 32.0, 68.0, 68.0, 30.0, 34.0, 16.0];

fn probe_group(ui: &mut Ui, g: &mut ProbeGroup, remove_group: &mut Option<usize>, gi: usize) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(160.0));
        ui.checkbox(&mut g.enabled, "Enabled");
        if ui.small_button("Remove group").clicked() {
            *remove_group = Some(gi);
        }
    });
    ui.horizontal(|ui| {
        interval(ui, "Latency every", &mut g.latency_ms);
        interval(ui, "DNS", &mut g.dns_ms);
        interval(ui, "Trace", &mut g.trace_ms);
    });
    ui.add_space(4.0);
    crate::table_cells(ui, &WIDTHS, false, |i, ui| {
        let h = ["Target", "ICMP", "TCP", "UDP", "DNS", "Trace", ""][i];
        let text = RichText::new(h).small().weak();
        // Over the checkbox, which starts the TCP and UDP cells.
        let label = if matches!(i, 2 | 3) {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(text)
            })
            .inner
        } else {
            ui.label(text)
        };
        match i {
            2 => {
                label.on_hover_text("Tick to time a TCP handshake to the port.");
            }
            3 => {
                label.on_hover_text(
                    "Tick to send a UDP datagram to the port. A closed port works: the host answers port-unreachable.",
                );
            }
            _ => {}
        }
    });
    let mut remove = None;
    for (ri, row) in g.targets.iter_mut().enumerate() {
        ui.push_id(ri, |ui| {
            let traceable = !matches!(row.target, Target::Gateway | Target::SystemResolver);
            crate::table_cells(ui, &WIDTHS, ri % 2 == 1, |i, ui| match i {
                0 => {
                    let mut text = row.target.to_string();
                    if ui
                        .add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY))
                        .changed()
                    {
                        row.target = Target::from(text);
                    }
                }
                1 => {
                    ui.add(egui::Checkbox::without_text(&mut row.icmp));
                }
                2 => port(ui, &mut row.tcp, TCP_PORT),
                3 => port(ui, &mut row.udp, UDP_PORT),
                4 => {
                    ui.add(egui::Checkbox::without_text(&mut row.dns));
                }
                5 => {
                    ui.add_enabled(traceable, egui::Checkbox::without_text(&mut row.trace))
                        .on_disabled_hover_text("Hop traces need an internet target");
                }
                _ => {
                    if ui
                        .small_button("×")
                        .on_hover_text("Remove target")
                        .clicked()
                    {
                        remove = Some(ri);
                    }
                }
            });
        });
    }
    if let Some(i) = remove {
        g.targets.remove(i);
    }
    if ui.small_button("+ Add target").clicked() {
        let mut row = TargetRow::new(Target::from("example.com".to_string()));
        row.icmp = true;
        g.targets.push(row);
    }
}

/// The port a Probe starts on when ticked: HTTPS, and the first port traceroute uses, which
/// is almost always closed and so draws port-unreachable.
const TCP_PORT: u16 = 443;
const UDP_PORT: u16 = 33434;

/// A checkbox turns the Probe on and shows its port beside it; unticking hides the port, which
/// comes back as it was if ticked again this session. A text field with an "off" hint didn't
/// read as an input.
fn port(ui: &mut Ui, value: &mut Option<u16>, default: u16) {
    let remembered = ui.make_persistent_id(("port", default));
    let mut on = value.is_some();
    let mut port = value.unwrap_or_else(|| ui.data(|d| d.get_temp(remembered)).unwrap_or(default));
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        // The theme's 10 pt button padding and 40 pt minimum widget width overflow the column.
        let spacing = ui.spacing_mut();
        spacing.item_spacing.x = 4.0;
        spacing.button_padding = egui::vec2(3.0, 1.0);
        spacing.interact_size.x = 0.0;
        ui.add(egui::Checkbox::without_text(&mut on));
        if on {
            ui.add_sized(
                [ui.available_width(), 18.0],
                egui::DragValue::new(&mut port)
                    .range(1..=u16::MAX)
                    .speed(1.0),
            )
            .on_hover_text("Port. Click to type, or drag.");
        }
    });
    ui.data_mut(|d| d.insert_temp(remembered, port));
    *value = probe_port(on, port);
}

fn probe_port(on: bool, port: u16) -> Option<u16> {
    (on && port > 0).then_some(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unticked_port_measures_nothing() {
        assert_eq!(probe_port(false, 443), None);
        assert_eq!(probe_port(true, 443), Some(443));
        assert_eq!(probe_port(true, 0), None);
    }
}
