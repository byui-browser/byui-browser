//! Errors exposed by the networking controller.

/// Errors that can occur while preparing, executing, or serving a request.
#[derive(Debug)]
pub enum RequestError {
    /// The request URL could not be parsed.
    InvalidUrl(String),
    /// The underlying HTTP client reported a transport error.
    Transport(reqwest::Error),
    /// `OnlyIfCached` was requested but no fresh cached response exists.
    CacheMiss,
}

impl std::fmt::Display for RequestError {
    /// Formats the error for logs and user-facing diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(url) => write!(f, "invalid request URL: {url}"),
            Self::Transport(error) => write!(f, "request failed: {error}"),
            Self::CacheMiss => f.write_str("request is not available in the HTTP cache"),
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
