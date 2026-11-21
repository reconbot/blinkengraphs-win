use crate::{View, theme};
use config::{Adapter, Config, ProbeGroup, Surface};
use egui::{RichText, Ui};
use model::{Metric, Probe, Protocol, Section, Target};

/// Returns true when `config` changed.
pub fn show(ui: &mut Ui, config: &mut Config, view: &View) -> bool {
    let before = config.clone();

    group(ui, "Display", |ui| {
        ui.horizontal(|ui| {
            ui.label("Surface");
            ui.radio_value(&mut config.surface, Surface::TrayIcons, "Tray Icons");
            ui.radio_value(&mut config.surface, Surface::TaskbarStrip, "Taskbar Strip");
        });
        egui::Grid::new("metrics")
            .num_columns(4)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                for h in ["Section / Metric", "Visible", "Meter", "Strip"] {
                    ui.label(RichText::new(h).small().weak());
                }
                ui.end_row();
                for section in Section::ALL {
                    let c = config.section_mut(section);
                    ui.label(
                        RichText::new(section.label())
                            .color(theme::accent(section))
                            .strong(),
                    );
                    ui.checkbox(&mut c.visible, "");
                    ui.end_row();
                    for m in Metric::ALL.into_iter().filter(|m| m.section() == section) {
                        ui.label(format!("   {}", m.label()));
                        ui.label("");
                        toggle(ui, &mut c.meter, m);
                        toggle(ui, &mut c.strip, m);
                        ui.end_row();
                    }
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
        ui.checkbox(
            &mut config.pause_when_idle,
            "Pause while locked or screen saver running",
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
        ui.horizontal(|ui| {
            ui.label("Panel hotkey");
            ui.text_edit_singleline(&mut config.hotkey);
        });
    });

    group(ui, "Latency Meter", |ui| {
        let all: Vec<Probe> = config
            .groups
            .iter()
            .flat_map(|g| g.probes.iter().cloned())
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

    let mut remove_group = None;
    for (gi, g) in config.groups.iter_mut().enumerate() {
        group(ui, &format!("Probe Group · {}", g.name), |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut g.enabled, "Enabled");
                interval(ui, "every", &mut g.interval_ms);
                if ui.small_button("Remove group").clicked() {
                    remove_group = Some(gi);
                }
            });
            probe_list(
                ui,
                ("p", gi),
                "Probes",
                &mut g.probes,
                &["icmp", "tcp:443", "tcp:80", "dns"],
            );
            probe_list(
                ui,
                ("t", gi),
                "Hop Traces",
                &mut g.traces,
                &["icmp", "tcp:443"],
            );
        });
    }
    if let Some(i) = remove_group {
        config.groups.remove(i);
    }
    if ui.button("Add Probe Group").clicked() {
        config.groups.push(ProbeGroup {
            name: format!("Group {}", config.groups.len() + 1),
            enabled: true,
            interval_ms: 1000,
            probes: vec![],
            traces: vec![],
        });
    }

    *config != before
}

fn group(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::CARD)
        .stroke(egui::Stroke::new(1.0, theme::CARD_EDGE))
        .corner_radius(10.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.add_space(4.0);
            body(ui);
        });
    ui.add_space(8.0);
}

fn toggle(ui: &mut Ui, list: &mut Vec<Metric>, m: Metric) {
    let mut on = list.contains(&m);
    if ui.checkbox(&mut on, "").changed() {
        if on {
            list.push(m);
            list.sort_by_key(|x| Metric::ALL.iter().position(|y| y == x));
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

fn probe_list(
    ui: &mut Ui,
    id: (&str, usize),
    title: &str,
    probes: &mut Vec<Probe>,
    protocols: &[&str],
) {
    ui.label(RichText::new(title).small().weak());
    let mut remove = None;
    for (i, p) in probes.iter_mut().enumerate() {
        ui.push_id((id, i), |ui| {
            ui.horizontal(|ui| {
                let mut proto = p.protocol.to_string();
                egui::ComboBox::from_id_salt("proto")
                    .width(70.0)
                    .selected_text(&proto)
                    .show_ui(ui, |ui| {
                        for o in protocols {
                            ui.selectable_value(&mut proto, o.to_string(), *o);
                        }
                    });
                if let Ok(np) = Protocol::try_from(proto) {
                    p.protocol = np;
                }
                let mut target = p.target.to_string();
                if ui
                    .add(egui::TextEdit::singleline(&mut target).desired_width(150.0))
                    .changed()
                {
                    p.target = Target::from(target);
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        probes.remove(i);
    }
    if ui.small_button("+ Add").clicked() {
        probes.push(Probe {
            target: Target::from("1.1.1.1".to_string()),
            protocol: Protocol::Icmp,
        });
    }
}
