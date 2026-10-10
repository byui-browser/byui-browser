use super::ResponseBody;
use crate::api::Headers;

/// The visibility class selected by Fetch before exposing a response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseType {
    /// A same-origin or navigation response with readable metadata and body.
    Basic,
    /// A CORS-approved response with only CORS-exposed headers.
    Cors,
    /// A no-CORS response with hidden status, URL, headers, and body.
    Opaque,
    /// A manually handled redirect with hidden metadata and body.
    /// This variant is reserved until controller-owned redirects are implemented.
    OpaqueRedirect,
    /// A failed fetch with hidden metadata and body.
    /// This variant is reserved; current fetch failures return `RequestError`.
    Error,
}

/// A fully buffered, already filtered Fetch response.
#[derive(Clone, Debug)]
pub struct Response {
    /// Fetch response visibility class.
    pub response_type: ResponseType,
    /// Exposed HTTP status code, or zero for filtered responses.
    pub status: u16,
    /// Canonical HTTP reason phrase, empty for filtered responses.
    /// A custom wire reason phrase is not retained by the current transport.
    pub status_text: String,
    /// Immutable exposed response headers, excluding cookie headers.
    pub headers: Headers,
    /// Exposed final URL, empty for filtered responses.
    pub url: String,
    /// Whether the request followed at least one redirect.
    pub redirected: bool,
    /// Exposed body bytes. Filtered and null bodies are empty.
    pub body: Vec<u8>,
    /// Whether this readable response came from the process-local cache.
    /// Filtered opaque responses always report `false` to avoid metadata leaks.
    pub from_cache: bool,
}

/// Response metadata and a lazily consumed HTTP response body.
///
/// Headers are available as soon as the server response is received. The body
/// remains attached to the network transport and is consumed by polling
/// [`StreamingResponse::body`].
pub struct StreamingResponse {
    /// Fetch response visibility class.
    pub response_type: ResponseType,
    /// Exposed HTTP status code, or zero for filtered responses.
    pub status: u16,
    /// Canonical HTTP reason phrase, empty for filtered responses.
    /// A custom wire reason phrase is not retained by the current transport.
    pub status_text: String,
    /// Immutable exposed response headers, excluding cookie headers.
    pub headers: Headers,
    /// Exposed final URL, empty for filtered responses.
    pub url: String,
    /// Whether the request followed at least one redirect.
    pub redirected: bool,
    /// Filtered, abort-aware body chunks. A dropped body releases its permit.
    pub body: ResponseBody,
    /// Whether this readable response came from the process-local cache.
    /// Filtered opaque responses always report `false` to avoid metadata leaks.
    pub from_cache: bool,
}

impl std::fmt::Debug for StreamingResponse {
    /// Formats response metadata without consuming the body.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StreamingResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("url", &self.url)
            .field("body", &self.body)
            .field("from_cache", &self.from_cache)
            .finish_non_exhaustive()
    }
}
