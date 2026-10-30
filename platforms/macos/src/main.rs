//! Minimal macOS shell. Run with `cargo run -p browser-macos`.
//!
//! The window, toolbar, and page rendering come from the shared
//! `browser-shell` crate; this crate only describes the macOS window, which
//! keeps the system's own traffic-light buttons and frame.

#![forbid(unsafe_code)]

#[cfg(target_os = "macos")]
fn main() -> Result<(), winit::error::EventLoopError> {
    use std::path::PathBuf;

    use winit::platform::macos::WindowAttributesExtMacOS;

    browser_shell::run(browser_shell::Platform {
        // Make the native titlebar transparent and let content extend under
        // it so the toolbar spans the full window width, Safari-style. The
        // system's traffic lights stay where macOS puts them, at the top
        // left, and the toolbar leaves room for them.
        configure_window: |attributes| {
            attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
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
        // Times is the default serif font Safari and Chrome use for pages;
        // the UI fonts are fallbacks.
        page_font_files: || {
            [
                "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
                "/System/Library/Fonts/Times.ttc",
                "/System/Library/Fonts/Supplemental/Georgia.ttf",
                "/System/Library/Fonts/Helvetica.ttc",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect()
        },
        window_controls_inset: 72.0,
    })
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("browser-macos requires macOS.");
}
