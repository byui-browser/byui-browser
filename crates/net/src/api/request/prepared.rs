use url::Url;

use super::Request;
use crate::api::Headers;

/// Controller-owned request state after initial policy validation.
///
/// Construction consumes the caller-facing [`Request`], freezes its initial
/// URL and origin state, and prevents unprepared requests from entering scheme
/// dispatch.
#[derive(Debug)]
pub(crate) struct PreparedRequest {
    /// Request configuration plus controller-owned execution state.
    pub(crate) request: Request,
    /// Parsed initial URL selected during validation.
    pub(crate) url: Url,
}

impl PreparedRequest {
    /// Creates the execution value for a validated initial URL.
    pub(crate) fn new(mut request: Request, url: Url) -> Self {
        request.prepare_url(url.clone());
        Self { request, url }
    }

    /// Splits the prepared value for a scheme-specific execution algorithm.
    pub(crate) fn into_parts(self) -> (Request, Url) {
        (self.request, self.url)
    }
}

/// A single HTTP transport attempt after per-hop policy has completed.
#[derive(Debug)]
pub(crate) struct TransportRequest {
    /// Request metadata and body ownership for this attempt.
    pub(crate) request: Request,
    /// Internal headers after Fetch-generated values are applied.
    pub(crate) headers: Headers,
    /// Parsed current-hop URL used by the HTTP transport.
    pub(crate) url: Url,
}
