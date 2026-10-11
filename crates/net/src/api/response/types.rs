use crate::api::{Headers, body::Body};

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

/// A filtered Fetch response whose body may remain live until consumed or dropped.
#[derive(Debug)]
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
    /// Filtered, abort-aware response body.
    ///
    /// A network body may remain live until callers consume or drop it.
    /// Filtered and null bodies are explicit null bodies.
    pub body: Body,
    /// Whether this readable response came from the process-local cache.
    /// Filtered opaque responses always report `false` to avoid metadata leaks.
    pub from_cache: bool,
}
