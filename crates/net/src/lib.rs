//! Browser networking boundary for HTTP requests and responses.
//!
//! **Owning team**: Networking Team
//!
//! The public entry point is [`RequestController`], which coordinates request
//! validation, cache access, cookie handling, CORS checks, scheduling, and the
//! underlying HTTP transport.
//! Renderer processes never open raw sockets; all network goes through
//! the Network process. Cookie jar / cache policy decisions are owned by
//! Security & Storage.

// Crate-wide flags to ignore dead code and unused imports warnings. Will be
// removed once implementation is finished, but is required for now to prevent
// the integration tests from failing.
#![allow(dead_code)]
#![allow(unused_imports)]
// Flag to forbid unsafe code. This is security-critical and permanant.
#![forbid(unsafe_code)]

// TODO(net): Add persistent cookie storage, strict CORS, and HTTP/3 transport.

mod cache;
mod config;
mod controller;
mod cookies;
mod cors;
mod error;
mod policy;
mod request;
mod response;
mod scheduler;
mod transport;

#[cfg(test)]
mod tests;

pub use config::Config;
pub use controller::RequestController;
pub use error::RequestError;
pub use request::{CacheMode, CredentialsMode, FetchContext, Request, RequestMode};
pub use response::Response;
