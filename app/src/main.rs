#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("windows only");
}

/// M0: replays a fixture through one Tray Icon per Section until real collectors land in M1.
#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(windows)]
mod win {
    use model::Section;
    use render::{Meter, meter, to_rgba};
    use tray_icon::{
        Icon, TrayIcon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub fn run() {
        let scenario = fixtures::all().into_iter().nth(2).unwrap();
        let px = unsafe { GetSystemMetrics(SM_CXSMICON) } as u32;
        let mut quit_ids = Vec::new();
        let icons: Vec<(Section, TrayIcon)> = Section::ALL
            .iter()
            .map(|&section| {
                let quit = MenuItem::new("Quit blinkengraphs", true, None);
                quit_ids.push(quit.id().clone());
                let menu = Menu::new();
                menu.append(&quit).unwrap();
                let icon = TrayIconBuilder::new()
                    .with_tooltip(format!(
                        "{} (fixture: {})",
                        section.label(),
                        scenario.caption
                    ))
                    .with_menu(Box::new(menu))
                    .build()
                    .unwrap();
                (section, icon)
            })
            .collect();

        let mut tick = 0usize;
        let draw = |tick: usize| {
            for (section, icon) in &icons {
                let w = |v: &[f32]| window(v, tick, px);
                let (a, b, rtt);
                let m = match section {
                    Section::Cpu => {
                        a = w(&scenario.cpu);
                        Meter::Level(&a)
                    }
                    Section::Memory => {
                        a = w(&scenario.memory);
                        Meter::Level(&a)
                    }
                    Section::Disk => {
                        a = w(&scenario.disk);
                        Meter::Level(&a)
                    }
                    Section::Network => {
                        a = w(&scenario.down);
                        b = w(&scenario.up);
                        Meter::Duplex { down: &a, up: &b }
                    }
                    Section::Latency => {
                        rtt = window(&scenario.rtt, tick, px);
                        Meter::Latency(&rtt)
                    }
                };
                let img = meter(*section, &m, px);
                icon.set_icon(Some(Icon::from_rgba(to_rgba(&img), px, px).unwrap()))
                    .unwrap();
            }
        };
        draw(tick);

        unsafe {
            SetTimer(std::ptr::null_mut(), 0, 1000, None);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_TIMER {
                    tick += 1;
                    draw(tick);
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
                while let Ok(e) = MenuEvent::receiver().try_recv() {
                    if quit_ids.contains(&e.id) {
                        return;
                    }
                }
            }
        }
    }

    fn window<T: Clone>(values: &[T], tick: usize, px: u32) -> Vec<T> {
        values
            .iter()
            .cycle()
            .skip(tick % values.len())
            .take(px as usize)
            .cloned()
            .collect()
    }
}
