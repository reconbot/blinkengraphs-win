# Draw the Panel in software when OpenGL 2.0 is missing

eframe's glow renderer needs OpenGL 2.0. A VM or VDI host without a GPU driver offers only Windows' built-in OpenGL 1.1, so the app exited with nothing on screen. CI's Windows runners are such hosts, which is how the installer's upgrade test found it.

- At startup `opengl2()` reads `GL_VERSION` from a throwaway WGL context. With 2.0 or later the app runs eframe as before. Without it, `soft::run` drives the same `App` through winit, egui-winit and softbuffer, rasterizing on the CPU.
- The check runs first because winit allows one event loop per process, so there's no retrying after eframe fails.
- The rasterizer is `crates/softpanel`, vendored from egui_software_backend 0.0.3 (MIT OR Apache-2.0). Upstream targets egui 0.34; the port to 0.37 changed two imports. Its winit runner and stats were dropped. The rasterizer is no_std code over egui's tessellated meshes, so it will track egui upgrades with small diffs.
- `App` no longer takes eframe types: `tick(ctx)` and `draw(ui)` serve both runners.
- Cost: about 237 KB of exe, leaving 0.15 MiB under the 7 MiB budget.

Rejected:
- wgpu, whose DX12 backend falls back to WARP: it would be the bigger exe.
- Shipping Mesa's opengl32.dll: tens of MB.
- An "OpenGL 2.0 required" message: it leaves VDI users without the app.
