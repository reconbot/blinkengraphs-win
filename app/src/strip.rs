//! Taskbar Strip: a layered child window inside `Shell_TrayWnd`, left of the notification area.
//! Undocumented placement (ADR 0001); any failure reports `false` so the app falls back to Tray Icons.

use render::Image;
use std::{ptr::null_mut, sync::OnceLock};
use windows_sys::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
            CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
            MapWindowPoints, ReleaseDC, SelectObject,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::w,
};

static ON_CLICK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

pub struct Strip {
    hwnd: HWND,
}

impl Strip {
    pub fn new(on_click: impl Fn() + Send + Sync + 'static) -> Self {
        let _ = ON_CLICK.set(Box::new(on_click));
        Self { hwnd: null_mut() }
    }

    /// Logical-to-pixel scale and pixel height the next image should have; `None` when there is no horizontal taskbar.
    pub fn geometry(&self) -> Option<(f32, u32)> {
        let (taskbar, rect) = taskbar()?;
        let scale = unsafe { GetDpiForWindow(taskbar) }.max(96) as f32 / 96.0;
        let h = (rect.bottom - rect.top) as f32;
        Some((scale, (h - 8.0 * scale).max(16.0) as u32))
    }

    /// Places and paints the Strip; `false` means detached and the caller should fall back.
    pub fn show(&mut self, img: &Image) -> bool {
        let Some((taskbar, tb)) = taskbar() else {
            return false;
        };
        unsafe {
            if self.hwnd.is_null() || IsWindow(self.hwnd) == 0 || GetParent(self.hwnd) != taskbar {
                self.destroy();
                self.hwnd = create(taskbar);
                if self.hwnd.is_null() {
                    return false;
                }
            }
            let notify = FindWindowExW(taskbar, null_mut(), w!("TrayNotifyWnd"), std::ptr::null());
            let mut right = tb.right - 320;
            if !notify.is_null() {
                let mut r: RECT = std::mem::zeroed();
                GetWindowRect(notify, &mut r);
                right = r.left;
            }
            let (w, h) = (img.width() as i32, img.height() as i32);
            let scale = GetDpiForWindow(taskbar).max(96) as i32 / 96;
            let mut pt = POINT {
                x: right - w - 8 * scale,
                y: tb.top + ((tb.bottom - tb.top) - h) / 2,
            };
            if pt.x < tb.left {
                return false;
            }
            MapWindowPoints(null_mut(), taskbar, &mut pt, 1);
            SetWindowPos(
                self.hwnd,
                HWND_TOP,
                pt.x,
                pt.y,
                w,
                h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            paint(self.hwnd, img)
        }
    }

    pub fn destroy(&mut self) {
        if !self.hwnd.is_null() {
            unsafe { DestroyWindow(self.hwnd) };
            self.hwnd = null_mut();
        }
    }
}

impl Drop for Strip {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn taskbar() -> Option<(HWND, RECT)> {
    unsafe {
        let tb = FindWindowW(w!("Shell_TrayWnd"), std::ptr::null());
        if tb.is_null() || IsWindowVisible(tb) == 0 {
            return None;
        }
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(tb, &mut r);
        ((r.right - r.left) > (r.bottom - r.top)).then_some((tb, r))
    }
}

unsafe fn create(parent: HWND) -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = w!("blinkengraphs-strip");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc_),
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(null_mut(), IDC_HAND),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("blinkengraphs"),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            1,
            1,
            parent,
            null_mut(),
            hinst,
            std::ptr::null(),
        )
    }
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_LBUTTONUP
        && let Some(f) = ON_CLICK.get()
    {
        f();
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe fn paint(hwnd: HWND, img: &Image) -> bool {
    unsafe {
        let (w, h) = (img.width() as i32, img.height() as i32);
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bits: *mut core::ffi::c_void = null_mut();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let bmp = CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let ok = !bmp.is_null() && !bits.is_null();
        if ok {
            // tiny-skia is premultiplied RGBA; GDI wants premultiplied BGRA.
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            for (d, s) in dst.chunks_exact_mut(4).zip(img.data().chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
            }
            let old = SelectObject(mem, bmp);
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let updated = UpdateLayeredWindow(
                hwnd,
                screen,
                null_mut(),
                &size,
                mem,
                &src,
                0,
                &blend,
                ULW_ALPHA,
            );
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(null_mut(), screen);
            return updated != 0;
        }
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
        false
    }
}
