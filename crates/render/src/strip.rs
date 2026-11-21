use crate::{Image, Meter, Taskbar, meter};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use model::Section;
use palette::Palette;
use tiny_skia::{Color, FilterQuality, Pixmap, PixmapPaint, Transform};

pub struct Cell<'a> {
    pub section: Section,
    pub value: String,
    pub meter: Meter<'a>,
}

const CELL_W: f32 = 76.0;
const GAP: f32 = 4.0;

/// Logical width of a strip with `n` cells; multiply by the DPI scale for pixels.
pub fn width(n: usize) -> f32 {
    n as f32 * (CELL_W + GAP) - GAP
}

/// One rounded cell per Section: graph behind, label top-left, value bottom-right.
pub fn strip(
    cells: &[Cell],
    height: u32,
    scale: f32,
    font: Option<&FontRef>,
    pal: &Palette,
) -> Image {
    let light = pal.shade == Taskbar::Light;
    let w = (width(cells.len()) * scale).ceil().max(1.0) as u32;
    let mut img = Pixmap::new(w, height.max(1)).expect("size");
    let cell_w = (CELL_W * scale).round() as u32;
    let pad = (4.0 * scale).round() as u32;
    let graph_h = height.saturating_sub(2 * pad).max(1);
    for (i, c) in cells.iter().enumerate() {
        let x = (i as f32 * (CELL_W + GAP) * scale).round() as i32;
        let mut cell = Pixmap::new(cell_w, graph_h).unwrap();
        cell.fill(if light {
            Color::from_rgba8(0, 0, 0, 0x0c)
        } else {
            Color::from_rgba8(0xff, 0xff, 0xff, 0x0c)
        });
        let g = meter(c.section, &c.meter, graph_h, pal);
        let stretch = cell_w as f32 / graph_h as f32;
        cell.draw_pixmap(
            0,
            0,
            g.as_ref(),
            &PixmapPaint {
                quality: FilterQuality::Nearest,
                opacity: 0.55,
                ..Default::default()
            },
            Transform::from_scale(stretch, 1.0),
            None,
        );
        if let Some(f) = font {
            let small = 10.0 * scale;
            let big = 13.0 * scale;
            text(
                &mut cell,
                f,
                c.section.label(),
                3.0 * scale,
                2.0 * scale,
                small,
                if light {
                    [0x4a, 0x51, 0x60]
                } else {
                    [0xc8, 0xcd, 0xd6]
                },
                light,
            );
            let vw = measure(f, &c.value, big);
            text(
                &mut cell,
                f,
                &c.value,
                cell_w as f32 - vw - 3.0 * scale,
                graph_h as f32 - big - 1.0 * scale,
                big,
                if light {
                    [0x11, 0x14, 0x1a]
                } else {
                    [0xff, 0xff, 0xff]
                },
                light,
            );
        }
        img.draw_pixmap(
            x,
            pad as i32,
            cell.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    img
}

fn measure(font: &FontRef, s: &str, px: f32) -> f32 {
    let f = font.as_scaled(PxScale::from(px));
    s.chars().map(|c| f.h_advance(f.glyph_id(c))).sum()
}

/// Draws `s` with its top at `y`, with a 1 px dark halo so it reads over any graph color.
#[allow(clippy::too_many_arguments)]
fn text(
    img: &mut Pixmap,
    font: &FontRef,
    s: &str,
    x: f32,
    y: f32,
    px: f32,
    rgb: [u8; 3],
    light: bool,
) {
    let halo = if light { [0xff, 0xff, 0xff] } else { [0, 0, 0] };
    for (dx, dy, c, a) in [(1.0, 1.0, halo, 0.7), (0.0, 0.0, rgb, 1.0)] {
        let f = font.as_scaled(PxScale::from(px));
        let mut pen = x + dx;
        let baseline = y + dy + f.ascent();
        for ch in s.chars() {
            let id = f.glyph_id(ch);
            let glyph = id.with_scale_and_position(px, ab_glyph::point(pen, baseline));
            pen += f.h_advance(id);
            let Some(outline) = font.outline_glyph(glyph) else {
                continue;
            };
            let b = outline.px_bounds();
            let (w, h) = (img.width() as i32, img.height() as i32);
            let pixels = img.pixels_mut();
            outline.draw(|gx, gy, cov| {
                let (px_, py_) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px_ < 0 || py_ < 0 || px_ >= w || py_ >= h {
                    return;
                }
                let dst = &mut pixels[(py_ * w + px_) as usize];
                let alpha = (cov * a).clamp(0.0, 1.0);
                let d = dst.demultiply();
                let blend =
                    |s: u8, d: u8| (s as f32 * alpha + d as f32 * (1.0 - alpha)).round() as u8;
                let out_a = (alpha * 255.0 + d.alpha() as f32 * (1.0 - alpha)).round() as u8;
                *dst = tiny_skia::ColorU8::from_rgba(
                    blend(c[0], d.red()),
                    blend(c[1], d.green()),
                    blend(c[2], d.blue()),
                    out_a,
                )
                .premultiply();
            });
        }
    }
}
