//! Browser-owned inputs to the process-local Fetch engine.

use crate::{HeaderList, Origin, Request, RequestError, RequestMode, ResponseType};

/// Decision made by the service-worker owner before network I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceWorkerDecision {
    /// No worker intercepts this request, so the network path may continue.
    Network,
    /// A worker would intercept this request; this engine slice cannot execute it.
    Intercept,
}

/// Immutable response metadata supplied to browser policy and diagnostics.
///
/// This trusted internal view includes headers such as `Set-Cookie` that are
/// removed before a response reaches ordinary Fetch callers.
#[derive(Clone, Debug)]
pub struct ResponseInfo {
    /// HTTP status code received from the transport.
    pub status: u16,
    /// Canonical HTTP reason phrase; the transport does not retain a custom phrase.
    pub status_text: String,
    /// Final URL for the one supported HTTP exchange.
    pub url: String,
    /// Complete ordered response headers for trusted policy processing.
    pub headers: HeaderList,
    /// Origin derived from the final response URL.
    pub response_origin: Option<Origin>,
    /// Origin of the client that initiated the request, when supplied.
    pub request_origin: Option<Origin>,
    /// Request mode used to select the exposed response type.
    pub request_mode: RequestMode,
    /// Visibility class selected by the engine before public exposure.
    pub response_type: ResponseType,
    /// Number of redirects followed; always zero in this single-exchange slice.
    pub redirect_count: usize,
    /// Whether this response came from the process-local cache.
    pub from_cache: bool,
}

/// Browser-owned services required when a request carries client context.
///
/// Implementations are expected to delegate to the storage, security,
/// service-worker, browser-context, and diagnostics owners. Methods return
/// decisions or typed errors; the engine does not own their durable state.
pub trait FetchServices: Send + Sync {
    /// Authorizes a request before cache lookup, cookies, or network I/O.
    fn check_request(&self, request: &Request) -> Result<(), RequestError>;

    /// Authorizes complete internal response metadata before public exposure.
    fn check_response(&self, request: &Request, info: &ResponseInfo) -> Result<(), RequestError>;

    /// Returns the cookie header selected by storage and security policy.
    ///
    /// The engine calls this only when the credentials mode permits credentials.
    fn cookie_header(&self, request: &Request) -> Result<Option<String>, RequestError>;

    /// Processes raw `Set-Cookie` values after response checks have passed.
    fn store_set_cookie(
        &self,
        request: &Request,
        values: &[http::HeaderValue],
    ) -> Result<(), RequestError>;

    /// Decides whether a service worker intercepts the request.
    ///
    /// An `Intercept` decision fails closed until worker response integration
    /// is implemented in a later plan step.
    fn service_worker(&self, request: &Request) -> Result<ServiceWorkerDecision, RequestError>;

    /// Returns an owned cache partition key for this browser request.
    ///
    /// The process-local cache uses the key only within this controller. It is
    /// not a durable HTTP cache or a substitute for the storage owner.
    fn cache_partition(&self, request: &Request) -> Result<String, RequestError>;

    /// Observes approved response headers for diagnostics and timing clients.
    ///
    /// Body lifecycle and timing events are outside this first contract slice.
    fn response_headers(&self, _request: &Request, _info: &ResponseInfo) {}
}
