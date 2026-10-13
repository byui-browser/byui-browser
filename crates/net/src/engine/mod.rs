//! Fetch pipeline orchestration.

mod cache_capture;
mod controller;
mod execute;
mod prepare;
mod process_response;
mod providers;

pub use controller::RequestController;
