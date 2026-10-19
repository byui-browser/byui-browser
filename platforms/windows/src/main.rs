//! Minimal Windows shell. Run with `cargo run -p byui-browser-windows`.
//!
//! The shared `browser-shell` crate owns the toolbar, page rendering, and
//! pointer behavior. This crate only supplies Windows' native window setup
//! and its system UI font candidates.

#![forbid(unsafe_code)]

#[cfg(target_os = "windows")]
fn main() {
    browser_shell::run(browser_shell::Platform {
        // Keep the native Windows titlebar and frame. Windows supplies the
        // rounded normal window and square maximized treatment.
        configure_window: |attributes| attributes,
        ui_font_files: windows::ui_font_files,
        window_controls_inset: 0.0,
    })
    .expect("Windows browser shell event loop failed");
}

#[cfg(target_os = "windows")]
mod windows {
    use std::path::PathBuf;

    /// Returns Windows UI font candidates in preference order.
    pub fn ui_font_files() -> Vec<PathBuf> {
        [
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\segoeuil.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("byui-browser-windows requires Windows.");
}
