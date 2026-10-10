use http::header::{HeaderMap, HeaderName, HeaderValue};

use crate::api::error::RequestError;

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

/// An ordered HTTP header collection with a Fetch mutation guard.
///
/// `Headers` is used for both request and response headers. The guard controls
/// which operations are available: request headers are caller-mutable,
/// response headers are filtered, and exposed response headers are immutable.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Headers {
    pub(super) entries: Vec<(HeaderName, HeaderValue)>,
    guard: HeaderGuard,
}

impl Headers {
    /// Creates an empty request header collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a collection from already parsed HTTP header pairs.
    pub fn try_from_iter<I>(iter: I) -> Result<Self, RequestError>
    where
        I: IntoIterator<Item = (HeaderName, HeaderValue)>,
    {
        let mut headers = Self::new();
        for (name, value) in iter {
            headers.append(name, value)?;
        }
        Ok(headers)
    }

    /// Creates an empty collection with an internal Fetch guard.
    pub(crate) fn with_guard(guard: HeaderGuard) -> Self {
        Self {
            entries: Vec::new(),
            guard,
        }
    }

    /// Returns the current mutation guard.
    pub fn guard(&self) -> HeaderGuard {
        self.guard
    }

    /// Changes the guard toward a stricter Fetch representation.
    pub(crate) fn set_guard(&mut self, guard: HeaderGuard) {
        if self.guard == HeaderGuard::Immutable {
            return;
        }
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
    pub fn set(&mut self, name: HeaderName, value: HeaderValue) -> Result<(), RequestError> {
        self.check(&name, &value)?;
        self.entries.retain(|(existing, _)| existing != name);
        self.entries.push((name, value));
        Ok(())
    }

    /// Replaces all values for a header name with one value.
    pub fn insert(&mut self, name: HeaderName, value: HeaderValue) -> Result<(), RequestError> {
        self.set(name, value)
    }

    /// Deletes all values associated with `name`.
    pub fn delete(&mut self, name: &str) -> Result<(), RequestError> {
        if self.guard == HeaderGuard::Immutable {
            return Err(RequestError::ImmutableHeaders);
        }
        let name = HeaderName::try_from(name)
            .map_err(|_| RequestError::InvalidHeaderName(name.to_owned()))?;
        self.entries.retain(|(existing, _)| existing != name);
        Ok(())
    }

    /// Iterates over headers in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.entries.iter().map(|(name, value)| (name, value))
    }

    /// Returns the combined value for a header name, if present.
    pub fn get(&self, name: &str) -> Option<String> {
        let values: Vec<&HeaderValue> = self
            .entries
            .iter()
            .filter(|(entry, _)| entry.as_str().eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
            .collect();
        if values.is_empty() {
            None
        } else if name.eq_ignore_ascii_case("set-cookie") {
            values
                .first()
                .map(|value| value.to_str().unwrap_or_default().to_owned())
        } else {
            Some(
                values
                    .iter()
                    .map(|value| value.to_str().unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        }
    }

    /// Returns each `Set-Cookie` value without combining cookie attributes.
    pub fn get_set_cookie(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(name, _)| name.as_str().eq_ignore_ascii_case("set-cookie"))
            .filter_map(|(_, value)| value.to_str().ok().map(str::to_owned))
            .collect()
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
        if self.guard == HeaderGuard::Immutable {
            return Err(RequestError::ImmutableHeaders);
        }
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

pub(crate) fn is_cors_safelisted_header(name: &HeaderName, value: &HeaderValue) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn value(value: &'static str) -> HeaderValue {
        HeaderValue::from_static(value)
    }

    #[test]
    fn headers_combine_duplicate_values_and_preserve_cookie_values() {
        let mut headers = Headers::with_guard(HeaderGuard::None);
        headers
            .append(HeaderName::from_static("x-test"), value("one"))
            .unwrap();
        headers
            .append(HeaderName::from_bytes(b"X-Test").unwrap(), value("two"))
            .unwrap();
        headers
            .append(HeaderName::from_static("set-cookie"), value("a=1"))
            .unwrap();
        headers
            .append(HeaderName::from_static("set-cookie"), value("b=2"))
            .unwrap();

        assert_eq!(headers.get("x-test").as_deref(), Some("one, two"));
        assert_eq!(headers.get_set_cookie(), ["a=1", "b=2"]);
        assert_eq!(
            headers
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["x-test", "x-test", "set-cookie", "set-cookie"]
        );
    }

    #[test]
    fn set_and_delete_have_fetch_mutation_semantics() {
        let mut headers = Headers::new();
        headers
            .append(HeaderName::from_static("x-test"), value("one"))
            .unwrap();
        headers
            .append(HeaderName::from_static("x-test"), value("two"))
            .unwrap();
        headers
            .set(HeaderName::from_static("x-test"), value("three"))
            .unwrap();
        assert_eq!(headers.get("x-test").as_deref(), Some("three"));
        headers.delete("X-TEST").unwrap();
        assert!(!headers.has("x-test"));
    }

    #[test]
    fn immutable_headers_reject_all_mutation() {
        let mut headers = Headers::with_guard(HeaderGuard::Immutable);
        assert!(matches!(
            headers.append(HeaderName::from_static("x-test"), value("one")),
            Err(RequestError::ImmutableHeaders)
        ));
        assert!(matches!(
            headers.delete("x-test"),
            Err(RequestError::ImmutableHeaders)
        ));
    }

    #[test]
    fn no_cors_guard_removes_existing_unsafelisted_headers() {
        let mut headers = Headers::new();
        headers
            .append(HeaderName::from_static("x-custom"), value("one"))
            .unwrap();
        headers
            .append(HeaderName::from_static("accept"), value("text/plain"))
            .unwrap();
        headers.set_guard(HeaderGuard::RequestNoCors);
        assert!(!headers.has("x-custom"));
        assert!(headers.has("accept"));
    }
}
