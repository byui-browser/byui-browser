//! Errors exposed by the networking client.

/// Errors that can occur while preparing, executing, or serving a request.
#[derive(Debug)]
pub enum RequestError {
    /// The request URL could not be parsed.
    InvalidUrl(String),
    /// The underlying HTTP client reported a transport error.
    Transport(reqwest::Error),
    /// `OnlyIfCached` was requested but no fresh cached response exists.
    CacheMiss,
    /// The URL uses a scheme unsupported by the HTTP transport.
    UnsupportedScheme(String),
    /// The serialized origin is malformed or is not an HTTP(S) origin.
    InvalidOrigin(String),
    /// The request method is not a valid HTTP token.
    InvalidMethod(String),
    /// Fetch forbids this request method.
    ForbiddenMethod(String),
    /// The method is outside the GET/HEAD/POST set allowed by no-CORS mode.
    NoCorsMethod(String),
    /// Fetch forbids this request header under the active header guard.
    ForbiddenHeader(String),
    /// A request body cannot be sent with this method.
    BodyNotAllowed(String),
    /// A one-shot body has already been consumed by an earlier send attempt.
    BodyAlreadyConsumed,
    /// The keepalive body exceeds the configured byte quota.
    KeepaliveBodyTooLarge { limit: u64 },
    /// The scheduler was shut down before the request could run.
    SchedulerClosed,
    /// The request was aborted before completion.
    Aborted,
}

impl std::fmt::Display for RequestError {
    /// Formats the error for logs and user-facing diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(url) => write!(f, "Invalid request URL: {url}"),
            Self::Transport(error) => write!(f, "Request failed: {error}"),
            Self::CacheMiss => f.write_str("Request is not available in the HTTP cache"),
            Self::UnsupportedScheme(scheme) => write!(f, "Unsupported URL scheme: {scheme}"),
            Self::InvalidOrigin(origin) => write!(f, "Invalid request origin: {origin}"),
            Self::InvalidMethod(method) => write!(f, "Invalid request method: {method}"),
            Self::ForbiddenMethod(method) => write!(f, "Forbidden request method: {method}"),
            Self::NoCorsMethod(method) => {
                write!(f, "Method is not allowed in no-CORS mode: {method}")
            }
            Self::ForbiddenHeader(name) => write!(f, "Forbidden request header: {name}"),
            Self::BodyNotAllowed(method) => write!(f, "A request body is not allowed for {method}"),
            Self::BodyAlreadyConsumed => f.write_str("Request body has already been consumed"),
            Self::KeepaliveBodyTooLarge { limit } => {
                write!(f, "Keepalive request body exceeds {limit} bytes")
            }
            Self::SchedulerClosed => f.write_str("Request scheduler is closed"),
            Self::Aborted => f.write_str("Request was aborted"),
        }
    }
}

/// Implements the standard error trait for `RequestError`.
impl std::error::Error for RequestError {}

impl From<reqwest::Error> for RequestError {
    /// Converts a reqwest error into the networking crate's error type.
    fn from(error: reqwest::Error) -> Self {
        Self::Transport(error)
    }
}
