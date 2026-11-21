//! Usage: screenshots <out_dir> [url_prefix]
//! Writes one PNG per fixture scenario and `index.md` linking them under `url_prefix`.

use fixtures::Scenario;
use model::Section;
use render::{Image, Meter, meter};
use std::{fmt::Write, fs, path::PathBuf};
use tiny_skia::{Color, FilterQuality, Pixmap, PixmapPaint, Transform};

const ZOOM: u32 = 4;
const ICON: u32 = 32;
const TRAY: u32 = 24;
const GAP: u32 = 16;
const TASKBAR: u32 = 48;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(
        args.next()
            .expect("usage: screenshots <out_dir> [url_prefix]"),
    );
    let prefix = args.next().unwrap_or_default();
    fs::create_dir_all(&out).unwrap();

    let mut index =
        String::from("| Scenario | Tray Icons (actual size above, 4× zoom below) |\n|---|---|\n");
    for s in fixtures::all() {
        let file = format!("{}.png", s.name);
        sheet(&s).save_png(out.join(&file)).unwrap();
        writeln!(index, "| {} | ![{}]({prefix}{file}) |", s.caption, s.name).unwrap();
    }
    index.push_str("\n| Panel | |\n|---|---|\n");
    for (file, caption) in panels(&out) {
        writeln!(index, "| {caption} | ![{file}]({prefix}{file}) |").unwrap();
    }
    fs::write(out.join("index.md"), index).unwrap();
}

fn meters(s: &Scenario, px: u32) -> Vec<Image> {
    Section::ALL
        .iter()
        .map(|&section| {
            let m = match section {
                Section::Cpu => Meter::Level(&s.cpu),
                Section::Memory => Meter::Level(&s.memory),
                Section::Disk => Meter::Level(&s.disk),
                Section::Network => Meter::Duplex {
                    down: &s.down,
                    up: &s.up,
                },
                Section::Latency => Meter::Latency(&s.rtt),
            };
            meter(section, &m, px)
        })
        .collect()
}

fn sheet(s: &Scenario) -> Pixmap {
    let n = Section::ALL.len() as u32;
    let zoomed = ICON * ZOOM;
    let width = n * zoomed + (n + 1) * GAP;
    let height = TASKBAR + GAP + zoomed + GAP;
    let mut sheet = Pixmap::new(width, height).unwrap();
    sheet.fill(Color::from_rgba8(0x1b, 0x2a, 0x3a, 0xff));

    let mut taskbar = Pixmap::new(width, TASKBAR).unwrap();
    taskbar.fill(Color::from_rgba8(0x20, 0x20, 0x20, 0xff));
    blit(&mut sheet, &taskbar, 0, 0, 1);
    let tray_start = width - GAP - n * (TRAY + 8);
    for (i, img) in meters(s, TRAY).iter().enumerate() {
        blit(
            &mut sheet,
            img,
            tray_start + i as u32 * (TRAY + 8),
            (TASKBAR - TRAY) / 2,
            1,
        );
    }
    for (i, img) in meters(s, ICON).iter().enumerate() {
        blit(
            &mut sheet,
            img,
            GAP + i as u32 * (zoomed + GAP),
            TASKBAR + GAP,
            ZOOM,
        );
    }
    sheet
}

fn blit(dst: &mut Pixmap, src: &Pixmap, x: u32, y: u32, scale: u32) {
    let paint = PixmapPaint {
        quality: FilterQuality::Nearest,
        ..Default::default()
    };
    let t = Transform::from_row(scale as f32, 0.0, 0.0, scale as f32, x as f32, y as f32);
    dst.draw_pixmap(0, 0, src.as_ref(), &paint, t, None);
}

const NOW: u64 = 1_800_000_000;

fn panels(out: &std::path::Path) -> Vec<(String, String)> {
    use panel::{Panel, Tab, View};
    let mut shots = Vec::new();
    let mut targets = vec![
        ("isp-loss", Tab::Overview),
        ("upload-saturation", Tab::Overview),
        ("isp-loss", Tab::Network),
    ];
    targets.push(("idle", Tab::Settings));
    for (name, tab) in targets {
        let s = fixtures::all()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap();
        let mut h = history::History::open(None, 15, 7);
        fixtures::populate(&mut h, &s, NOW);
        let probes: Vec<_> = h
            .probes()
            .into_iter()
            .map(|p| {
                let st = h.stat(&p, 30);
                (p, st)
            })
            .collect();
        let peak = h
            .recent(&history::Key::Metric(model::Metric::NetUp), 900)
            .into_iter()
            .flatten()
            .fold(0.0f32, f32::max) as f64;
        let findings = verdict::diagnose(&verdict::Input {
            sample: h.last_sample(),
            probes: &probes,
            upload_peak_bps: peak,
        });
        let processes = fixtures::processes();
        let adapters = vec!["Wi-Fi".to_string(), "Ethernet".to_string()];
        let mut config = config::Config::default();
        let mut panel = Panel {
            tab,
            ..Panel::default()
        };
        let mut themed = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(460.0, 760.0))
            .with_pixels_per_point(2.0)
            .allow_missing_glyphs()
            .with_accessibility_check(false)
            .wgpu()
            .build_ui(|ui| {
                if !themed {
                    panel::theme::install(ui.ctx());
                    themed = true;
                }
                let view = View {
                    history: &h,
                    findings: &findings,
                    processes: &processes,
                    adapters: &adapters,
                    now: NOW,
                    elevated: false,
                    config_error: None,
                };
                panel.show(ui, &view, &mut config);
            });
        harness.run_steps(4);
        let file = format!("panel-{name}-{tab:?}.png").to_lowercase();
        harness
            .render()
            .expect("wgpu render")
            .save(out.join(&file))
            .unwrap();
        shots.push((file, format!("{tab:?} · {}", s.caption)));
    }
    shots
}
