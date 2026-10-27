//! Window, toolbar, and page rendering shared by the native shells.
//!
//! The platform crates (`browser-macos`, `browser-linux`,
//! `byui-browser-windows`) stay thin: each
//! describes how its window should look through `Platform` and calls `run`.
//! Everything else, from the winit event loop to the software-painted
//! chrome and the welcome-page demo, lives here so the shells behave the same
//! on every desktop.
//!
//! On operating systems without a supported shell the crate only exposes the
//! pure drawing code, so workspace checks and tests still run there.

#![forbid(unsafe_code)]

// The drawing code is only used by `app`, so on unsupported operating systems
// it is compiled for tests alone; building it there would otherwise fail
// `-D warnings` with dead-code errors.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows", test))]
mod content;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows", test))]
mod context_menu;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows", test))]
mod page_source;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows", test))]
mod toolbar;

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod app;

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use app::{Platform, run};
