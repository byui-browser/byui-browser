//! Browser-owned capability boundaries for the process-local Fetch engine.

use crate::{Headers, Origin, Request, RequestError, RequestMode, ResponseType, Url};

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
    pub headers: Headers,
    /// Origin derived from the final response URL.
    pub response_origin: Option<Origin>,
    /// Origin of the client that initiated the request, when supplied.
    pub request_origin: Option<Origin>,
    /// Request mode used to select the exposed response type.
    pub request_mode: RequestMode,
    /// Visibility class selected by the engine before public exposure.
    pub response_type: ResponseType,
    /// Number of redirects followed before this response was approved.
    pub redirect_count: usize,
    /// Whether this response came from the process-local cache.
    pub from_cache: bool,
}

/// Browser-owned authorization and header generation for request phases.
pub trait RequestPolicyService: Send + Sync {
    /// Authorizes a request before cache lookup, cookies, or network I/O.
    ///
    /// The controller invokes this once for the initial request and again for
    /// each followed redirect. Use [`Request::active_url`] when making a
    /// per-hop decision; [`Request::url`] remains the original caller input.
    fn check_request(&self, request: &Request) -> Result<(), RequestError>;

    /// Returns browser-owned request headers for one validated transport hop.
    ///
    /// These values are merged into an internal transport-only channel before
    /// CORS-preflight calculation and after Fetch-owned headers are derived.
    /// The default is empty. Use [`Headers::generated_request_headers`] for
    /// forbidden browser-owned names such as `Sec-Fetch-*`; Fetch-owned
    /// `Origin`, referrer, body, cookie, and preflight fields are rejected.
    fn generated_request_headers(
        &self,
        _request: &Request,
        _target: &Url,
    ) -> Result<Headers, RequestError> {
        Ok(Headers::generated_request_headers())
    }
}

/// Browser-owned authorization and diagnostics for response phases.
pub trait ResponsePolicyService: Send + Sync {
    /// Authorizes complete internal response metadata before public exposure.
    fn check_response(&self, request: &Request, info: &ResponseInfo) -> Result<(), RequestError>;

    /// Observes approved response headers for diagnostics and timing clients.
    ///
    /// Body lifecycle and timing events are outside this first contract slice.
    fn response_headers(&self, _request: &Request, _info: &ResponseInfo) {}

    /// Observes headers from an approved validating `304 Not Modified` response.
    ///
    /// The engine invokes this after the retained representation has passed
    /// normal response authorization and before it is exposed. The supplied
    /// headers are owned by the callback and include internal fields such as
    /// `Set-Cookie`; implementations can apply browser-owned validation
    /// effects without making those fields part of future cache hits.
    fn validation_response_headers(&self, _request: &Request, _headers: Headers) {}
}

/// Browser-owned HTTP cookie selection and storage.
pub trait CookieService: Send + Sync {
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
}

/// Browser-owned service-worker interception decisions.
pub trait ServiceWorkerService: Send + Sync {
    /// Decides whether a service worker intercepts the initial request.
    ///
    /// An [`ServiceWorkerDecision::Intercept`] decision fails closed until
    /// worker response execution is implemented.
    fn service_worker(&self, request: &Request) -> Result<ServiceWorkerDecision, RequestError>;
}

/// Browser-owned HTTP and CORS-preflight cache isolation.
pub trait CachePartitionService: Send + Sync {
    /// Returns an owned process-local cache partition key for this request.
    ///
    /// The key is not durable storage identity and is not shared outside this
    /// controller. Return an empty key when no HTTP cache partition is
    /// available; the engine will use `no-store` behavior for that request.
    fn cache_partition(&self, request: &Request) -> Result<String, RequestError>;
}
