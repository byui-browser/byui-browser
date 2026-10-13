//! Fetch pipeline orchestration.

mod cache_capture;
mod controller;
mod execute;
mod prepare;
mod process_response;

pub use controller::RequestController;
