//! Usage: brand <spec brand/icon.svg> <out_dir>

use brand::Icon;
use std::{fs, path::Path};

fn main() {
    let mut args = std::env::args().skip(1);
    let (svg, out) = match (args.next(), args.next()) {
        (Some(s), Some(o)) => (s, o),
        _ => panic!("usage: brand <icon.svg> <out_dir>"),
    };
    let icon = Icon::parse(&fs::read(&svg).expect("read svg"));
    let out = Path::new(&out);
    fs::create_dir_all(out).unwrap();
    fs::write(out.join("blinkengraphs.ico"), icon.ico()).unwrap();
    fs::write(out.join("icon-256.png"), icon.png(256)).unwrap();
    fs::write(out.join("banner.bmp"), icon.banner_bmp()).unwrap();
    fs::write(out.join("dialog.bmp"), icon.dialog_bmp()).unwrap();
}
