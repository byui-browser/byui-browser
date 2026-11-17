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
        window_changed: sync_native_toolbar,
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

/// Normal windows keep the existing unified titlebar/traffic-light layout.
#[cfg(target_os = "macos")]
fn center_traffic_lights(window: &winit::window::Window) {
    sync_native_toolbar(window);
}

/// AppKit manages native toolbars specially in full screen. Detach the empty
/// titlebar-sizing helper there so it cannot obscure the software-painted tabs
/// and address row in the full-size content view. Restore it on exit. Native
/// decorations remain AppKit-owned; winit keeps its own window delegate.
/// Returns true only when a native layout change requires a fresh frame.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn sync_native_toolbar(window: &winit::window::Window) -> bool {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSToolbar, NSView, NSWindowStyleMask, NSWindowToolbarStyle};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    // SAFETY: winit's AppKit handle points at the window's live content
    // NSView, which `window` keeps alive for this main-thread call. Retaining
    // it only adds a reference; it is never retained past this function.
    let Some(view) = (unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }) else {
        return false;
    };
    let Some(ns_window) = view.window() else {
        return false;
    };
    // Inspect AppKit's actual style rather than winit's requested mode, which
    // can change before the native full-screen transition has finished.
    let fullscreen = ns_window
        .styleMask()
        .contains(NSWindowStyleMask::FullScreen);
    let attached = ns_window.toolbar().is_some();
    if fullscreen {
        if !attached {
            return false;
        }
        ns_window.setToolbar(None);
    } else {
        if attached {
            return false;
        }
        let toolbar = NSToolbar::new(mtm);
        ns_window.setToolbar(Some(&toolbar));
        ns_window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    }
    ns_window.setTitlebarAppearsTransparent(true);
    // Native titlebar dragging must not steal address-field text-selection
    // drags. Empty chrome still uses Window::drag_window, as before.
    ns_window.setMovable(false);
    true
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("browser-macos requires macOS.");
}
