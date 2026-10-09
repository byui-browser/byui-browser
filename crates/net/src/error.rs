//! Errors exposed by the networking client.

/// Errors that can occur while preparing, executing, or consuming a fetch.
///
/// URL, method, header, capability, keepalive, and scheduler errors occur
/// before public response headers and do not consume a response body. Correct
/// the request before retrying; a one-shot request body can still be unusable
/// if an earlier attempt consumed it. `Transport` and `Aborted` can also occur
/// while a response body is being streamed. In that case the stream is spent.
/// `CorsDenied`, `SameOriginViolation`, and `RedirectFailure` occur after
/// internal headers but before public exposure; the internal body is dropped.
/// A retry always requires a new request and a replayable request body.
///
/// | Variants | Recovery and body effect |
/// | --- | --- |
/// | `Initialization` | Fix client configuration; no request or response body exists. |
/// | `InvalidUrl`, `InvalidOrigin`, `InvalidMethod`, `ForbiddenMethod`, `NoCorsMethod`, `ForbiddenHeader`, `BodyNotAllowed`, `KeepaliveBodyTooLarge`, `UnsupportedScheme`, `UnsupportedFeature` | Correct the request; no body is consumed by this failed attempt. |
/// | `SchedulerClosed` | Use a live controller; the request body was not sent. |
/// | `NetworkError` | A cache-only miss; retry after cache state changes or choose another cache mode. No body was consumed. |
/// | `BodyAlreadyConsumed` | The request body was spent by a prior attempt; create a fresh body. |
/// | `CorsDenied`, `SameOriginViolation`, `RedirectFailure` | Internal headers arrived; the internal response body is discarded. The request body may have been sent. |
/// | `Transport`, `Aborted` | May occur before public headers or during streaming. The request body may have been sent; a delivered response stream is spent. |
#[derive(Debug)]
pub enum RequestError {
    /// A network failure before headers; retrying requires a new request.
    NetworkError,
    /// The request URL could not be parsed.
    InvalidUrl(String),
    /// The HTTP transport failed. Its message is diagnostic, not a stable API.
    Transport(String),
    /// HTTP client construction failed before any request or body was used.
    Initialization(String),
    /// CORS checks denied access before any response was exposed.
    CorsDenied,
    /// Same-origin mode rejected a cross-origin result before exposure.
    SameOriginViolation,
    /// A redirect could not be processed under the selected redirect mode.
    RedirectFailure,
    /// A request selected a capability without an installed provider.
    UnsupportedFeature(&'static str),
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
    /// A one-shot request body was consumed by an earlier send attempt.
    /// This failure occurs before headers; the original body cannot be retried.
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
            Self::NetworkError => f.write_str("Fetch network error"),
            Self::InvalidUrl(url) => write!(f, "Invalid request URL: {url}"),
            Self::Transport(error) => write!(f, "Request failed: {error}"),
            Self::Initialization(error) => write!(f, "HTTP client initialization failed: {error}"),
            Self::CorsDenied => f.write_str("CORS check denied the response"),
            Self::SameOriginViolation => f.write_str("Same-origin request crossed origins"),
            Self::RedirectFailure => f.write_str("Redirect could not be processed"),
            Self::UnsupportedFeature(feature) => write!(f, "Unsupported Fetch feature: {feature}"),
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
