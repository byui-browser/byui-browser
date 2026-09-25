//! Request types and cache-policy controls.

use reqwest::{Method, header::HeaderMap};

/// Controls the browser context in which a request was initiated.
#[derive(Clone, Debug, Default)]
pub struct FetchContext {
    /// Origin of the document or worker that initiated the request.
    pub origin: Option<String>,
    /// Cross-origin mode used when making the request.
    pub mode: RequestMode,
    /// Whether credentials such as cookies may be included.
    pub credentials: CredentialsMode,
}

/// Controls how a request may cross origins.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequestMode {
    #[default]
    /// Apply CORS processing to the request and response.
    Cors,
    /// Permit the request only when it stays within the initiating origin.
    SameOrigin,
    /// Use the restricted no-CORS fetch behavior.
    NoCors,
}

/// Controls whether credentials may be sent with a request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CredentialsMode {
    /// Never send credentials.
    Omit,
    #[default]
    /// Send credentials only for same-origin requests.
    SameOrigin,
    /// Permit credentials for cross-origin requests when policy allows them.
    Include,
}

/// An HTTP request submitted to the networking client.
#[derive(Clone, Debug)]
pub struct Request {
    /// HTTP method, such as `GET`, `POST`, or `PUT`.
    pub method: Method,
    /// Absolute URL to request.
    pub url: String,
    /// Headers supplied by the caller.
    pub headers: HeaderMap,
    /// Optional raw request body. Bodies are bytes so binary uploads are supported.
    pub body: Option<Vec<u8>>,
    /// Controls whether the local HTTP cache may be read or written.
    pub cache_mode: CacheMode,
    /// Browser context used by security, cookie, and CORS policy modules.
    pub context: FetchContext,
}

impl Request {
    /// Creates a GET request with empty headers, no body, and default caching.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::GET,
            url: url.into(),
            headers: HeaderMap::new(),
            body: None,
            cache_mode: CacheMode::Default,
            context: FetchContext::default(),
        }
    }

    pub(crate) fn is_cacheable_method(&self) -> bool {
        matches!(self.method, Method::GET | Method::HEAD)
    }
}

/// Controls how a request interacts with the client's local cache.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CacheMode {
    /// Use a fresh cached response when available and cache cacheable responses.
    #[default]
    Default,
    /// Skip an existing cached response, but permit the new response to be cached.
    Reload,
    /// Skip the cache and do not store the response.
    NoStore,
    /// Return a fresh cached response or fail without making a network request.
    OnlyIfCached,
}
