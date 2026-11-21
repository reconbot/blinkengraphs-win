use crate::strip::Strip;
use config::Config;
use config::{MenuIcon, Surface};
use eframe::egui::{self, ViewportCommand};
use engine::{Command, Engine};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use model::Section;
use panel::{Action, Panel, Tab, View};
use std::{
    path::PathBuf,
    str::FromStr,
    sync::{Arc, Mutex, atomic::Ordering, mpsc},
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
            .with_visible(false)
            .with_position([-32000.0, -32000.0]),
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
    drawn_samples: u64,
    visible: bool,
    shown_at: Instant,
    px: u32,
    strip: Strip,
    strip_attached: bool,
    font: Option<render::FontRef<'static>>,
    styled: bool,
    elevated: bool,
    db: PathBuf,
    export: Arc<Mutex<Option<String>>>,
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
        watch_display();
        let engine = Engine::start(
            config.clone(),
            Some(db.clone()),
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
            drawn_samples: u64::MAX,
            visible: false,
            shown_at: Instant::now(),
            px,
            strip,
            strip_attached: false,
            font: system_font(),
            styled: false,
            elevated: elevated(),
            db,
            export: Arc::default(),
        };
        app.apply_visibility();
        app.register_hotkey();
        app
    }

    /// With the Strip attached only the Latency icon's slot stays, as the Menu Icon.
    fn shown(&self, s: Section) -> bool {
        if self.strip_attached {
            s == Section::Latency
        } else {
            !self.config.section(s).tray.is_empty()
        }
    }

    fn apply_visibility(&self) {
        for (s, icon) in &self.icons {
            let _ = icon.set_visible(self.shown(*s));
        }
    }

    /// Runs off the UI thread with its own read-only connection; the result lands in the Panel's notice.
    fn export_history(&self, ctx: egui::Context) {
        let dir = std::env::var_os("USERPROFILE")
            .map(|p| PathBuf::from(p).join("Downloads"))
            .filter(|d| d.is_dir())
            .unwrap_or_else(config::dir);
        let out = dir.join(format!("blinkengraphs-history-{}.csv", model::now()));
        let (db, status) = (self.db.clone(), self.export.clone());
        *status.lock().unwrap() = Some("Exporting…".into());
        std::thread::spawn(move || {
            let msg = match history::export_csv(&db, &out) {
                Ok(rows) => {
                    let _ = std::process::Command::new("explorer")
                        .arg(format!("/select,{}", out.display()))
                        .spawn();
                    format!("Saved {rows} rows to {}", out.display())
                }
                Err(e) => format!("Export failed: {e}"),
            };
            *status.lock().unwrap() = Some(msg);
            ctx.request_repaint();
        });
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
        if s.sample_count == self.drawn_samples {
            return;
        }
        self.drawn_samples = s.sample_count;
        let taskbar = taskbar_theme();
        let attached = self.config.surface == Surface::TaskbarStrip
            && self.strip.geometry().is_some_and(|(scale, h)| {
                let img = meters::strip(
                    &s.history,
                    &self.config,
                    h,
                    scale,
                    self.font.as_ref(),
                    taskbar,
                );
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
            if !self.shown(*section) {
                continue;
            }
            let img = if attached {
                meters::menu_icon(&s.history, &self.config, &s.findings, self.px, taskbar)
            } else {
                meters::draw(
                    *section,
                    &s.history,
                    &self.config,
                    &self.config.section(*section).tray,
                    self.px,
                    taskbar,
                )
            };
            if let Ok(i) = Icon::from_rgba(render::to_rgba(&img), self.px, self.px) {
                let _ = icon.set_icon(Some(i));
            }
            let tip = match self.config.menu_icon {
                MenuIcon::Meter(m) if attached => m,
                _ => *section,
            };
            let _ = icon.set_tooltip(Some(meters::tooltip(
                tip,
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
        if !self.styled {
            self.styled = style_window();
            // eframe shows the root viewport after its first frame even when built invisible.
            if self.styled && !self.visible {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
        }
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
        let notice = self.export.lock().unwrap().clone();
        let s = shared.lock().unwrap();
        let view = View {
            history: &s.history,
            findings: &s.findings,
            processes: &s.processes,
            adapters: &s.adapters,
            now: model::now(),
            elevated: self.elevated,
            config_error: self.error.as_deref(),
            notice: notice.as_deref(),
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
                    self.drawn_samples = u64::MAX;
                }
                Action::ExportHistory => self.export_history(ui.ctx().clone()),
                Action::Quit => self.quit(),
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.hide(ui.ctx());
        }
    }
}

/// Win11 rounded corners, dark frame and a hairline border on the borderless Panel window.
fn style_window() -> bool {
    use windows_sys::Win32::Graphics::Dwm::*;
    unsafe {
        let hwnd = windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(
            std::ptr::null(),
            windows_sys::core::w!("blinkengraphs"),
        );
        if hwnd.is_null() {
            return false;
        }
        let set = |attr: i32, v: u32| {
            DwmSetWindowAttribute(hwnd, attr as u32, (&v as *const u32).cast(), 4)
        };
        set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND as u32);
        set(DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
        set(DWMWA_BORDER_COLOR, 0x003d312a);
    }
    true
}

/// Windows lets the taskbar be light while apps are dark, so Meters follow the taskbar, not the app theme.
fn taskbar_theme() -> render::Taskbar {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let mut light: u32 = 0;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows_sys::core::w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            windows_sys::core::w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut light as *mut u32).cast(),
            &mut size,
        )
    } == 0;
    if ok && light != 0 {
        render::Taskbar::Light
    } else {
        render::Taskbar::Dark
    }
}

fn system_font() -> Option<render::FontRef<'static>> {
    let bytes = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf").ok()?;
    render::FontRef::try_from_slice(Box::leak(bytes.into_boxed_slice())).ok()
}

static DISPLAY_OFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// GUID_CONSOLE_DISPLAY_STATE: 0 off, 1 on, 2 dimmed.
/// https://learn.microsoft.com/windows/win32/power/power-setting-guids
fn watch_display() {
    use windows_sys::Win32::System::{Power::*, SystemServices::GUID_CONSOLE_DISPLAY_STATE};
    unsafe extern "system" fn on_change(
        _: *const core::ffi::c_void,
        _: u32,
        setting: *const core::ffi::c_void,
    ) -> u32 {
        let s = unsafe { &*(setting as *const POWERBROADCAST_SETTING) };
        if s.DataLength >= 4 {
            DISPLAY_OFF.store(s.Data[0] == 0, Ordering::Relaxed);
        }
        0
    }
    // Leaked: Windows may read the parameters for the life of the registration.
    let params = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(on_change),
        Context: std::ptr::null_mut(),
    }));
    let mut handle = std::ptr::null_mut();
    unsafe {
        PowerSettingRegisterNotification(
            &GUID_CONSOLE_DISPLAY_STATE,
            windows_sys::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK,
            (params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS).cast(),
            &mut handle,
        );
    }
}

fn elevated() -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut e as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && e.TokenIsElevated != 0
    }
}

/// A locked session can't switch to the input desktop, so `SwitchDesktop` failing means locked.
fn idle() -> bool {
    if DISPLAY_OFF.load(Ordering::Relaxed) {
        return true;
    }
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
