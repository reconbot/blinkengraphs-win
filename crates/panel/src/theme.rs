use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Frame, Margin, Stroke, Ui,
    Visuals,
};
use model::Section;
use verdict::Severity;

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

pub const BG: Color32 = Color32::from_rgb(0x12, 0x15, 0x1c);
pub const CARD: Color32 = Color32::from_rgb(0x1b, 0x20, 0x2a);
pub const CARD_EDGE: Color32 = Color32::from_rgb(0x2a, 0x31, 0x3d);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xea, 0xf0);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x8b, 0x95, 0xa5);
pub const OK: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80);
pub const WARN: Color32 = Color32::from_rgb(0xff, 0xb3, 0x40);
pub const CRIT: Color32 = Color32::from_rgb(0xff, 0x5a, 0x5a);
pub const UPLOAD: Color32 = Color32::from_rgb(0xff, 0x9b, 0xe1);

/// Same hues as the Meters (crates/render) so a Tray Icon and its Panel card read as one thing.
pub fn accent(s: Section) -> Color32 {
    match s {
        Section::Cpu => Color32::from_rgb(0x4f, 0xc3, 0xf7),
        Section::Memory => Color32::from_rgb(0x9c, 0xcc, 0x65),
        Section::Network => Color32::from_rgb(0x7a, 0xb8, 0xff),
        Section::Disk => Color32::from_rgb(0xb3, 0x9d, 0xdb),
        Section::Latency => Color32::from_rgb(0xff, 0xd5, 0x4f),
    }
}

pub fn faint() -> Color32 {
    Color32::from_rgb(0x20, 0x26, 0x31)
}

pub fn series(i: usize) -> Color32 {
    const P: [Color32; 8] = [
        Color32::from_rgb(0xff, 0xd5, 0x4f),
        Color32::from_rgb(0x4f, 0xc3, 0xf7),
        Color32::from_rgb(0xf4, 0x8f, 0xb1),
        Color32::from_rgb(0x9c, 0xcc, 0x65),
        Color32::from_rgb(0xb3, 0x9d, 0xdb),
        Color32::from_rgb(0xff, 0x8a, 0x65),
        Color32::from_rgb(0x4d, 0xd0, 0xe1),
        Color32::from_rgb(0xe0, 0xe0, 0xe0),
    ];
    P[i % P.len()]
}

pub fn level(s: Section, v: f32) -> Color32 {
    if v >= 0.85 {
        CRIT
    } else if v >= 0.6 {
        WARN
    } else {
        accent(s)
    }
}

pub fn severity_color(s: Severity) -> Color32 {
    match s {
        Severity::Ok => OK,
        Severity::Warn => WARN,
        Severity::Crit => CRIT,
    }
}

pub fn window_frame() -> Frame {
    Frame::new().fill(BG).inner_margin(Margin::same(14))
}

pub fn card(ui: &mut Ui, s: Section, body: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_EDGE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        })
        .response
        .rect
        .pipe(|r| {
            let y = r.top() + 0.5;
            ui.painter().line_segment(
                [
                    egui::pos2(r.left() + 12.0, y),
                    egui::pos2(r.left() + 40.0, y),
                ],
                Stroke::new(2.0, accent(s)),
            );
        });
}

/// Segoe UI Variable when present (Windows 11), egui's bundled font otherwise.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for path in [
        r"C:\Windows\Fonts\SegUIVar.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("system".into(), FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "system".into());
            break;
        }
    }
    ctx.set_fonts(fonts);

    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = Color32::from_rgb(0x0e, 0x11, 0x16);
    v.faint_bg_color = Color32::from_rgb(0x20, 0x26, 0x31);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = Color32::from_rgb(0x2b, 0x5b, 0x8a);
    v.widgets.inactive.corner_radius = CornerRadius::same(6);
    v.widgets.hovered.corner_radius = CornerRadius::same(6);
    v.widgets.active.corner_radius = CornerRadius::same(6);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x22, 0x28, 0x33);
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
    });
}
