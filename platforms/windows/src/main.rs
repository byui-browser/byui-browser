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
        // rounded normal window and square maximized treatment.
        configure_window: |attributes| attributes,
        ui_font_files: windows::ui_font_files,
        page_font_files: windows::page_font_files,
        window_controls_inset: 0.0,
    })
    .expect("Windows browser shell event loop failed");
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
