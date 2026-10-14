//! Fetch pipeline orchestration.

mod cache_capture;
mod controller;
mod execute;
mod http;
mod prepare;
mod process_response;
mod schemes;

pub use controller::RequestController;
