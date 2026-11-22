//! Rasterizes the spec's icon into what Windows needs: an `.ico` for the exe, shortcut and
//! Apps & features, and the installer's banner and dialog bitmaps.

use resvg::{tiny_skia::Pixmap, usvg};

const ICO_SIZES: [u32; 7] = [16, 20, 24, 32, 48, 64, 256];
/// WixUI's bitmap sizes: https://docs.firegiant.com/wix/tools/wixext/wixui/
const BANNER: (u32, u32) = (493, 58);
const DIALOG: (u32, u32) = (493, 312);
/// WixUI draws black text on the banner and on the dialog's right side, so those stay white.
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
/// tokens color.surface.background
const TILE: [u8; 3] = [0x12, 0x15, 0x1c];

pub struct Icon(usvg::Tree);

impl Icon {
    pub fn parse(svg: &[u8]) -> Self {
        Self(usvg::Tree::from_data(svg, &usvg::Options::default()).expect("parse svg"))
    }

    pub fn png(&self, px: u32) -> Vec<u8> {
        render(&self.0, px).encode_png().unwrap()
    }

    pub fn ico(&self) -> Vec<u8> {
        let pngs: Vec<Vec<u8>> = ICO_SIZES.iter().map(|&s| self.png(s)).collect();
        ico(&pngs)
    }

    pub fn banner_bmp(&self) -> Vec<u8> {
        let mut banner = canvas(BANNER, |_| WHITE);
        blend(&mut banner, &render(&self.0, 44), BANNER.0 - 44 - 7, 7);
        bmp(&banner)
    }

    pub fn dialog_bmp(&self) -> Vec<u8> {
        let mut dialog = canvas(DIALOG, |x| if x < 164 { TILE } else { WHITE });
        blend(&mut dialog, &render(&self.0, 96), (164 - 96) / 2, 40);
        bmp(&dialog)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_directory_points_at_one_png_per_size() {
        let svg = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spec/brand/icon.svg"
        ));
        let ico = Icon::parse(&svg.unwrap()).ico();
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
        assert_eq!(
            u16::from_le_bytes([ico[4], ico[5]]) as usize,
            ICO_SIZES.len()
        );
        for (i, &s) in ICO_SIZES.iter().enumerate() {
            let e = &ico[6 + 16 * i..][..16];
            assert_eq!(e[0], if s >= 256 { 0 } else { s as u8 });
            let off = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
            assert_eq!(&ico[off + 1..off + 4], b"PNG");
        }
    }
}
