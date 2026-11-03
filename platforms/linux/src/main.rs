//! Minimal Linux shell for X11 and Wayland. Run with `cargo run -p browser-linux`.
//!
//! The window, toolbar, and page rendering come from the shared
//! `browser-shell` crate; this crate only describes the Linux window, which
//! keeps the desktop's own decorations, and finds system fonts for the
//! toolbar and page text.

#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
fn main() -> Result<(), winit::error::EventLoopError> {
    browser_shell::run(browser_shell::Platform {
        // Keep the desktop's own decorations: the window manager (or, on
        // Wayland compositors without server-side decorations, winit's
        // Adwaita-style client-side frame) provides the titlebar, its
        // buttons, and the resize borders above and around the toolbar.
        configure_window: |attributes| attributes,
        ui_font_files: linux::ui_font_files,
        page_font_files: linux::page_font_files,
        window_controls_inset: 0.0,
    })
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::Command;

    /// Font files to try for toolbar text, best first: the desktop's
    /// configured sans-serif font from fontconfig, then common distribution
    /// fonts at their usual locations.
    pub fn ui_font_files() -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fontconfig_match("sans-serif").into_iter().collect();
        files.extend(
            [
                // Fedora and other Noto-based desktops.
                "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                // Debian and Ubuntu.
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                // Fedora and Arch.
                "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
                "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
            ]
            .into_iter()
            .map(PathBuf::from),
        );
        files
    }

    /// Font files to try for page text, best first: the desktop's
    /// configured serif font from fontconfig, matching the serif default of
    /// other browsers, then common distribution serif fonts, then the
    /// toolbar's sans-serif candidates as a last resort.
    pub fn page_font_files() -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fontconfig_match("serif").into_iter().collect();
        files.extend(
            [
                // Metric-compatible with Times New Roman (Debian, Ubuntu,
                // Fedora, Arch).
                "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
                "/usr/share/fonts/liberation-serif/LiberationSerif-Regular.ttf",
                "/usr/share/fonts/liberation/LiberationSerif-Regular.ttf",
                // Noto-based desktops.
                "/usr/share/fonts/google-noto/NotoSerif-Regular.ttf",
                "/usr/share/fonts/noto/NotoSerif-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSerif-Regular.ttf",
                // DejaVu (Debian, Ubuntu, Fedora, Arch).
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
                "/usr/share/fonts/dejavu-serif-fonts/DejaVuSerif.ttf",
                "/usr/share/fonts/TTF/DejaVuSerif.ttf",
            ]
            .into_iter()
            .map(PathBuf::from),
        );
        files.extend(ui_font_files());
        files
    }

    /// Asks fontconfig for the file backing the user's default font for
    /// `pattern`, such as `serif` or `sans-serif`. Returns `None` when
    /// `fc-match` is unavailable or fails, so the fallback lists still apply.
    fn fontconfig_match(pattern: &str) -> Option<PathBuf> {
        let output = Command::new("fc-match")
            .args(["--format=%{file}", pattern])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let path = String::from_utf8(output.stdout).ok()?;
        let path = path.trim();
        (!path.is_empty()).then(|| PathBuf::from(path))
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("browser-linux requires Linux.");
}
