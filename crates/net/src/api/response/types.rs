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
    /// This variant is returned by manual redirect handling.
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

impl Response {
    /// Constructs the filtered null-body response used to represent a Fetch network error.
    ///
    /// Controller fetch failures continue to return [`crate::RequestError`];
    /// this value is available to bindings that require a response-shaped error.
    pub fn network_error() -> Self {
        Self {
            response_type: ResponseType::Error,
            status: 0,
            status_text: String::new(),
            headers: Headers::with_guard(crate::HeaderGuard::Immutable),
            url: String::new(),
            redirected: false,
            body: Body::null(),
            from_cache: false,
        }
    }

    /// Clones response metadata and tees an unused body into bounded branches.
    ///
    /// Cloning fails after body consumption has started. Live streams use a
    /// one-chunk-per-branch buffer so a slow clone applies backpressure.
    pub fn try_clone(&mut self) -> Result<Self, crate::RequestError> {
        let cloned = self.body.tee()?;
        Ok(Self {
            response_type: self.response_type,
            status: self.status,
            status_text: self.status_text.clone(),
            headers: self.headers.clone(),
            url: self.url.clone(),
            redirected: self.redirected,
            body: cloned,
            from_cache: self.from_cache,
        })
    }

    /// Returns whether the exposed status is in the successful 200-299 range.
    pub fn ok(&self) -> bool {
        (200..=299).contains(&self.status)
    }

    /// Returns whether this response has a readable non-null body.
    pub fn has_body(&self) -> bool {
        !self.body.is_null()
    }
}
