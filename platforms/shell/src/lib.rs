//! Window, toolbar, and page rendering shared by the native shells.
//!
//! The platform crates (`browser-macos`, `browser-linux`) stay thin: each
//! describes how its window should look through a [`Platform`] and calls
//! [`run`]. Everything else, from the winit event loop to the software-painted
//! chrome and the welcome-page demo, lives here so the shells behave the same
//! on every desktop.
//!
//! On operating systems without a supported shell the crate only exposes the
//! pure drawing code, so workspace checks and tests still run there.

#![forbid(unsafe_code)]

mod content;
mod toolbar;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod app;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use app::{Platform, run};
pub use toolbar::load_ui_font;
