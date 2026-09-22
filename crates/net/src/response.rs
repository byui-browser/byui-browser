//! Response type returned by the networking controller.

use reqwest::{StatusCode, header::HeaderMap};

/// A fully buffered HTTP response returned by the networking controller.
#[derive(Clone, Debug)]
pub struct Response {
    /// HTTP status code returned by the server.
    pub status: StatusCode,
    /// Response headers.
    pub headers: HeaderMap,
    /// Final URL after redirects.
    pub url: String,
    /// Raw response body bytes.
    pub body: Vec<u8>,
    /// Whether this response was served from the local cache.
    pub from_cache: bool,
}
