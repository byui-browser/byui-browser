//! Networking stack: HTTP/1.1, HTTP/2, HTTP/3, TLS, caching, cookies.
//!
//! **Owning team**: Networking Team
//!
//! Planned high-level API (architecture §3.2): `fetch(request) -> Response`.
//! Renderer processes never open raw sockets; all network goes through
//! the Network process. Cookie jar / cache policy decisions are owned by
//! Security & Storage.

#![forbid(unsafe_code)]

// TODO(net): Implement fetch API, protocols, TLS, and connection pooling.

mod cache;
mod config;
mod controller;
mod error;
mod request;
mod response;

#[cfg(test)]
mod tests;

pub(crate) use config::Config;
pub(crate) use controller::RequestController;
pub(crate) use error::RequestError;
pub(crate) use request::{CacheMode, Request};
pub(crate) use response::Response;