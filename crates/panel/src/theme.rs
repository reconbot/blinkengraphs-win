use config::{Appearance, Mode};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Frame, Margin, Stroke, Ui,
    Visuals,
};
use model::{Probe, Section};
use palette::{Palette, Preset, Rgb, Shade};
use std::cell::RefCell;
use verdict::Severity;

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

thread_local! {
    static PAL: RefCell<Palette> = RefCell::new(Palette::new(
        Preset::Default,
        Shade::Dark,
        &Default::default(),
    ));
}

fn with<R>(f: impl FnOnce(&Palette) -> R) -> R {
    PAL.with(|p| f(&p.borrow()))
}

pub fn color(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

pub fn bg() -> Color32 {
    with(|p| color(p.bg))
}
pub fn card_fill() -> Color32 {
    with(|p| color(p.card))
}
pub fn card_edge() -> Color32 {
    with(|p| color(p.card_edge))
}
pub fn text() -> Color32 {
    with(|p| color(p.text))
}
pub fn text_weak() -> Color32 {
    with(|p| color(p.text_weak))
}
pub fn ok() -> Color32 {
    with(|p| color(p.ok))
}
pub fn warn() -> Color32 {
    with(|p| color(p.warn))
}
pub fn crit() -> Color32 {
    with(|p| color(p.crit))
}
pub fn upload() -> Color32 {
    with(|p| color(p.upload))
}
pub fn faint() -> Color32 {
    with(|p| color(p.faint))
}

/// Same colors as the Meters so a Tray Icon and its Panel card read as one thing.
pub fn accent(s: Section) -> Color32 {
    with(|p| color(p.section(s)))
}

pub fn series(i: usize) -> Color32 {
    with(|p| color(p.series(i)))
}

/// The user's color for `probe`, else the `i`th series color.
pub fn probe(probe: &Probe, i: usize) -> Color32 {
    with(|p| color(p.probe(&probe.to_string(), i)))
}

pub fn level(s: Section, v: f32) -> Color32 {
    if v >= 0.85 {
        crit()
    } else if v >= 0.6 {
        warn()
    } else {
        accent(s)
    }
}

pub fn severity_color(s: Severity) -> Color32 {
    match s {
        Severity::Ok => ok(),
        Severity::Warn => warn(),
        Severity::Crit => crit(),
    }
}

pub fn window_frame() -> Frame {
    Frame::new().fill(bg()).inner_margin(Margin::same(14))
}

pub fn card(ui: &mut Ui, s: Section, body: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(card_fill())
        .stroke(Stroke::new(1.0, card_edge()))
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

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
    });
    apply(ctx, &Appearance::default());
}

/// Rebuilds colors and visuals when the Appearance or Windows' app theme changes.
pub fn apply(ctx: &egui::Context, a: &Appearance) {
    let shade = match a.mode {
        Mode::Light => Shade::Light,
        Mode::Dark => Shade::Dark,
        Mode::System => match ctx.system_theme() {
            Some(egui::Theme::Light) => Shade::Light,
            _ => Shade::Dark,
        },
    };
    let pal = a.palette(shade);
    let id = egui::Id::unique("blinkengraphs-palette");
    PAL.with(|p| *p.borrow_mut() = pal.clone());
    if ctx.data(|d| d.get_temp::<Palette>(id)).as_ref() == Some(&pal) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, pal.clone()));
    let light = shade == Shade::Light;
    let mut v = if light {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    v.panel_fill = color(pal.bg);
    v.window_fill = color(pal.bg);
    v.extreme_bg_color = color(pal.field);
    v.faint_bg_color = color(pal.faint);
    v.override_text_color = Some(color(pal.text));
    v.selection.bg_fill = if light {
        Color32::from_rgb(0xb3, 0xd4, 0xf5)
    } else {
        Color32::from_rgb(0x2b, 0x5b, 0x8a)
    };
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.inactive.weak_bg_fill = if light {
        Color32::from_rgb(0xe4, 0xe7, 0xec)
    } else {
        Color32::from_rgb(0x22, 0x28, 0x33)
    };
    let theme = if light {
        egui::Theme::Light
    } else {
        egui::Theme::Dark
    };
    ctx.set_visuals_of(theme, v);
    ctx.set_theme(theme);
}
