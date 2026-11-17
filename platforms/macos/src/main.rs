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
        // it so the tab strip and toolbar span the full window width. The
        // system's traffic lights stay at the top left, centered on the tab
        // row, and the tab strip leaves room for them.
        configure_window: |attributes| {
            attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
        },
        window_created: center_traffic_lights,
        window_changed: sync_native_chrome,
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
        // The traffic lights end near x = 72; the rest leaves a Brave-like gap
        // before the first tab.
        window_controls_inset: 88.0,
    })
}

/// Logical distance from the window's left edge to the close button, chosen
/// so the traffic lights' side margin roughly matches their top margin.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_LEADING: f64 = 14.0;

/// Places the traffic lights on the tab strip's center line once the window
/// exists.
#[cfg(target_os = "macos")]
fn center_traffic_lights(window: &winit::window::Window) {
    sync_native_chrome(window);
}

/// Keeps the traffic lights vertically centered on the tab strip's row, the
/// way Brave and Chrome do, rather than at AppKit's default titlebar height.
///
/// AppKit lays the buttons out inside a titlebar container view and can reset
/// that layout on resize or full-screen changes, so this runs after every
/// window change and re-applies the geometry only when it drifted. In full
/// screen AppKit owns the revealed titlebar and it is left alone. Native
/// decorations remain AppKit-owned; winit keeps its own window delegate.
/// Returns true only when a native layout change requires a fresh frame.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn sync_native_chrome(window: &winit::window::Window) -> bool {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindowButton, NSWindowStyleMask};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

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
    ns_window.setTitlebarAppearsTransparent(true);
    // Native titlebar dragging must not steal address-field text-selection
    // drags. Empty chrome still uses Window::drag_window, as before.
    ns_window.setMovable(false);
    if ns_window
        .styleMask()
        .contains(NSWindowStyleMask::FullScreen)
    {
        return false;
    }
    let buttons: Vec<_> = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ]
    .into_iter()
    .filter_map(|kind| ns_window.standardWindowButton(kind))
    .collect();
    // The buttons live in a titlebar view inside AppKit's titlebar container.
    // SAFETY: `superview` is a plain main-thread view-hierarchy getter; the
    // returned views are retained and only used within this call.
    let Some(container) = buttons
        .first()
        .and_then(|button| unsafe { button.superview() })
        .and_then(|titlebar| unsafe { titlebar.superview() })
    else {
        return false;
    };
    // A container twice the row center keeps the buttons centered on it.
    let height = 2.0 * f64::from(browser_shell::TAB_ROW_CENTER);
    let mut changed = false;
    let mut frame = container.frame();
    let top = ns_window.frame().size.height - height;
    if frame.size.height != height || frame.origin.y != top {
        frame.size.height = height;
        frame.origin.y = top;
        container.setFrame(frame);
        changed = true;
    }
    // Shift the group so the close button's left margin matches the space
    // above it, keeping AppKit's spacing between the buttons.
    let shift = TRAFFIC_LIGHT_LEADING - buttons[0].frame().origin.x;
    for button in &buttons {
        let mut frame = button.frame();
        let y = ((height - frame.size.height) / 2.0).round();
        if frame.origin.y != y || shift != 0.0 {
            frame.origin.y = y;
            frame.origin.x += shift;
            button.setFrame(frame);
            changed = true;
        }
    }
    changed
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("browser-macos requires macOS.");
}
