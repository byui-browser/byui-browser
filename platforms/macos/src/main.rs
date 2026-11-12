//! Minimal macOS shell. Run with `cargo run -p browser-macos`.
//!
//! The window, toolbar, and page rendering come from the shared
//! `browser-shell` crate; this crate only describes the macOS window, which
//! keeps the system's own traffic-light buttons and frame.

#![deny(unsafe_code)]

#[cfg(target_os = "macos")]
fn main() -> Result<(), winit::error::EventLoopError> {
    use std::path::PathBuf;

    use winit::platform::macos::WindowAttributesExtMacOS;

    browser_shell::run(browser_shell::Platform {
        // Make the native titlebar transparent and let content extend under
        // it so the toolbar spans the full window width, Safari-style. The
        // system's traffic lights stay at the top left, and the toolbar
        // leaves room for them.
        configure_window: |attributes| {
            attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
        },
        window_created: center_traffic_lights,
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

/// Gives the window an empty unified-style `NSToolbar`, Safari's setup. That
/// makes AppKit's titlebar 52 points tall, the shared toolbar's height, and
/// AppKit centers the traffic lights vertically in it, so they line up with
/// the Back, Forward, and Reload buttons. The toolbar holds no items; the
/// transparent titlebar and hidden title keep it invisible, and the content
/// view still extends under it.
///
/// The window is also made unmovable by AppKit's own titlebar dragging, so
/// drags reach the toolbar controls; the shell starts window drags itself.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn center_traffic_lights(window: &winit::window::Window) {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSToolbar, NSView, NSWindowToolbarStyle};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's AppKit handle points at the window's live content
    // `NSView`, which `window` keeps alive for this call; retaining it only
    // adds a reference.
    let Some(view) = (unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }) else {
        return;
    };
    let Some(ns_window) = view.window() else {
        return;
    };
    let toolbar = NSToolbar::new(mtm);
    ns_window.setToolbar(Some(&toolbar));
    ns_window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    // AppKit would otherwise move the window for any drag in the now
    // 52-point titlebar, because winit's content view reports that a mouse
    // down there can move the window. That steals drags meant to select text
    // in the address field. The shared shell still moves the window from
    // empty toolbar space with `Window::drag_window`.
    ns_window.setMovable(false);
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("browser-macos requires macOS.");
}
