use http::header::{HeaderName, HeaderValue};

use super::cors::is_cors_safelisted_header;

/// Guard that controls which header names and values may be added to a list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HeaderGuard {
    /// No Fetch restrictions; intended for internal or non-request header lists.
    None,
    /// A request header list with forbidden request headers blocked.
    #[default]
    Request,
    /// A no-CORS request header list restricted to CORS-safelisted headers.
    RequestNoCors,
    /// A response header list that blocks forbidden response headers.
    ///
    /// This includes the obsolete `Set-Cookie2` header. The header is retained
    /// only as a compatibility case for filtering; it is not a supported
    /// cookie-processing API and is never exposed through a public response.
    Response,
    /// A header list that cannot be changed.
    Immutable,
}

/// Checks whether a header guard may move to a stricter state.
pub(super) fn guard_transition_allowed(from: HeaderGuard, to: HeaderGuard) -> bool {
    matches!(
        (from, to),
        (HeaderGuard::None, _)
            | (
                HeaderGuard::Request,
                HeaderGuard::RequestNoCors | HeaderGuard::Immutable
            )
            | (HeaderGuard::RequestNoCors, HeaderGuard::Immutable)
            | (HeaderGuard::Response, HeaderGuard::Immutable)
    )
}
/// Applies the selected guard's request or response mutation restrictions.
pub(super) fn allowed_header(guard: HeaderGuard, name: &HeaderName, value: &HeaderValue) -> bool {
    match guard {
        HeaderGuard::None => true,
        HeaderGuard::Immutable => false,
        HeaderGuard::Request => !is_forbidden_request_header(name),
        HeaderGuard::Response => !matches!(name.as_str(), "set-cookie" | "set-cookie2"),
        HeaderGuard::RequestNoCors => {
            !is_forbidden_request_header(name) && is_cors_safelisted_header(name, value)
        }
    }
}

/// Reports whether a caller is forbidden from setting a request header.
fn is_forbidden_request_header(name: &HeaderName) -> bool {
    let name = name.as_str();
    matches!(
        name,
        "accept-charset"
            | "accept-encoding"
            | "access-control-request-headers"
            | "access-control-request-method"
            | "access-control-request-private-network"
            | "connection"
            | "content-length"
            | "cookie"
            | "cookie2"
            | "date"
            | "dnt"
            | "expect"
            | "host"
            | "keep-alive"
            | "origin"
            | "permissions-policy"
            | "proxy-connection"
            | "referer"
            | "set-cookie"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "via"
    ) || name.starts_with("proxy-")
        || name.starts_with("sec-")
}
