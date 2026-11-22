//! The Panel without OpenGL: one winit window driven like eframe drives its root viewport,
//! rasterized on the CPU by `softpanel` and presented through softbuffer (GDI on Windows).

use crate::win::App;
use eframe::egui::{self, TexturesDelta, ViewportBuilder, ViewportId, ViewportInfo};
use softpanel::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use std::{
    num::NonZeroU32,
    rc::Rc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

/// Another thread asked for a repaint at this time.
struct Wake(Instant);

struct Surface {
    window: Rc<Window>,
    state: egui_winit::State,
    info: ViewportInfo,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
}

struct Runner<F> {
    ctx: egui::Context,
    viewport: ViewportBuilder,
    make: Option<F>,
    app: Option<App>,
    surface: Option<Surface>,
    renderer: EguiSoftwareRender,
    /// Texture updates that arrived while the Panel was hidden; the rasterizer needs them all.
    textures: TexturesDelta,
    next: Option<Instant>,
}

pub fn run(
    viewport: ViewportBuilder,
    make: impl FnOnce(&egui::Context) -> App,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::<Wake>::with_user_event().build()?;
    let ctx = egui::Context::default();
    let proxy = event_loop.create_proxy();
    ctx.set_request_repaint_callback(move |info| {
        let _ = proxy.send_event(Wake(Instant::now() + info.delay));
    });
    let mut runner = Runner {
        ctx,
        viewport,
        make: Some(make),
        app: None,
        surface: None,
        renderer: EguiSoftwareRender::new(ColorFieldOrder::Bgra),
        textures: TexturesDelta::default(),
        next: Some(Instant::now()),
    };
    event_loop.run_app(&mut runner)?;
    Ok(())
}

impl<F: FnOnce(&egui::Context) -> App> Runner<F> {
    fn create(&mut self, el: &ActiveEventLoop) -> Result<(), Box<dyn std::error::Error>> {
        let attrs = egui_winit::create_winit_window_attributes(&self.ctx, self.viewport.clone());
        let window = Rc::new(el.create_window(attrs)?);
        egui_winit::apply_viewport_builder_to_window(&self.ctx, &window, &self.viewport);
        let state = egui_winit::State::new(
            self.ctx.clone(),
            ViewportId::ROOT,
            el,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );
        let context = softbuffer::Context::new(window.clone())?;
        let surface = softbuffer::Surface::new(&context, window.clone())?;
        let mut info = ViewportInfo::default();
        egui_winit::update_viewport_info(&mut info, &self.ctx, &window, true);
        self.surface = Some(Surface {
            window,
            state,
            info,
            surface,
        });
        if let Some(make) = self.make.take() {
            self.app = Some(make(&self.ctx));
        }
        Ok(())
    }

    fn frame(&mut self) {
        let (Some(s), Some(app)) = (self.surface.as_mut(), self.app.as_mut()) else {
            return;
        };
        let mut raw = s.state.take_egui_input(&s.window);
        egui_winit::update_viewport_info(&mut s.info, &self.ctx, &s.window, false);
        raw.viewports.insert(ViewportId::ROOT, s.info.clone());
        let out = self.ctx.run_ui(raw, |ui| {
            app.tick(ui.ctx());
            app.draw(ui);
        });
        s.state
            .handle_platform_output(&s.window, out.platform_output);
        let mut delay = Duration::MAX;
        if let Some(v) = out.viewport_output.get(&ViewportId::ROOT) {
            delay = v.repaint_delay;
            egui_winit::process_viewport_commands(
                &self.ctx,
                &mut s.info,
                v.commands.clone(),
                &s.window,
                &mut Vec::new(),
            );
        }
        self.next = Instant::now().checked_add(delay);
        self.textures.append(out.textures_delta);
        if s.window.is_visible() == Some(false) {
            return;
        }
        let size = s.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        let fill = self.ctx.global_style().visuals.panel_fill;
        let painted = (|| -> Result<(), softbuffer::SoftBufferError> {
            s.surface.resize(w, h)?;
            let mut buf = s.surface.buffer_mut()?;
            buf.fill(u32::from_be_bytes([0, fill.r(), fill.g(), fill.b()]));
            let pixels: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut buf);
            self.renderer.render(
                &mut BufferMutRef::new(pixels, w.get() as usize, h.get() as usize),
                &prims,
                &self.textures,
                out.pixels_per_point,
            );
            self.textures.clear();
            buf.present()
        })();
        if let Err(e) = painted {
            crate::win::crash_log(&format!("software paint failed: {e}"));
        }
    }
}

impl<F: FnOnce(&egui::Context) -> App> ApplicationHandler<Wake> for Runner<F> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.surface.is_none() {
            if let Err(e) = self.create(el) {
                crate::win::crash_log(&format!("software window failed: {e}"));
                el.exit();
                return;
            }
            self.frame();
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, Wake(at): Wake) {
        self.next = Some(self.next.map_or(at, |n| n.min(at)));
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(s) = self.surface.as_mut() else {
            return;
        };
        match event {
            WindowEvent::RedrawRequested => self.frame(),
            // The Panel hides rather than closes; Quit exits the process.
            WindowEvent::CloseRequested => {}
            event => {
                if s.state.on_window_event(&s.window, &event).repaint {
                    self.next = Some(Instant::now());
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.next.is_some_and(|n| n <= Instant::now()) {
            self.frame();
        }
        el.set_control_flow(match self.next {
            Some(n) => ControlFlow::WaitUntil(n),
            None => ControlFlow::Wait,
        });
    }
}
