use crate::{
    engine::{Command, Engine},
    strip::Strip,
};
use config::Config;
use config::Surface;
use eframe::egui::{self, ViewportCommand};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use model::Section;
use panel::{Action, Panel, Tab, View};
use std::{
    path::PathBuf,
    str::FromStr,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, Rect, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
    System::{
        StationsAndDesktops::{
            CloseDesktop, DESKTOP_SWITCHDESKTOP, OpenInputDesktop, SwitchDesktop,
        },
        Threading::CreateMutexW,
    },
    UI::{
        HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi},
        WindowsAndMessaging::{SM_CXSMICON, SPI_GETSCREENSAVERRUNNING, SystemParametersInfoW},
    },
};

const PANEL: egui::Vec2 = egui::vec2(460.0, 760.0);

enum UiEvent {
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    Hotkey(GlobalHotKeyEvent),
    StripClick,
}

pub fn run() {
    let name: Vec<u16> = "Local\\blinkengraphs-single-instance\0"
        .encode_utf16()
        .collect();
    unsafe {
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
    }
    let path = config::dir().join("config.toml");
    let (config, error) = Config::load(&path);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("blinkengraphs")
            .with_inner_size(PANEL)
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_visible(false),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "blinkengraphs",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, config, error, path)))),
    );
}

struct App {
    engine: Engine,
    config: Config,
    path: PathBuf,
    error: Option<String>,
    panel: Panel,
    icons: Vec<(Section, TrayIcon)>,
    menu: Vec<(MenuId, MenuAction)>,
    events: mpsc::Receiver<UiEvent>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
    last_tick: u64,
    visible: bool,
    shown_at: Instant,
    px: u32,
    strip: Strip,
    strip_attached: bool,
    font: Option<render::FontRef<'static>>,
}

#[derive(Clone, Copy)]
enum MenuAction {
    Open,
    Settings,
    Quit,
}

impl App {
    fn new(
        cc: &eframe::CreationContext,
        config: Config,
        error: Option<String>,
        path: PathBuf,
    ) -> Self {
        let ctx = cc.egui_ctx.clone();
        panel::theme::install(&ctx);

        let (tx, events) = mpsc::channel();
        let forward = |tx: mpsc::Sender<UiEvent>, ctx: egui::Context| {
            move |e: UiEvent| {
                let _ = tx.send(e);
                ctx.request_repaint();
            }
        };
        let f = forward(tx.clone(), ctx.clone());
        TrayIconEvent::set_event_handler(Some(move |e| f(UiEvent::Tray(e))));
        let f = forward(tx.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |e| f(UiEvent::Menu(e))));
        let strip_click = forward(tx.clone(), ctx.clone());
        let strip = Strip::new(move || strip_click(UiEvent::StripClick));
        let f = forward(tx, ctx.clone());
        GlobalHotKeyEvent::set_event_handler(Some(move |e| f(UiEvent::Hotkey(e))));

        let db = config::dir().join("history.db");
        let wake_ctx = ctx.clone();
        let engine = Engine::start(
            config.clone(),
            Some(db),
            move || wake_ctx.request_repaint(),
            Box::new(idle),
        );

        let px = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.max(16) as u32;
        let mut menu = Vec::new();
        let icons = Section::ALL
            .iter()
            .map(|&section| {
                let m = Menu::new();
                for (label, action) in [
                    ("Open", MenuAction::Open),
                    ("Settings", MenuAction::Settings),
                ] {
                    let item = MenuItem::new(label, true, None);
                    menu.push((item.id().clone(), action));
                    m.append(&item).unwrap();
                }
                m.append(&PredefinedMenuItem::separator()).unwrap();
                let quit = MenuItem::new("Quit blinkengraphs", true, None);
                menu.push((quit.id().clone(), MenuAction::Quit));
                m.append(&quit).unwrap();
                let icon = TrayIconBuilder::new()
                    .with_id(format!("blinkengraphs-{section:?}"))
                    .with_tooltip(section.label())
                    .with_menu(Box::new(m))
                    .with_menu_on_left_click(false)
                    .build()
                    .expect("tray icon");
                (section, icon)
            })
            .collect();

        let mut app = Self {
            engine,
            config,
            path,
            error,
            panel: Panel::default(),
            icons,
            menu,
            events,
            hotkeys: GlobalHotKeyManager::new().ok(),
            hotkey: None,
            last_tick: u64::MAX,
            visible: false,
            shown_at: Instant::now(),
            px,
            strip,
            strip_attached: false,
            font: system_font(),
        };
        app.apply_visibility();
        app.register_hotkey();
        app
    }

    /// With the Strip attached only the Latency icon stays, as the Menu Icon.
    fn apply_visibility(&self) {
        for (s, icon) in &self.icons {
            let visible = if self.strip_attached {
                *s == Section::Latency
            } else {
                self.config.section(*s).visible
            };
            let _ = icon.set_visible(visible);
        }
    }

    fn register_hotkey(&mut self) {
        let Some(m) = &self.hotkeys else { return };
        if let Some(old) = self.hotkey.take() {
            let _ = m.unregister(old);
        }
        match HotKey::from_str(&self.config.hotkey) {
            Ok(k) => match m.register(k) {
                Ok(()) => self.hotkey = Some(k),
                Err(e) => self.error = Some(format!("hotkey {}: {e}", self.config.hotkey)),
            },
            Err(e) => self.error = Some(format!("hotkey {}: {e}", self.config.hotkey)),
        }
    }

    fn redraw_icons(&mut self) {
        let s = self.engine.shared.lock().unwrap();
        if s.tick == self.last_tick {
            return;
        }
        self.last_tick = s.tick;
        let attached = self.config.surface == Surface::TaskbarStrip
            && self.strip.geometry().is_some_and(|(scale, h)| {
                let img = meters::strip(&s.history, &self.config, h, scale, self.font.as_ref());
                self.strip.show(&img)
            });
        if !attached {
            self.strip.destroy();
        }
        if attached != self.strip_attached {
            self.strip_attached = attached;
            self.apply_visibility();
        }
        for (section, icon) in &self.icons {
            let c = self.config.section(*section);
            if !c.visible || (attached && *section != Section::Latency) {
                continue;
            }
            let img = meters::draw(*section, &s.history, &self.config, &c.meter, self.px);
            if let Ok(i) = Icon::from_rgba(render::to_rgba(&img), self.px, self.px) {
                let _ = icon.set_icon(Some(i));
            }
            let _ = icon.set_tooltip(Some(meters::tooltip(
                *section,
                &s.history,
                &self.config,
                &s.findings,
            )));
        }
    }

    fn show(&mut self, ctx: &egui::Context, anchor: Option<Rect>, tab: Option<Tab>) {
        if let Some(t) = tab {
            self.panel.tab = t;
        }
        let scale = unsafe { GetDpiForSystem() } as f32 / 96.0;
        let monitor = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(egui::vec2(1920.0, 1080.0));
        let (x, y) = match anchor {
            Some(r) => {
                let cx = (r.position.x as f32 + r.size.width as f32 / 2.0) / scale;
                let top = r.position.y as f32 / scale;
                (
                    cx - PANEL.x / 2.0,
                    if top > monitor.y / 2.0 {
                        top - PANEL.y - 12.0
                    } else {
                        top + r.size.height as f32 / scale + 12.0
                    },
                )
            }
            None => (monitor.x - PANEL.x - 16.0, monitor.y - PANEL.y - 64.0),
        };
        let pos = egui::pos2(
            x.clamp(8.0, (monitor.x - PANEL.x - 8.0).max(8.0)),
            y.max(8.0),
        );
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos));
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.visible = true;
        self.shown_at = Instant::now();
        self.engine.panel_open.store(true, Ordering::Relaxed);
    }

    fn hide(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.visible = false;
        self.engine.panel_open.store(false, Ordering::Relaxed);
    }

    fn toggle(&mut self, ctx: &egui::Context, anchor: Option<Rect>) {
        if self.visible {
            self.hide(ctx)
        } else {
            self.show(ctx, anchor, None)
        }
    }

    fn quit(&self) -> ! {
        let _ = self.config.save(&self.path);
        std::process::exit(0)
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(e) = self.events.try_recv() {
            match e {
                UiEvent::Tray(TrayIconEvent::Click {
                    rect,
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }) => self.toggle(ctx, Some(rect)),
                UiEvent::Tray(_) => {}
                UiEvent::Menu(m) => match self
                    .menu
                    .iter()
                    .find(|(id, _)| *id == m.id)
                    .map(|(_, a)| *a)
                {
                    Some(MenuAction::Open) => self.show(ctx, None, Some(Tab::Overview)),
                    Some(MenuAction::Settings) => self.show(ctx, None, Some(Tab::Settings)),
                    Some(MenuAction::Quit) => self.quit(),
                    None => {}
                },
                UiEvent::Hotkey(h)
                    if h.state == HotKeyState::Pressed
                        && Some(h.id) == self.hotkey.map(|k| k.id()) =>
                {
                    self.toggle(ctx, None)
                }
                UiEvent::Hotkey(_) => {}
                UiEvent::StripClick => self.toggle(ctx, None),
            }
        }
        self.redraw_icons();
        if self.visible {
            let focused = ctx.input(|i| i.viewport().focused);
            if focused == Some(false) && self.shown_at.elapsed() > Duration::from_millis(400) {
                self.hide(ctx);
            } else {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.visible {
            return;
        }
        let shared = self.engine.shared.clone();
        let s = shared.lock().unwrap();
        let view = View {
            history: &s.history,
            findings: &s.findings,
            processes: &s.processes,
            adapters: &s.adapters,
            now: model::now(),
            elevated: false,
            config_error: self.error.as_deref(),
        };
        let actions = self.panel.show(ui, &view, &mut self.config);
        drop(s);
        for a in actions {
            match a {
                Action::ConfigChanged => {
                    if let Err(e) = self.config.save(&self.path) {
                        self.error = Some(e.to_string());
                    }
                    self.engine
                        .send(Command::Reconfigure(Box::new(self.config.clone())));
                    self.apply_visibility();
                    self.register_hotkey();
                    self.last_tick = u64::MAX;
                }
                Action::Quit => self.quit(),
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.hide(ui.ctx());
        }
    }
}

fn system_font() -> Option<render::FontRef<'static>> {
    let bytes = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf").ok()?;
    render::FontRef::try_from_slice(Box::leak(bytes.into_boxed_slice())).ok()
}

/// Locked sessions can't switch to the input desktop; that is the documented-by-folklore lock test.
fn idle() -> bool {
    unsafe {
        let mut saver = 0i32;
        SystemParametersInfoW(
            SPI_GETSCREENSAVERRUNNING,
            0,
            (&mut saver as *mut i32).cast(),
            0,
        );
        if saver != 0 {
            return true;
        }
        let desk = OpenInputDesktop(0, 0, DESKTOP_SWITCHDESKTOP);
        if desk.is_null() {
            return true;
        }
        let ok = SwitchDesktop(desk);
        CloseDesktop(desk);
        ok == 0
    }
}
