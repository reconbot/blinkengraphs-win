//! Usage: screenshots <out_dir> [url_prefix]
//! Writes one PNG per fixture scenario and `index.md` linking them under `url_prefix`.

use fixtures::Scenario;
use model::Section;
use render::{Image, Meter, Status, Taskbar, app_icon, meter};
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

    let mut index = String::from("| Taskbar Strip | |\n|---|---|\n");
    for (file, caption) in strips(&out) {
        writeln!(index, "| {caption} | ![{file}]({prefix}{file}) |").unwrap();
    }
    index.push_str("\n| Panel | |\n|---|---|\n");
    for (file, caption) in panels(&out) {
        writeln!(index, "| {caption} | ![{file}]({prefix}{file}) |").unwrap();
    }
    let busy = fixtures::all().remove(1);
    sheet(&busy).save_png(out.join("tray-icons.png")).unwrap();
    writeln!(
        index,
        "\n**Tray Icons** ({}): the five Meters, then the Menu Icon as app icon and in each Verdict color; actual size on dark and light taskbars, then 4× zoom\n\n![tray-icons]({prefix}tray-icons.png)",
        busy.caption
    )
    .unwrap();
    fs::write(out.join("index.md"), index).unwrap();
}

fn meters(s: &Scenario, px: u32, taskbar: Taskbar) -> Vec<Image> {
    let pal = config::Config::default().appearance.palette(taskbar);
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
            meter(section, &m, px, &pal)
        })
        .chain(
            [
                None,
                Some(Status::Ok),
                Some(Status::Warn),
                Some(Status::Crit),
            ]
            .map(|status| app_icon(px, &pal, status)),
        )
        .collect()
}

fn sheet(s: &Scenario) -> Pixmap {
    let n = Section::ALL.len() as u32 + 4;
    let zoomed = ICON * ZOOM;
    let width = n * zoomed + (n + 1) * GAP;
    let height = 2 * TASKBAR + GAP + zoomed + GAP;
    let mut sheet = Pixmap::new(width, height).unwrap();
    sheet.fill(Color::from_rgba8(0x1b, 0x2a, 0x3a, 0xff));

    let tray_start = width - GAP - n * (TRAY + 8);
    for (row, (taskbar, bg)) in [(Taskbar::Dark, 0x20), (Taskbar::Light, 0xf3)]
        .into_iter()
        .enumerate()
    {
        let y = row as u32 * TASKBAR;
        let mut bar = Pixmap::new(width, TASKBAR).unwrap();
        bar.fill(Color::from_rgba8(bg, bg, bg, 0xff));
        blit(&mut sheet, &bar, 0, y, 1);
        for (i, img) in meters(s, TRAY, taskbar).iter().enumerate() {
            blit(
                &mut sheet,
                img,
                tray_start + i as u32 * (TRAY + 8),
                y + (TASKBAR - TRAY) / 2,
                1,
            );
        }
    }
    for (i, img) in meters(s, ICON, Taskbar::Dark).iter().enumerate() {
        blit(
            &mut sheet,
            img,
            GAP + i as u32 * (zoomed + GAP),
            2 * TASKBAR + GAP,
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
    use panel::{Panel, Range, Tab, View};
    let mut shots = Vec::new();
    let targets = [
        ("isp-loss", Tab::Overview, Range::Min1),
        ("upload-saturation", Tab::Overview, Range::Min1),
        ("isp-loss", Tab::Network, Range::Min1),
        ("isp-loss", Tab::Network, Range::Hour),
        ("idle", Tab::Settings, Range::Min1),
    ];
    for (name, tab, range) in targets {
        let s = fixtures::all()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap();
        let mut h = history::History::open(None, 15, 7);
        fixtures::populate(&mut h, &s, NOW);
        let findings = verdict::from_history(&h, true);
        let processes = fixtures::processes();
        let adapters = vec!["Wi-Fi".to_string(), "Ethernet".to_string()];
        let mut config = config::Config::default();
        let mut panel = Panel::default();
        panel.tab = tab;
        panel.range = range;
        let mut themed = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(
                460.0,
                if tab == Tab::Settings { 2400.0 } else { 1080.0 },
            ))
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
                    notice: None,
                };
                panel.show(ui, &view, &mut config);
            });
        harness.run_steps(4);
        let suffix = if range == Range::Min1 {
            String::new()
        } else {
            format!("-{range:?}")
        };
        let file = format!("panel-{name}-{tab:?}{suffix}.png").to_lowercase();
        harness
            .render()
            .expect("wgpu render")
            .save(out.join(&file))
            .unwrap();
        shots.push((file, format!("{tab:?} · {} · {range:?}", s.caption)));
    }
    shots
}

fn font_bytes() -> Option<Vec<u8>> {
    [
        r"C:\Windows\Fonts\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .find_map(|p| fs::read(p).ok())
}

fn strips(out: &std::path::Path) -> Vec<(String, String)> {
    let bytes = font_bytes();
    let font = bytes
        .as_deref()
        .and_then(|b| render::FontRef::try_from_slice(b).ok());
    let config = config::Config::default();
    let scale = 1.5;
    let height = (48.0 * scale) as u32;
    fixtures::all()
        .iter()
        .map(|s| {
            let mut h = history::History::open(None, 15, 7);
            fixtures::populate(&mut h, s, NOW);
            let dark = meters::strip(&h, &config, height, scale, font.as_ref(), Taskbar::Dark);
            let light = meters::strip(&h, &config, height, scale, font.as_ref(), Taskbar::Light);
            let w = dark.width() + (220.0 * scale) as u32;
            let mut bar = Pixmap::new(w, 2 * height).unwrap();
            bar.fill(Color::from_rgba8(0x1c, 0x1c, 0x1c, 0xff));
            let mut lightbar = Pixmap::new(w, height).unwrap();
            lightbar.fill(Color::from_rgba8(0xf3, 0xf3, 0xf3, 0xff));
            blit(&mut bar, &lightbar, 0, height, 1);
            blit(&mut bar, &dark, (40.0 * scale) as u32, 0, 1);
            blit(&mut bar, &light, (40.0 * scale) as u32, height, 1);
            let file = format!("strip-{}.png", s.name);
            bar.save_png(out.join(&file)).unwrap();
            (file, s.caption.to_string())
        })
        .collect()
}
