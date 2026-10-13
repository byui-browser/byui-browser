//! Minimal Linux shell for X11 and Wayland. Run with `cargo run -p browser-linux`.
//!
//! The window, toolbar, and page rendering come from the shared
//! `browser-shell` crate; this crate only describes the Linux window and
//! finds a system font for the toolbar text.

#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
fn main() -> Result<(), winit::error::EventLoopError> {
    browser_shell::run(browser_shell::Platform {
        // Hide the desktop's decorations so the toolbar can span the full
        // window width; the shell draws the window controls itself. Without
        // decorations there is no native resize border either, so the shell
        // adds its own resize handles.
        configure_window: |attributes| attributes.with_decorations(false),
        ui_font_files: linux::ui_font_files,
        resize_borders: true,
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
        let mut files: Vec<PathBuf> = fontconfig_sans_serif().into_iter().collect();
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

    /// Asks fontconfig for the file backing the user's default sans-serif
    /// font. Returns `None` when `fc-match` is unavailable or fails, so the
    /// fallback list above still applies.
    fn fontconfig_sans_serif() -> Option<PathBuf> {
        let output = Command::new("fc-match")
            .args(["--format=%{file}", "sans-serif"])
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
