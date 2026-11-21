use crate::{View, theme};
use config::{Adapter, Config, MenuIcon, ProbeGroup, Surface, TargetRow};
use egui::{RichText, Ui};
use model::{Probe, Readout, Section, Target};

/// Returns true when `config` changed.
pub fn show(ui: &mut Ui, config: &mut Config, view: &View) -> bool {
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
    });
}

fn group(ui: &mut Ui, title: impl Into<RichText>, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::CARD)
        .stroke(egui::Stroke::new(1.0, theme::CARD_EDGE))
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

const WIDTHS: [f32; 7] = [0.0, 32.0, 52.0, 52.0, 30.0, 34.0, 16.0];

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
        let label = ui.label(RichText::new(h).small().weak());
        if i == 3 {
            label.on_hover_text(
                "A closed port works: the host answers port-unreachable. Try 33434.",
            );
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
                2 => port(ui, &mut row.tcp),
                3 => port(ui, &mut row.udp),
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

/// Empty means off.
fn port(ui: &mut Ui, value: &mut Option<u16>) {
    let mut text = value.map(|p| p.to_string()).unwrap_or_default();
    let edit = egui::TextEdit::singleline(&mut text)
        .hint_text("off")
        .desired_width(f32::INFINITY)
        .align(egui::Align2::RIGHT_CENTER);
    if ui.add(edit).changed() {
        match text.trim().parse::<u16>() {
            Ok(p) if p > 0 => *value = Some(p),
            _ if text.trim().is_empty() => *value = None,
            _ => {}
        }
    }
}
