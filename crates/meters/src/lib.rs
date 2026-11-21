use config::Config;
use history::{History, Key};
use model::{Metric, Section, Unit};
use render::{Cell, FontRef, Image, Meter, Taskbar, cube_root, meter};

/// Draws one Section's Meter at `px` from the last `px` samples, per the Section's configured Metrics.
pub fn draw(
    section: Section,
    h: &History,
    config: &Config,
    metrics: &[Metric],
    px: u32,
    taskbar: Taskbar,
) -> Image {
    let n = px as usize;
    match section {
        Section::Latency => {
            let raw = h.recent(&Key::Probe(config.primary.clone()), n);
            let baseline = h
                .stat(&config.primary, history::RECENT_SECS)
                .baseline_ms
                .unwrap_or(20.0);
            let scale = (baseline * 4.0).max(100.0);
            let v: Vec<Option<f32>> = raw
                .into_iter()
                .map(|r| r.map(|ms| (ms / scale).min(1.0)))
                .collect();
            meter(section, &Meter::Latency(&v), px, taskbar)
        }
        Section::Network
            if metrics.contains(&Metric::NetDown) && metrics.contains(&Metric::NetUp) =>
        {
            let down = h.recent(&Key::Metric(Metric::NetDown), n);
            let up = h.recent(&Key::Metric(Metric::NetUp), n);
            let max = down
                .iter()
                .chain(&up)
                .flatten()
                .fold(1e6f32, |a, b| a.max(*b)) as f64;
            let norm = |v: Vec<Option<f32>>| {
                v.into_iter()
                    .map(|x| cube_root(x.unwrap_or(0.0) as f64, max))
                    .collect::<Vec<_>>()
            };
            let (d, u) = (norm(down), norm(up));
            meter(section, &Meter::Duplex { down: &d, up: &u }, px, taskbar)
        }
        _ => {
            let Some(&m) = metrics.first() else {
                return meter(section, &Meter::Level(&[]), px, taskbar);
            };
            let raw = h.recent(&Key::Metric(m), n);
            let v = normalize(m, &raw);
            meter(section, &Meter::Level(&v), px, taskbar)
        }
    }
}

fn normalize(m: Metric, raw: &[Option<f32>]) -> Vec<f32> {
    let vals = raw.iter().map(|v| v.unwrap_or(0.0));
    match m.unit() {
        Unit::Percent => vals.map(|v| (v / 100.0).clamp(0.0, 1.0)).collect(),
        Unit::BytesPerSec | Unit::PerSec => {
            let max = raw.iter().flatten().fold(1.0f32, |a, b| a.max(*b)) as f64;
            vals.map(|v| cube_root(v as f64, max)).collect()
        }
        Unit::Millis => vals.map(|v| (v / 100.0).clamp(0.0, 1.0)).collect(),
        Unit::Count => vals.map(|v| (v / 8.0).clamp(0.0, 1.0)).collect(),
    }
}

type Owned = (Section, String, Vec<f32>, Vec<f32>, Vec<Option<f32>>);

/// One Strip cell per visible Section, from the Section's Strip Metrics.
pub fn strip(
    h: &History,
    config: &Config,
    height: u32,
    scale: f32,
    font: Option<&FontRef>,
    taskbar: Taskbar,
) -> Image {
    let n = 60usize;
    let mut owned: Vec<Owned> = Vec::new();
    for section in Section::ALL {
        let c = config.section(section);
        if !c.visible {
            continue;
        }
        let s = h.last_sample();
        let get = |m| s.map(|s| s.get(m)).unwrap_or(0.0);
        let (value, a, b, l) = match section {
            Section::Latency => {
                let st = h.stat(&config.primary, history::RECENT_SECS);
                let raw = h.recent(&Key::Probe(config.primary.clone()), n);
                let scale = (st.baseline_ms.unwrap_or(20.0) * 4.0).max(100.0);
                let v = raw
                    .into_iter()
                    .map(|r| r.map(|ms| (ms / scale).min(1.0)))
                    .collect();
                let text = match st.rtt_ms {
                    Some(r) if st.loss > 0.0 => format!("{r:.0}ms {:.0}%", st.loss * 100.0),
                    Some(r) => format!("{r:.0} ms"),
                    None => "no reply".into(),
                };
                (text, vec![], vec![], v)
            }
            Section::Network => {
                let down = h.recent(&Key::Metric(Metric::NetDown), n);
                let up = h.recent(&Key::Metric(Metric::NetUp), n);
                let max = down
                    .iter()
                    .chain(&up)
                    .flatten()
                    .fold(1e6f32, |a, b| a.max(*b)) as f64;
                let norm = |v: Vec<Option<f32>>| {
                    v.into_iter()
                        .map(|x| cube_root(x.unwrap_or(0.0) as f64, max))
                        .collect()
                };
                let text = verdict::bytes(get(Metric::NetDown) + get(Metric::NetUp));
                (text, norm(down), norm(up), vec![])
            }
            _ => {
                let m = c.strip.first().copied().unwrap_or_else(|| {
                    Metric::ALL
                        .into_iter()
                        .find(|m| m.section() == section)
                        .unwrap()
                });
                let v = normalize(m, &h.recent(&Key::Metric(m), n));
                let text = match m.unit() {
                    Unit::Percent => format!("{:.0}%", get(m)),
                    Unit::BytesPerSec => verdict::bytes(get(m)),
                    Unit::Millis => format!("{:.0} ms", get(m)),
                    _ => format!("{:.0}", get(m)),
                };
                (text, v, vec![], vec![])
            }
        };
        owned.push((section, value, a, b, l));
    }
    let cells: Vec<Cell> = owned
        .iter()
        .map(|(section, value, a, b, l)| Cell {
            section: *section,
            value: value.clone(),
            meter: match section {
                Section::Latency => Meter::Latency(l),
                Section::Network => Meter::Duplex { down: a, up: b },
                _ => Meter::Level(a),
            },
        })
        .collect();
    render::strip(&cells, height, scale, font, taskbar)
}

/// Windows truncates notification-area tooltips to 128 characters.
pub fn tooltip(
    section: Section,
    h: &History,
    config: &Config,
    findings: &[verdict::Finding],
) -> String {
    let s = h.last_sample();
    let get = |m| s.map(|s| s.get(m)).unwrap_or(0.0);
    let body = match section {
        Section::Cpu => format!(
            "CPU {:.0}%  kernel {:.0}%",
            get(Metric::CpuTotal),
            get(Metric::CpuKernel)
        ),
        Section::Memory => format!(
            "Commit {:.0}%  RAM {:.0}%  {:.0} faults/s",
            get(Metric::MemCommit),
            get(Metric::MemPhysical),
            get(Metric::MemHardFaults)
        ),
        Section::Network => format!(
            "Down {}/s  Up {}/s",
            verdict::bytes(get(Metric::NetDown)),
            verdict::bytes(get(Metric::NetUp))
        ),
        Section::Disk => format!(
            "Disk {:.0}% busy  {:.0} ms",
            get(Metric::DiskActive),
            get(Metric::DiskResponse)
        ),
        Section::Latency => {
            let st = h.stat(&config.primary, history::RECENT_SECS);
            let rtt = st
                .rtt_ms
                .map(|r| format!("{r:.0} ms"))
                .unwrap_or("no reply".into());
            format!("{} {rtt}  {:.0}% loss", config.primary, st.loss * 100.0)
        }
    };
    let headline = findings.first().map(|f| f.headline.as_str()).unwrap_or("");
    let mut t = format!("{body}\n{headline}");
    if t.len() > 127 {
        let mut cut = 124;
        while !t.is_char_boundary(cut) {
            cut -= 1;
        }
        t.truncate(cut);
        t.push_str("...");
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meters_render_every_section_from_fixture_history() {
        let mut h = History::open(None, 15, 7);
        let s = fixtures::all().remove(2);
        fixtures::populate(&mut h, &s, 1_800_000_000);
        let c = Config::default();
        for section in Section::ALL {
            let img = draw(
                section,
                &h,
                &c,
                &c.section(section).meter,
                24,
                Taskbar::Dark,
            );
            assert_eq!(img.width(), 24);
            assert!(
                img.pixels().iter().any(|p| p.alpha() == 255),
                "{section:?} drew nothing"
            );
        }
    }

    #[test]
    fn tooltip_fits_windows_limit() {
        let mut h = History::open(None, 15, 7);
        fixtures::populate(&mut h, &fixtures::all().remove(1), 1_800_000_000);
        let long = vec![verdict::Finding {
            domain: verdict::FaultDomain::Isp,
            severity: verdict::Severity::Crit,
            headline: "x".repeat(300),
        }];
        for section in Section::ALL {
            assert!(tooltip(section, &h, &Config::default(), &long).len() < 128);
        }
    }
}
