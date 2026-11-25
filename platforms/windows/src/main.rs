//! Minimal Windows shell. Run with `cargo run -p byui-browser-windows`.
//!
//! The shared `browser-shell` crate owns the toolbar, page rendering, and
//! pointer behavior. This crate only supplies Windows' native window setup
//! and its system UI and page font candidates.

#![forbid(unsafe_code)]

#[cfg(target_os = "windows")]
fn main() {
    browser_shell::run(browser_shell::Platform {
        // Keep the native Windows titlebar and frame. Windows supplies the
        // native titlebar and resize borders; the callback below makes the
        // Windows 11 corner policy explicit for restored and maximized states.
        configure_window: |attributes| attributes,
        window_created: windows_window_created,
        window_changed: sync_windows_chrome,
        ui_font_files: windows::ui_font_files,
        page_font_files: windows::page_font_files,
        window_controls_inset: 0.0,
    })
    .expect("Windows browser shell event loop failed");
}

#[cfg(target_os = "windows")]
fn windows_window_created(window: &winit::window::Window) {
    sync_windows_chrome(window);
}

#[cfg(target_os = "windows")]
fn sync_windows_chrome(window: &winit::window::Window) -> bool {
    use winit::platform::windows::{CornerPreference, WindowExtWindows};

    // Windows 11 normally chooses these values automatically, but explicitly
    // applying them keeps the browser shell's restored/maximized behavior
    // stable when the system theme, snap layout, or native frame changes.
    let preference = if window.is_maximized() {
        CornerPreference::DoNotRound
    } else {
        CornerPreference::Round
    };
    window.set_corner_preference(preference);
    false
}

#[cfg(target_os = "windows")]
mod windows {
    use std::path::PathBuf;

    /// Returns Windows UI font candidates in preference order.
    pub fn ui_font_files() -> Vec<PathBuf> {
        fonts(&["segoeui.ttf", "segoeuil.ttf", "arial.ttf"])
    }

    /// Returns page font candidates in preference order: Times New Roman,
    /// the serif font Edge and Chrome use by default, then other serif and
    /// sans-serif fonts that ship with Windows.
    pub fn page_font_files() -> Vec<PathBuf> {
        fonts(&[
            "times.ttf",
            "georgia.ttf",
            "cambria.ttc",
            "arial.ttf",
            "segoeui.ttf",
        ])
    }

    /// Paths of `names` in the system fonts folder, which lives under
    /// `%WINDIR%` (usually `C:\Windows`).
    fn fonts(names: &[&str]) -> Vec<PathBuf> {
        let windows =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        names
            .iter()
            .map(|name| windows.join("Fonts").join(name))
            .collect()
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("byui-browser-windows requires Windows.");
}
