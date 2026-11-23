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
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
    System::{
        StationsAndDesktops::{
            CloseDesktop, DESKTOP_SWITCHDESKTOP, OpenInputDesktop, SwitchDesktop,
        },
        Threading::{
            CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent,
            WaitForSingleObject,
        },
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
    /// Another launch found this one running and asked it to show the Panel.
    Relaunched,
    /// An installer's Restart Manager, or sign-out, asked the app to close.
    EndSession,
}

const SHOW_EVENT: &str = "Local\\blinkengraphs-show\0";

pub fn run() {
    let name: Vec<u16> = "Local\\blinkengraphs-single-instance\0"
        .encode_utf16()
        .collect();
    unsafe {
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let show: Vec<u16> = SHOW_EVENT.encode_utf16().collect();
            let e = OpenEventW(EVENT_MODIFY_STATE, 0, show.as_ptr());
            if !e.is_null() {
                SetEvent(e);
                CloseHandle(e);
            }
            return;
        }
    }
    install_crash_log();
    let path = config::dir().join("config.toml");
    let (config, error) = Config::load(&path);
    let viewport = egui::ViewportBuilder::default()
        .with_title("blinkengraphs")
        .with_inner_size(PANEL)
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_always_on_top()
        .with_visible(false)
        .with_position([-32000.0, -32000.0]);
    let options = eframe::NativeOptions {
        viewport: viewport.clone(),
        ..Default::default()
    };
    if !opengl2() {
        // A VM or VDI host without a GPU driver offers only Windows' OpenGL 1.1; egui_glow needs 2.0.
        if let Err(e) = crate::soft::run(viewport, move |ctx| App::new(ctx, config, error, path)) {
            crash_log(&format!("software window failed: {e}"));
        }
        return;
    }
    if let Err(e) = eframe::run_native(
        "blinkengraphs",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(&cc.egui_ctx, config, error, path)))),
    ) {
        crash_log(&format!("window failed to start: {e}"));
    }
}

/// Whether the default OpenGL driver is 2.0 or later, from a throwaway legacy WGL context.
/// winit allows one event loop per process, so this has to be known before eframe starts.
fn opengl2() -> bool {
    use windows_sys::Win32::{
        Graphics::{Gdi::*, OpenGL::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            windows_sys::core::w!("STATIC"),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        let dc = GetDC(hwnd);
        let pfd = PIXELFORMATDESCRIPTOR {
            nSize: size_of::<PIXELFORMATDESCRIPTOR>() as u16,
            nVersion: 1,
            dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
            iPixelType: PFD_TYPE_RGBA,
            cColorBits: 32,
            iLayerType: PFD_MAIN_PLANE as u8,
            ..std::mem::zeroed()
        };
        let mut major = 0;
        if SetPixelFormat(dc, ChoosePixelFormat(dc, &pfd), &pfd) != 0 {
            let rc = wglCreateContext(dc);
            if !rc.is_null() && wglMakeCurrent(dc, rc) != 0 {
                let v = glGetString(GL_VERSION);
                if !v.is_null() {
                    major = std::ffi::CStr::from_ptr(v.cast())
                        .to_bytes()
                        .iter()
                        .take_while(|b| b.is_ascii_digit())
                        .fold(0u32, |n, b| n * 10 + u32::from(b - b'0'));
                }
                wglMakeCurrent(std::ptr::null_mut(), std::ptr::null_mut());
            }
            if !rc.is_null() {
                wglDeleteContext(rc);
            }
        }
        ReleaseDC(hwnd, dc);
        DestroyWindow(hwnd);
        major >= 2
    }
}

pub(crate) struct App {
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
    fn new(ctx: &egui::Context, config: Config, error: Option<String>, path: PathBuf) -> Self {
        let ctx = ctx.clone();
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
        let relaunch = forward(tx.clone(), ctx.clone());
        watch_relaunch(move || relaunch(UiEvent::Relaunched));
        let end = forward(tx.clone(), ctx.clone());
        watch_session_end(move || end(UiEvent::EndSession));
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
        watch_power(engine.commands());

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
        let now = model::now();
        let attached = self.config.surface == Surface::TaskbarStrip
            && self.strip.geometry().is_some_and(|(scale, h)| {
                let img = meters::strip(
                    &s.history,
                    &self.config,
                    h,
                    scale,
                    self.font.as_ref(),
                    taskbar,
                    now,
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
                meters::menu_icon(&s.history, &self.config, &s.findings, self.px, taskbar, now)
            } else {
                meters::draw(
                    *section,
                    &s.history,
                    &self.config,
                    &self.config.section(*section).tray,
                    self.px,
                    taskbar,
                    now,
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
                now,
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
        self.engine.send(Command::PanelOpen(true));
    }

    fn hide(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.visible = false;
        self.engine.send(Command::PanelOpen(false));
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
        self.engine.shutdown(Duration::from_secs(3));
        std::process::exit(0)
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx)
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui)
    }
}

impl App {
    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
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
                UiEvent::Relaunched => self.show(ctx, None, None),
                UiEvent::EndSession => self.quit(),
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

    pub(crate) fn draw(&mut self, ui: &mut egui::Ui) {
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
            links: &s.links,
            public: s.public.as_ref(),
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

static POWER: std::sync::OnceLock<std::sync::mpsc::Sender<engine::Command>> =
    std::sync::OnceLock::new();

/// Suspend begins a Pause and resume ends it (spec `rules/history.md`, Pauses). Registering
/// also opts the app into Modern Standby's suspend notice; PBT_APMRESUMEAUTOMATIC is sent on
/// every resume, PBT_APMRESUMESUSPEND only when a user woke the machine (research 07 §1).
/// https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registersuspendresumenotification
fn watch_power(commands: std::sync::mpsc::Sender<engine::Command>) {
    use windows_sys::Win32::{System::Power::*, UI::WindowsAndMessaging::*};
    unsafe extern "system" fn on_power(
        _: *const core::ffi::c_void,
        kind: u32,
        _: *const core::ffi::c_void,
    ) -> u32 {
        // Windows allows about 2 s after PBT_APMSUSPEND; a channel send returns at once and
        // the engine writes Raw on its own thread.
        let c = match kind {
            PBT_APMSUSPEND => engine::Command::Suspend(model::now_f64()),
            PBT_APMRESUMEAUTOMATIC => engine::Command::Resume(model::now_f64()),
            _ => return 0,
        };
        if let Some(tx) = POWER.get() {
            let _ = tx.send(c);
        }
        0
    }
    if POWER.set(commands).is_err() {
        return;
    }
    // Leaked: Windows may read the parameters for the life of the registration.
    let params = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(on_power),
        Context: std::ptr::null_mut(),
    }));
    unsafe {
        RegisterSuspendResumeNotification(
            (params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS).cast(),
            DEVICE_NOTIFY_CALLBACK,
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

/// A second launch sets the named event; this thread turns that into showing the Panel.
fn watch_relaunch(on_show: impl Fn() + Send + 'static) {
    let name: Vec<u16> = SHOW_EVENT.encode_utf16().collect();
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) } as usize;
    if event == 0 {
        return;
    }
    std::thread::spawn(move || {
        loop {
            if unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } != WAIT_OBJECT_0 {
                return;
            }
            on_show();
        }
    });
}

/// An MSI upgrade's Restart Manager closes apps holding its files by sending WM_QUERYENDSESSION
/// and WM_ENDSESSION to their top-level windows, then relaunches the ones registered for restart.
/// winit's window ignores both, so this hidden one quits cleanly instead.
/// https://learn.microsoft.com/windows/win32/rstmgr/guidelines-for-applications
fn watch_session_end(on_end: impl Fn() + Send + 'static) {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Recovery::*},
        UI::WindowsAndMessaging::*,
    };
    thread_local! {
        static ON_END: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
    }
    unsafe extern "system" fn proc_(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_QUERYENDSESSION => 1,
            WM_ENDSESSION if wp != 0 => {
                ON_END.with(|f| f.borrow().as_ref().map(|f| f()));
                // The UI thread saves and exits the process; returning first would let Windows
                // treat the app as closed while it is still writing.
                std::thread::sleep(Duration::from_secs(10));
                0
            }
            _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
        }
    }
    // No restart after a reboot or crash: the Run value starts the app at sign-in.
    unsafe {
        RegisterApplicationRestart(
            std::ptr::null(),
            RESTART_NO_CRASH | RESTART_NO_HANG | RESTART_NO_REBOOT,
        )
    };
    std::thread::spawn(move || unsafe {
        ON_END.with(|f| *f.borrow_mut() = Some(Box::new(on_end)));
        let hinst = GetModuleHandleW(std::ptr::null());
        let class: Vec<u16> = "blinkengraphs-session\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc_),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        // Top-level (not message-only): Restart Manager only messages top-level windows.
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }
    });
}

/// Release builds abort on panic with no console, so the reason is otherwise lost.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crash_log(&format!(
            "{info}\n{}",
            std::backtrace::Backtrace::force_capture()
        ));
        default(info);
    }));
}

pub(crate) fn crash_log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config::dir().join("crash.log"))
    {
        let _ = writeln!(f, "{} v{} {msg}", model::now(), env!("CARGO_PKG_VERSION"));
    }
}
