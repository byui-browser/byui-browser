use url::Url;

use super::Request;

/// A request after Fetch policy validation has produced a parsed URL.
#[derive(Clone, Debug)]
pub(crate) struct PreparedRequest {
    /// Request metadata and caller-supplied state.
    pub(crate) request: Request,
    /// Parsed current URL used by the HTTP transport.
    pub(crate) url: Url,
}
