use config::{Config, MenuIcon, SectionConfig};
use history::{History, Key};
use model::{Metric, Ping, Readout, Section, Unit};
use render::{Cell, FontRef, Image, Meter, Status, Taskbar, cube_root, meter};

/// Meter data a Section draws from, owned so a Strip can borrow many at once.
enum Series {
    Level(Vec<f32>),
    Duplex(Vec<f32>, Vec<f32>),
    Latency(Vec<Option<f32>>),
    Loss(Vec<bool>),
}

impl Series {
    fn meter(&self) -> Meter<'_> {
        match self {
            Series::Level(v) => Meter::Level(v),
            Series::Duplex(down, up) => Meter::Duplex { down, up },
            Series::Latency(v) => Meter::Latency(v),
            Series::Loss(v) => Meter::Loss(v),
        }
    }
}

fn series(
    section: Section,
    h: &History,
    config: &Config,
    shown: &[Readout],
    n: usize,
    now: u64,
) -> Series {
    if section == Section::Latency {
        // One column a second, so a Pause shows as empty columns rather than old and new results
        // side by side. The newest seconds' Rounds may still be waiting for replies, so the
        // Meter ends where every Round has reported: a timeout plus a second.
        let end = now.saturating_sub(h.grace_s());
        let raw = h.by_second(&Key::Probe(config.primary.clone()), end, n);
        let latency = shown.contains(&Readout::Ping(Ping::Latency));
        let loss = shown.contains(&Readout::Ping(Ping::Loss));
        if !latency {
            return Series::Loss(raw.iter().map(|r| loss && *r == Some(None)).collect());
        }
        let scale = latency_scale(h.now(&config.primary, now).and_then(|s| s.baseline_ms));
        return Series::Latency(
            raw.into_iter()
                .map(|r| match r {
                    Some(Some(ms)) => Some((ms / scale).min(1.0)),
                    Some(None) if loss => None,
                    _ => Some(0.0),
                })
                .collect(),
        );
    }
    let seconds = |m: Metric| -> Vec<Option<f32>> {
        h.by_second(&Key::Metric(m), now, n)
            .into_iter()
            .map(Option::flatten)
            .collect()
    };
    let metrics = SectionConfig::metrics(shown);
    if metrics.contains(&Metric::NetDown) && metrics.contains(&Metric::NetUp) {
        let down = seconds(Metric::NetDown);
        let up = seconds(Metric::NetUp);
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
        return Series::Duplex(norm(down), norm(up));
    }
    match metrics.first() {
        Some(&m) => Series::Level(normalize(m, &seconds(m))),
        None => Series::Level(vec![]),
    }
}

pub fn latency_scale(baseline_ms: Option<f32>) -> f32 {
    (baseline_ms.unwrap_or(20.0) * 4.0).max(100.0)
}

/// Draws one Section's Meter at `px` from the last `px` samples, showing `shown`.
pub fn draw(
    section: Section,
    h: &History,
    config: &Config,
    shown: &[Readout],
    px: u32,
    taskbar: Taskbar,
    now: u64,
) -> Image {
    meter(
        section,
        &series(section, h, config, shown, px as usize, now).meter(),
        px,
        &config.appearance.palette(taskbar),
    )
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

/// The Latency Meter's Probe summarized as the user chose (default: median over 30 s).
fn primary_summary(h: &History, config: &Config, now: u64) -> history::Summary {
    let v = config.probe_view(&config.primary);
    h.aggregate(&config.primary, now, v.window_secs, v.aggregate)
}

fn value_text(
    section: Section,
    h: &History,
    config: &Config,
    shown: &[Readout],
    now: u64,
) -> String {
    let s = h.last_sample();
    let get = |m| s.map(|s| s.get(m)).unwrap_or(0.0);
    if section == Section::Latency {
        let st = primary_summary(h, config, now);
        let loss = format!("{:.0}%", st.loss * 100.0);
        return match (shown.contains(&Readout::Ping(Ping::Latency)), st.value) {
            (false, _) => format!("{loss} loss"),
            (true, None) => "no reply".into(),
            (true, Some(r)) if st.loss > 0.0 && shown.contains(&Readout::Ping(Ping::Loss)) => {
                format!("{r:.0}ms {loss}")
            }
            (true, Some(r)) => model::rtt_text(r as f64),
        };
    }
    let metrics = SectionConfig::metrics(shown);
    if metrics.contains(&Metric::NetDown) && metrics.contains(&Metric::NetUp) {
        return verdict::bytes(get(Metric::NetDown) + get(Metric::NetUp));
    }
    let Some(&m) = metrics.first() else {
        return String::new();
    };
    match m.unit() {
        Unit::Percent => format!("{:.0}%", get(m)),
        Unit::BytesPerSec => verdict::bytes(get(m)),
        Unit::Millis => format!("{:.0} ms", get(m)),
        _ => format!("{:.0}", get(m)),
    }
}

/// One Strip cell per Section with Strip Readouts.
pub fn strip(
    h: &History,
    config: &Config,
    height: u32,
    scale: f32,
    font: Option<&FontRef>,
    taskbar: Taskbar,
    now: u64,
) -> Image {
    let owned: Vec<(Section, String, Series)> = Section::ALL
        .into_iter()
        .filter_map(|section| {
            let shown = &config.section(section).strip;
            (!shown.is_empty()).then(|| {
                (
                    section,
                    value_text(section, h, config, shown, now),
                    series(section, h, config, shown, 60, now),
                )
            })
        })
        .collect();
    let cells: Vec<Cell> = owned
        .iter()
        .map(|(section, value, series)| Cell {
            section: *section,
            value: value.clone(),
            meter: series.meter(),
        })
        .collect();
    render::strip(
        &cells,
        height,
        scale,
        font,
        &config.appearance.palette(taskbar),
    )
}

/// The Menu Icon drawn while the Strip is attached.
pub fn menu_icon(
    h: &History,
    config: &Config,
    findings: &[verdict::Finding],
    px: u32,
    taskbar: Taskbar,
    now: u64,
) -> Image {
    match config.menu_icon {
        MenuIcon::App => render::app_icon(px, &config.appearance.palette(taskbar), None),
        MenuIcon::Verdict => {
            let status = match findings.first().map(|f| f.severity) {
                Some(verdict::Severity::Crit) => Status::Crit,
                Some(verdict::Severity::Warn) => Status::Warn,
                _ => Status::Ok,
            };
            render::app_icon(px, &config.appearance.palette(taskbar), Some(status))
        }
        MenuIcon::Meter(section) => {
            let c = config.section(section);
            let shown = [&c.tray, &c.strip, &c.panel]
                .into_iter()
                .find(|l| !l.is_empty())
                .map_or(&[][..], |l| l);
            draw(section, h, config, shown, px, taskbar, now)
        }
    }
}

/// Windows truncates notification-area tooltips to 128 characters.
pub fn tooltip(
    section: Section,
    h: &History,
    config: &Config,
    findings: &[verdict::Finding],
    now: u64,
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
            let st = primary_summary(h, config, now);
            let v = config.probe_view(&config.primary);
            let rtt = st
                .value
                .map(|r| format!("{} {}", v.aggregate.label(), model::rtt_text(r as f64)))
                .unwrap_or("no reply".into());
            format!(
                "{} {rtt}  {:.0}% loss over {}",
                config.primary,
                st.loss * 100.0,
                config::ProbeView::window_label(v.window_secs)
            )
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
        let mut h = History::open(None, Config::default().history_limits());
        let s = fixtures::all().remove(2);
        fixtures::populate(&mut h, &s, 1_800_000_000);
        let c = Config::default();
        for section in Section::ALL {
            let img = draw(
                section,
                &h,
                &c,
                &c.section(section).tray,
                24,
                Taskbar::Dark,
                1_800_000_000,
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
        let mut h = History::open(None, Config::default().history_limits());
        fixtures::populate(&mut h, &fixtures::all().remove(1), 1_800_000_000);
        let long = vec![verdict::Finding {
            domain: verdict::FaultDomain::Isp,
            severity: verdict::Severity::Crit,
            headline: "x".repeat(300),
        }];
        for section in Section::ALL {
            assert!(tooltip(section, &h, &Config::default(), &long, 1_800_000_000).len() < 128);
        }
    }
}
