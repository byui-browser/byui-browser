use http::header::{HeaderMap, HeaderName, HeaderValue};

use crate::error::RequestError;

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
    Response,
    /// A header list that cannot be changed.
    Immutable,
}

/// An ordered HTTP header list with a Fetch mutation guard.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeaderList {
    pub(super) entries: Vec<(HeaderName, HeaderValue)>,
    guard: HeaderGuard,
}

impl HeaderList {
    /// Creates an empty request-guarded list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty list with the supplied Fetch guard.
    pub fn with_guard(guard: HeaderGuard) -> Self {
        Self {
            entries: Vec::new(),
            guard,
        }
    }

    /// Returns the current mutation guard.
    pub fn guard(&self) -> HeaderGuard {
        self.guard
    }

    /// Sets a stricter guard. Existing entries are filtered when switching to a request guard.
    pub(crate) fn set_guard(&mut self, guard: HeaderGuard) {
        self.guard = guard;
        self.entries
            .retain(|(name, value)| allowed_header(guard, name, value));
    }

    /// Returns whether the list contains no headers.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Appends a header while preserving duplicate entries and insertion order.
    pub fn append(&mut self, name: HeaderName, value: HeaderValue) -> Result<(), RequestError> {
        self.check(&name, &value)?;
        self.entries.push((name, value));
        Ok(())
    }

    /// Replaces all entries with `name` with one header value.
    pub fn insert(&mut self, name: HeaderName, value: HeaderValue) -> Result<(), RequestError> {
        self.check(&name, &value)?;
        self.entries.retain(|(existing, _)| existing != name);
        self.entries.push((name, value));
        Ok(())
    }

    /// Iterates over headers in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.entries.iter().map(|(name, value)| (name, value))
    }

    /// Returns the first value for a header name, if present.
    pub fn get(&self, name: &str) -> Option<&HeaderValue> {
        self.entries
            .iter()
            .find(|(entry, _)| entry.as_str().eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    }

    /// Returns whether a header with this name is present.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Copies transport headers into an immutable, filtered response list.
    pub(crate) fn exposed_response(headers: &HeaderMap, cors: bool) -> Self {
        let mut list = Self::with_guard(HeaderGuard::Response);
        for (name, value) in headers {
            if allowed_header(HeaderGuard::Response, name, value)
                && (!cors || is_cors_exposed(name, headers))
            {
                list.entries.push((name.clone(), value.clone()));
            }
        }
        list.guard = HeaderGuard::Immutable;
        list
    }

    /// Copies all transport headers for trusted provider decisions.
    pub(crate) fn internal_response(headers: &HeaderMap) -> Self {
        let mut list = Self::with_guard(HeaderGuard::Immutable);
        list.entries.extend(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        list
    }

    /// Converts the list to the representation expected by the HTTP transport.
    pub(crate) fn to_reqwest(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.entries {
            headers.append(name.clone(), value.clone());
        }
        headers
    }

    fn check(&self, name: &HeaderName, value: &HeaderValue) -> Result<(), RequestError> {
        if allowed_header(self.guard, name, value) {
            Ok(())
        } else {
            Err(RequestError::ForbiddenHeader(name.as_str().to_owned()))
        }
    }

    pub(crate) fn insert_internal_header(&mut self, name: HeaderName, value: HeaderValue) {
        self.entries.retain(|(existing, _)| existing != name);
        self.entries.push((name, value));
    }
}

fn is_cors_exposed(name: &HeaderName, headers: &HeaderMap) -> bool {
    if matches!(
        name.as_str(),
        "cache-control"
            | "content-language"
            | "content-length"
            | "content-type"
            | "expires"
            | "last-modified"
            | "pragma"
    ) {
        return true;
    }
    headers
        .get_all("access-control-expose-headers")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|exposed| exposed.trim().eq_ignore_ascii_case(name.as_str()))
}

fn allowed_header(guard: HeaderGuard, name: &HeaderName, value: &HeaderValue) -> bool {
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

pub(super) fn is_cors_safelisted_header(name: &HeaderName, value: &HeaderValue) -> bool {
    let raw = value.as_bytes();
    if raw.len() > 128
        || raw
            .iter()
            .any(|byte| matches!(*byte, 0x00..=0x08 | 0x0a..=0x1f | 0x7f))
    {
        return false;
    }
    match name.as_str() {
        "accept" | "accept-language" | "content-language" => true,
        "content-type" => value.to_str().is_ok_and(|value| {
            let media_type = value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            matches!(
                media_type.as_str(),
                "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
            )
        }),
        "range" => value.to_str().is_ok_and(|value| {
            value.strip_prefix("bytes=").is_some_and(|range| {
                let mut parts = range.split('-');
                let first = parts.next().unwrap_or("");
                let last = parts.next().unwrap_or("");
                !first.is_empty()
                    && first.bytes().all(|byte| byte.is_ascii_digit())
                    && (last.is_empty() || last.bytes().all(|byte| byte.is_ascii_digit()))
                    && parts.next().is_none()
            })
        }),
        _ => false,
    }
}
