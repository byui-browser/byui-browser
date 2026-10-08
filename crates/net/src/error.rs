//! Errors exposed by the networking client.

/// Errors that can occur while preparing, executing, or serving a request.
#[derive(Debug)]
pub enum RequestError {
    /// The request URL could not be parsed.
    InvalidUrl(String),
    /// The underlying HTTP client reported a transport error.
    Transport(reqwest::Error),
    /// The request timed out before it could complete.
    Timeout(reqwest::Error),

    /// An error occurred while reading the response body.
    Body(reqwest::Error),

    /// An error occurred while decoding the response.
    Decode(reqwest::Error),

    CacheMiss,
    /// The URL uses a scheme unsupported by the HTTP transport.
    UnsupportedScheme(String),
    /// The scheduler was shut down before the request could run.
    SchedulerClosed,
}

impl std::fmt::Display for RequestError {
    /// Formats the error for logs and user-facing diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(url) => write!(f, "Invalid request URL: {url}"),
            Self::Transport(error) => write!(f, "Request failed: {error}"),
            Self::Timeout(error) => write!(f, "Request timed out: {error}"),
            Self::Body(error) => write!(f, "Failed to read response body: {error}"),
            Self::Decode(error) => write!(f, "Response decode error: {error}"),
            Self::CacheMiss => f.write_str("Request is not available in the HTTP cache"),
            Self::UnsupportedScheme(scheme) => write!(f, "Unsupported URL scheme: {scheme}"),
            Self::SchedulerClosed => f.write_str("Request scheduler is closed"),
        }
    }
}

/// Implements the standard error trait for `RequestError`.
impl std::error::Error for RequestError {}

impl From<reqwest::Error> for RequestError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout(error)
        } else if error.is_body() {
            Self::Body(error)
        } else if error.is_decode() {
            Self::Decode(error)
        } else {
            Self::Transport(error)
        }
    }
}
