use model::Section;
use tiny_skia::{Color, Pixmap};

pub use tiny_skia::Pixmap as Image;

mod strip;
pub use ab_glyph::FontRef;
pub use strip::{Cell, strip, width as strip_width};

/// Values are normalized to 0..=1; the newest sample is last.
pub enum Meter<'a> {
    Level(&'a [f32]),
    Duplex {
        down: &'a [f32],
        up: &'a [f32],
    },
    /// `None` is a lost probe.
    Latency(&'a [Option<f32>]),
    /// `true` is a lost probe; replies draw nothing.
    Loss(&'a [bool]),
}

/// Colors the app icon by the Verdict's worst Finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Crit,
}

/// Normalized Meter value at which bars turn amber.
pub const WARN: f32 = 0.6;
/// Normalized Meter value at which bars turn red.
pub const CRIT: f32 = 0.85;

/// Small and large rates both stay visible at 16 px (MenuMeters' default network scale).
pub fn cube_root(value: f64, max: f64) -> f32 {
    (value.max(0.0) / max).cbrt().min(1.0) as f32
}

/// The taskbar the image sits on; Windows lets users pick light or dark independently of apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Taskbar {
    #[default]
    Dark,
    Light,
}

pub fn meter(section: Section, meter: &Meter, px: u32, taskbar: Taskbar) -> Image {
    let mut img = Pixmap::new(px, px).expect("non-zero size");
    let h = px as f32;
    match meter {
        Meter::Level(values) => {
            for (x, &v) in tail(values, px) {
                bar(
                    &mut img,
                    x,
                    h * (1.0 - v),
                    h,
                    level_color(section, v, taskbar),
                );
            }
        }
        Meter::Duplex { down, up } => {
            let mid = h / 2.0;
            for (x, &v) in tail(down, px) {
                bar(&mut img, x, mid * (1.0 - v), mid, base(section, taskbar));
            }
            for (x, &v) in tail(up, px) {
                bar(&mut img, x, mid, mid + mid * v, upload(taskbar));
            }
        }
        Meter::Latency(rtts) => {
            for (x, rtt) in tail(rtts, px) {
                match rtt {
                    Some(v) => bar(
                        &mut img,
                        x,
                        h * (1.0 - v),
                        h,
                        level_color(section, *v, taskbar),
                    ),
                    None => bar(&mut img, x, 0.0, h, loss(taskbar)),
                }
            }
        }
        Meter::Loss(lost) => {
            for (x, _) in tail(lost, px).filter(|(_, l)| **l) {
                bar(&mut img, x, 0.0, h, loss(taskbar));
            }
        }
    }
    img
}

/// Three bars in the CPU, MEM and NET colors, or all in the Verdict's color.
pub fn app_icon(px: u32, taskbar: Taskbar, status: Option<Status>) -> Image {
    let mut img = Pixmap::new(px, px).expect("non-zero size");
    let h = px as f32;
    let w = (px / 4).max(1);
    let gap = (px - 3 * w) / 4;
    let bars = [
        (Section::Cpu, 0.55),
        (Section::Memory, 1.0),
        (Section::Network, 0.75),
    ];
    for (i, (section, height)) in bars.into_iter().enumerate() {
        let color = match status {
            None => base(section, taskbar),
            Some(Status::Ok) => pick(taskbar, (0x66, 0xbb, 0x6a), (0x2e, 0x7d, 0x32)),
            Some(Status::Warn) => rgba(0xff, 0xb3, 0x40, 0xff),
            Some(Status::Crit) => rgba(0xff, 0x5a, 0x5a, 0xff),
        };
        let left = gap + i as u32 * (w + gap);
        for x in left..left + w {
            bar(&mut img, x as f32, h * (1.0 - height), h, color);
        }
    }
    img
}

/// Straight (non-premultiplied) RGBA8, as icon APIs expect.
pub fn to_rgba(img: &Image) -> Vec<u8> {
    img.pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

fn tail<T>(values: &[T], px: u32) -> impl Iterator<Item = (f32, &T)> {
    let n = values.len().min(px as usize);
    let offset = px as usize - n;
    values[values.len() - n..]
        .iter()
        .enumerate()
        .map(move |(i, v)| ((offset + i) as f32, v))
}

fn bar(img: &mut Pixmap, x: f32, top: f32, bottom: f32, color: Color) {
    let w = img.width() as usize;
    let (x, top, bottom) = (x as usize, top.round() as usize, bottom.round() as usize);
    let top = top.min(bottom.saturating_sub(1));
    let px = color.premultiply().to_color_u8();
    for y in top..bottom.min(img.height() as usize) {
        img.pixels_mut()[y * w + x] = px;
    }
}

fn level_color(section: Section, v: f32, t: Taskbar) -> Color {
    if v >= CRIT {
        rgba(0xff, 0x5a, 0x5a, 0xff)
    } else if v >= WARN {
        rgba(0xff, 0xb3, 0x40, 0xff)
    } else {
        base(section, t)
    }
}

fn base(section: Section, t: Taskbar) -> Color {
    match section {
        Section::Cpu => pick(t, (0x4f, 0xc3, 0xf7), (0x02, 0x77, 0xbd)),
        Section::Memory => pick(t, (0x9c, 0xcc, 0x65), (0x38, 0x8e, 0x3c)),
        Section::Network => pick(t, (0x7a, 0xb8, 0xff), (0x15, 0x65, 0xc0)),
        Section::Disk => pick(t, (0xb3, 0x9d, 0xdb), (0x5e, 0x35, 0xb1)),
        Section::Latency => pick(t, (0xff, 0xd5, 0x4f), (0xb2, 0x6a, 0x00)),
    }
}

fn upload(t: Taskbar) -> Color {
    pick(t, (0xff, 0x9b, 0xe1), (0xc2, 0x18, 0x5b))
}

fn loss(t: Taskbar) -> Color {
    pick(t, (0xff, 0x3b, 0x3b), (0xc6, 0x28, 0x28))
}

/// Dark-taskbar colors are the spec tokens; light-taskbar ones are darker shades of the same hues for contrast.
fn pick(t: Taskbar, dark: (u8, u8, u8), light: (u8, u8, u8)) -> Color {
    let (r, g, b) = if t == Taskbar::Dark { dark } else { light };
    rgba(r, g, b, 0xff)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(img: &Image, x: u32, y: u32) -> [u8; 4] {
        let p = img.pixel(x, y).unwrap().demultiply();
        [p.red(), p.green(), p.blue(), p.alpha()]
    }

    #[test]
    fn newest_sample_is_rightmost() {
        let img = meter(
            Section::Cpu,
            &Meter::Level(&[0.0, 0.0, 0.3]),
            16,
            Taskbar::Dark,
        );
        assert_eq!(pixel(&img, 15, 12), [0x4f, 0xc3, 0xf7, 0xff]);
        assert_ne!(pixel(&img, 14, 12), [0x4f, 0xc3, 0xf7, 0xff]);
    }

    #[test]
    fn critical_level_turns_red() {
        let img = meter(Section::Cpu, &Meter::Level(&[0.95]), 16, Taskbar::Dark);
        assert_eq!(pixel(&img, 15, 15), [0xff, 0x5a, 0x5a, 0xff]);
    }

    #[test]
    fn lost_probe_fills_column() {
        let img = meter(
            Section::Latency,
            &Meter::Latency(&[None]),
            16,
            Taskbar::Dark,
        );
        assert_eq!(pixel(&img, 15, 0), [0xff, 0x3b, 0x3b, 0xff]);
    }

    #[test]
    fn loss_meter_marks_only_lost_probes() {
        let img = meter(
            Section::Latency,
            &Meter::Loss(&[true, false]),
            16,
            Taskbar::Dark,
        );
        assert_eq!(pixel(&img, 14, 0), [0xff, 0x3b, 0x3b, 0xff]);
        assert_eq!(pixel(&img, 15, 15)[3], 0);
    }

    #[test]
    fn verdict_icon_takes_status_color() {
        let img = app_icon(16, Taskbar::Dark, Some(Status::Crit));
        assert_eq!(pixel(&img, 7, 15), [0xff, 0x5a, 0x5a, 0xff]);
        let plain = app_icon(16, Taskbar::Dark, None);
        assert_eq!(pixel(&plain, 7, 15), [0x9c, 0xcc, 0x65, 0xff]);
    }

    #[test]
    fn cube_root_saturates() {
        assert_eq!(cube_root(8.0, 1.0), 1.0);
        assert!((cube_root(0.125, 1.0) - 0.5).abs() < 1e-6);
    }
}
