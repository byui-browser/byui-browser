//! Request types and cache-policy controls.

use reqwest::{Method, header::HeaderMap};

/// An HTTP request submitted to the networking controller.
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
        }
    }
}

/// Controls how a request interacts with the controller's local cache.
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
