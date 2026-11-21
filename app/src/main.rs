#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod strip;
#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::run();
    #[cfg(not(windows))]
    eprintln!(
        "blinkengraphs runs on Windows; use `cargo run -p screenshots` to preview on this OS"
    );
}
