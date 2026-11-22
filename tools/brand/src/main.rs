//! Usage: brand <spec brand/icon.svg> <out_dir>
//! Rasterizes the spec's icon into what Windows needs: blinkengraphs.ico for the exe, shortcut
//! and Apps & features, and the installer's banner and dialog bitmaps.

use resvg::{tiny_skia::Pixmap, usvg};
use std::{fs, path::Path};

const ICO_SIZES: [u32; 7] = [16, 20, 24, 32, 48, 64, 256];
/// WixUI's bitmap sizes: https://docs.firegiant.com/wix/tools/wixext/wixui/
const BANNER: (u32, u32) = (493, 58);
const DIALOG: (u32, u32) = (493, 312);
/// WixUI draws black text on the banner and on the dialog's right side, so those stay white.
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
/// tokens color.surface.background
const TILE: [u8; 3] = [0x12, 0x15, 0x1c];

fn main() {
    let mut args = std::env::args().skip(1);
    let (svg, out) = match (args.next(), args.next()) {
        (Some(s), Some(o)) => (s, o),
        _ => panic!("usage: brand <icon.svg> <out_dir>"),
    };
    let tree = usvg::Tree::from_data(
        &fs::read(&svg).expect("read svg"),
        &usvg::Options::default(),
    )
    .expect("parse svg");
    let out = Path::new(&out);
    fs::create_dir_all(out).unwrap();

    let pngs: Vec<Vec<u8>> = ICO_SIZES
        .iter()
        .map(|&s| render(&tree, s).encode_png().unwrap())
        .collect();
    fs::write(out.join("blinkengraphs.ico"), ico(&pngs)).unwrap();
    fs::write(out.join("icon-256.png"), &pngs[ICO_SIZES.len() - 1]).unwrap();

    let mut banner = canvas(BANNER, |_| WHITE);
    blend(&mut banner, &render(&tree, 44), BANNER.0 - 44 - 7, 7);
    fs::write(out.join("banner.bmp"), bmp(&banner)).unwrap();

    let mut dialog = canvas(DIALOG, |x| if x < 164 { TILE } else { WHITE });
    blend(&mut dialog, &render(&tree, 96), (164 - 96) / 2, 40);
    fs::write(out.join("dialog.bmp"), bmp(&dialog)).unwrap();
}

fn render(tree: &usvg::Tree, px: u32) -> Pixmap {
    let mut p = Pixmap::new(px, px).unwrap();
    let s = px as f32 / tree.size().width();
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(s, s),
        &mut p.as_mut(),
    );
    p
}

/// Opaque RGB rows, top first.
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<[u8; 3]>,
}

fn canvas((w, h): (u32, u32), color: impl Fn(u32) -> [u8; 3]) -> Canvas {
    let px = (0..h).flat_map(|_| (0..w).map(&color)).collect();
    Canvas { w, h, px }
}

fn blend(c: &mut Canvas, img: &Pixmap, x0: u32, y0: u32) {
    for y in 0..img.height() {
        for x in 0..img.width() {
            let p = img.pixel(x, y).unwrap().demultiply();
            let a = p.alpha() as u32;
            let d = &mut c.px[((y0 + y) * c.w + x0 + x) as usize];
            for (i, s) in [p.red(), p.green(), p.blue()].into_iter().enumerate() {
                d[i] = ((s as u32 * a + d[i] as u32 * (255 - a)) / 255) as u8;
            }
        }
    }
}

/// 24-bit bottom-up BMP, as Windows Installer dialogs expect.
fn bmp(c: &Canvas) -> Vec<u8> {
    let row = (c.w * 3).div_ceil(4) * 4;
    let size = 54 + row * c.h;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(c.w as i32).to_le_bytes());
    b.extend_from_slice(&(c.h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    for v in [0u32, row * c.h, 2835, 2835, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..c.h).rev() {
        let start = b.len();
        for x in 0..c.w {
            let [r, g, bl] = c.px[(y * c.w + x) as usize];
            b.extend_from_slice(&[bl, g, r]);
        }
        b.resize(start + row as usize, 0);
    }
    b
}

/// ICO with PNG-compressed entries (Windows Vista and later read these at every size).
fn ico(pngs: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0, 0, 1, 0];
    b.extend_from_slice(&(pngs.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * pngs.len() as u32;
    for (png, &s) in pngs.iter().zip(&ICO_SIZES) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        b.extend_from_slice(&[dim, dim, 0, 0]);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(&(png.len() as u32).to_le_bytes());
        b.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in pngs {
        b.extend_from_slice(png);
    }
    b
}
