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
//!
//! The current transport executes HTTP(S) requests. Relative URLs require a
//! [`FetchEnvironment`] with a base URL; `about:`, `blob:`, `data:`, and `file:`
//! fetching are unsupported and return [`RequestError::UnsupportedScheme`].
//! Request construction validates methods and caller headers and models
//! replayable or one-shot request bodies, but the crate does not yet implement
//! every Fetch algorithm (notably the redirect, CORS, and body-extraction
//! algorithms described in the project gap audit).

// Crate-wide flags to ignore dead code warnings. Will be removed once implementation
// is finished, but is required for now to prevent the integration tests from failing.
#![allow(dead_code)]
// Flag to forbid unsafe code. This is security-critical and permanant.
#![forbid(unsafe_code)]

// TODO(net): Add persistent cookie storage, strict CORS, and HTTP/3 transport.

mod cache;
mod cancellation;
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

pub use cancellation::{AbortController, AbortSignal};
pub use config::Config;
pub use controller::RequestController;
pub use error::RequestError;
pub use request::{
    CacheMode, CredentialsMode, FetchContext, FetchEnvironment, HeaderGuard, HeaderList,
    InitiatorType, NetworkPartitionKey, Origin, RedirectMode, Referrer, ReferrerPolicy, Request,
    RequestBody, RequestDestination, RequestMode, RequestPriority, ServiceWorkersMode,
};
pub use response::{Response, ResponseBody, StreamingResponse};
