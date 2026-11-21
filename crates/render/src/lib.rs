use model::Section;
use palette::{Palette, Rgb};
use tiny_skia::{Color, Pixmap};

pub use palette::Shade as Taskbar;

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

pub fn meter(section: Section, meter: &Meter, px: u32, pal: &Palette) -> Image {
    let mut img = Pixmap::new(px, px).expect("non-zero size");
    let h = px as f32;
    match meter {
        Meter::Level(values) => {
            for (x, &v) in tail(values, px) {
                bar(&mut img, x, h * (1.0 - v), h, level_color(section, v, pal));
            }
        }
        Meter::Duplex { down, up } => {
            let mid = h / 2.0;
            for (x, &v) in tail(down, px) {
                bar(&mut img, x, mid * (1.0 - v), mid, c(pal.section(section)));
            }
            for (x, &v) in tail(up, px) {
                bar(&mut img, x, mid, mid + mid * v, c(pal.upload));
            }
        }
        Meter::Latency(rtts) => {
            for (x, rtt) in tail(rtts, px) {
                match rtt {
                    Some(v) => bar(&mut img, x, h * (1.0 - v), h, level_color(section, *v, pal)),
                    None => bar(&mut img, x, 0.0, h, c(pal.loss)),
                }
            }
        }
        Meter::Loss(lost) => {
            for (x, _) in tail(lost, px).filter(|(_, l)| **l) {
                bar(&mut img, x, 0.0, h, c(pal.loss));
            }
        }
    }
    img
}

/// Three bars in the CPU, MEM and NET colors, or all in the Verdict's color.
pub fn app_icon(px: u32, pal: &Palette, status: Option<Status>) -> Image {
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
        let color = c(match status {
            None => pal.section(section),
            Some(Status::Ok) => pal.ok,
            Some(Status::Warn) => pal.warn,
            Some(Status::Crit) => pal.crit,
        });
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

fn level_color(section: Section, v: f32, pal: &Palette) -> Color {
    c(if v >= CRIT {
        pal.crit
    } else if v >= WARN {
        pal.warn
    } else {
        pal.section(section)
    })
}

pub(crate) fn c(rgb: Rgb) -> Color {
    Color::from_rgba8(rgb.0, rgb.1, rgb.2, 0xff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dark() -> Palette {
        Palette::new(Default::default(), Taskbar::Dark, &Default::default())
    }

    fn pixel(img: &Image, x: u32, y: u32) -> [u8; 4] {
        let p = img.pixel(x, y).unwrap().demultiply();
        [p.red(), p.green(), p.blue(), p.alpha()]
    }

    #[test]
    fn newest_sample_is_rightmost() {
        let img = meter(Section::Cpu, &Meter::Level(&[0.0, 0.0, 0.3]), 16, &dark());
        assert_eq!(pixel(&img, 15, 12), [0x4f, 0xc3, 0xf7, 0xff]);
        assert_ne!(pixel(&img, 14, 12), [0x4f, 0xc3, 0xf7, 0xff]);
    }

    #[test]
    fn critical_level_turns_red() {
        let img = meter(Section::Cpu, &Meter::Level(&[0.95]), 16, &dark());
        assert_eq!(pixel(&img, 15, 15), [0xff, 0x5a, 0x5a, 0xff]);
    }

    #[test]
    fn lost_probe_fills_column() {
        let img = meter(Section::Latency, &Meter::Latency(&[None]), 16, &dark());
        assert_eq!(pixel(&img, 15, 0), [0xff, 0x3b, 0x3b, 0xff]);
    }

    #[test]
    fn loss_meter_marks_only_lost_probes() {
        let img = meter(Section::Latency, &Meter::Loss(&[true, false]), 16, &dark());
        assert_eq!(pixel(&img, 14, 0), [0xff, 0x3b, 0x3b, 0xff]);
        assert_eq!(pixel(&img, 15, 15)[3], 0);
    }

    #[test]
    fn verdict_icon_takes_status_color() {
        let img = app_icon(16, &dark(), Some(Status::Crit));
        assert_eq!(pixel(&img, 7, 15), [0xff, 0x5a, 0x5a, 0xff]);
        let plain = app_icon(16, &dark(), None);
        assert_eq!(pixel(&plain, 7, 15), [0x9c, 0xcc, 0x65, 0xff]);
    }

    #[test]
    fn cube_root_saturates() {
        assert_eq!(cube_root(8.0, 1.0), 1.0);
        assert!((cube_root(0.125, 1.0) - 0.5).abs() < 1e-6);
    }
}
