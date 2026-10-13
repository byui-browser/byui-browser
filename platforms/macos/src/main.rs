//! Minimal macOS shell. Run with `cargo run -p browser-macos`.
//!
//! The window, toolbar, and page rendering come from the shared
//! `browser-shell` crate; this crate only describes the macOS window.

#![forbid(unsafe_code)]

#[cfg(target_os = "macos")]
fn main() -> Result<(), winit::error::EventLoopError> {
    use std::path::PathBuf;

    use winit::platform::macos::WindowAttributesExtMacOS;

    browser_shell::run(browser_shell::Platform {
        // Hide the native titlebar so the toolbar can span the full window
        // width; the shell draws the window controls itself. The frameless
        // window keeps macOS's own edge resizing.
        configure_window: |attributes| {
            attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
                .with_titlebar_buttons_hidden(true)
        },
        ui_font_files: || {
            [
                "/System/Library/Fonts/SFNS.ttf",
                "/System/Library/Fonts/HelveticaNeue.ttc",
                "/System/Library/Fonts/Helvetica.ttc",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect()
        },
        resize_borders: false,
    })
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("browser-macos requires macOS.");
}
