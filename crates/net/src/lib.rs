//! Rust Fetch engine boundary for browser and other Rust consumers.
//!
//! **Owning team**: Networking Team
//!
//! [`RequestController::fetch`] returns a filtered [`Response`].
//! [`RequestController::fetch_stream`] returns the same filtered metadata with
//! an abort-aware streaming body whose network errors can arrive after headers.
//! The transport and its Reqwest header/status/stream types stay behind this
//! response boundary. The `webapis` crate adapts these Rust values to
//! JavaScript objects, promises, and streams; this crate owns no JS identity.
//!
//! The engine coordinates process-local scheduling and a small in-memory cache.
//! Durable storage, security policy, service workers, client context, and
//! diagnostics belong to their browser services and enter through
//! [`FetchServices`]. Renderer processes do not open sockets directly.
//!
//! The current transport executes HTTP(S) requests. Relative URLs require a
//! [`FetchEnvironment`] with a base URL; `about:`, `blob:`, `data:`, and `file:`
//! fetching are unsupported and return [`RequestError::UnsupportedScheme`].
//! This first engine slice supports basic same-origin responses, simple CORS
//! checks, opaque no-CORS views, and forbidden response-header filtering.
//! Requests requiring absent providers or algorithms fail with a typed
//! [`RequestError::UnsupportedFeature`] before transport. HTTP redirects fail
//! with [`RequestError::RedirectFailure`] before public response exposure. See the
//! [engine contract](../docs/fetch-engine-contract.md) for the supported subset.

// Crate-wide flags to ignore dead code warnings. Will be removed once implementation
// is finished, but is required for now to prevent the integration tests from failing.
#![allow(dead_code)]
// Flag to forbid unsafe code. This is security-critical and permanant.
#![forbid(unsafe_code)]

// TODO(net): Add persistent cookie storage, strict CORS, and HTTP/3 transport.

mod api;
mod cache;
mod engine;
mod policy;
mod scheduling;
mod transport;

#[cfg(test)]
mod tests;

pub use api::{
    cancellation::{AbortController, AbortSignal},
    config::Config,
    error::RequestError,
    request::{
        CacheMode, CredentialsMode, FetchContext, FetchEnvironment, HeaderGuard, HeaderList,
        InitiatorType, NetworkPartitionKey, Origin, RedirectMode, Referrer, ReferrerPolicy,
        Request, RequestBody, RequestDestination, RequestMode, RequestPriority, ServiceWorkersMode,
    },
    response::{Response, ResponseBody, ResponseType, StreamingResponse},
    services::{FetchServices, ResponseInfo, ServiceWorkerDecision},
};
pub use engine::RequestController;
pub use http::{HeaderName, HeaderValue, Method};
pub use url::Url;
