use http::header::{HeaderMap, HeaderName, HeaderValue};

use crate::api::{error::RequestError, request::CredentialsMode};

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

    /// Creates request headers from string name/value pairs.
    ///
    /// Names are validated and normalized to ASCII lowercase. Values are
    /// validated as HTTP field values and have leading and trailing HTTP
    /// whitespace removed before insertion.
    pub fn try_from_strings<I, N, V>(iter: I) -> Result<Self, RequestError>
    where
        I: IntoIterator<Item = (N, V)>,
        N: AsRef<str>,
        V: AsRef<str>,
    {
        let mut headers = Self::new();
        for (name, value) in iter {
            headers.append_str(name.as_ref(), value.as_ref())?;
        }
        Ok(headers)
    }

    /// Creates request headers from an `http` header map.
    ///
    /// Duplicate values are copied in map iteration order and remain subject
    /// to the request guard. This API does not expose a Reqwest-specific type.
    pub fn try_from_header_map(headers: &HeaderMap) -> Result<Self, RequestError> {
        Self::try_from_iter(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        )
    }

    /// Clones another header collection, including its guard and raw order.
    pub fn from_headers(headers: &Self) -> Self {
        headers.clone()
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
    pub(crate) fn set_guard(&mut self, guard: HeaderGuard) -> Result<(), RequestError> {
        if self.guard == guard {
            return Ok(());
        }
        if !guard_transition_allowed(self.guard, guard) {
            return Err(RequestError::InvalidHeaderGuardTransition {
                from: self.guard,
                to: guard,
            });
        }
        self.guard = guard;
        if guard != HeaderGuard::Immutable {
            self.entries
                .retain(|(name, value)| allowed_header(guard, name, value));
        }
        Ok(())
    }

    /// Returns whether the list contains no headers.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Appends a header while preserving duplicate entries and insertion order.
    pub fn append(&mut self, name: HeaderName, value: HeaderValue) -> Result<(), RequestError> {
        let value = normalize_value(value);
        self.check(&name, &value)?;
        self.entries.push((name, value));
        Ok(())
    }

    /// Validates and appends a string header name and value.
    pub fn append_str(&mut self, name: &str, value: &str) -> Result<(), RequestError> {
        let parsed_name = parse_name(name)?;
        let parsed_value = parse_value(parsed_name.as_str(), value)?;
        self.append(parsed_name, parsed_value)
    }

    /// Replaces all entries with `name` with one header value.
    pub fn set(&mut self, name: HeaderName, value: HeaderValue) -> Result<(), RequestError> {
        let value = normalize_value(value);
        self.check(&name, &value)?;
        self.entries.retain(|(existing, _)| existing != name);
        self.entries.push((name, value));
        Ok(())
    }

    /// Validates and replaces a string header name and value.
    pub fn set_str(&mut self, name: &str, value: &str) -> Result<(), RequestError> {
        let parsed_name = parse_name(name)?;
        let parsed_value = parse_value(parsed_name.as_str(), value)?;
        self.set(parsed_name, parsed_value)
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

    /// Iterates over exposed header fields in first-insertion order.
    ///
    /// Duplicate ordinary fields are combined with `", "`. `Set-Cookie`
    /// values remain separate so cookie attributes are never comma-joined.
    pub fn iter(&self) -> impl Iterator<Item = (HeaderName, String)> + '_ {
        self.combined_entries().into_iter()
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
            values.first().map(|value| header_value_string(value))
        } else {
            Some(
                values
                    .iter()
                    .map(|value| header_value_string(value))
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
            .map(|(_, value)| header_value_string(value))
            .collect()
    }

    /// Returns whether a header with this name is present.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Iterates over the raw ordered representation used by Fetch internals.
    pub(crate) fn iter_raw(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.entries.iter().map(|(name, value)| (name, value))
    }

    /// Copies transport headers into an immutable, filtered response list.
    pub(crate) fn exposed_response(
        headers: &HeaderMap,
        cors: bool,
        credentials_mode: CredentialsMode,
    ) -> Self {
        let mut list = Self::with_guard(HeaderGuard::Response);
        let cors_exposure = cors.then(|| CorsExposure::parse(headers, credentials_mode));
        for (name, value) in headers {
            if allowed_header(HeaderGuard::Response, name, value)
                && cors_exposure
                    .as_ref()
                    .is_none_or(|exposure| exposure.includes(name))
            {
                list.entries.push((name.clone(), value.clone()));
            }
        }
        list.set_guard(HeaderGuard::Immutable)
            .expect("response headers may become immutable");
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

    pub(crate) fn insert_internal_header(
        &mut self,
        name: HeaderName,
        value: HeaderValue,
    ) -> Result<(), RequestError> {
        if self.guard == HeaderGuard::Immutable {
            return Err(RequestError::ImmutableHeaders);
        }
        let value = normalize_value(value);
        self.entries.retain(|(existing, _)| existing != name);
        self.entries.push((name, value));
        Ok(())
    }

    fn combined_entries(&self) -> Vec<(HeaderName, String)> {
        let mut combined = Vec::new();
        for (index, (name, value)) in self.entries.iter().enumerate() {
            if is_non_combinable(name) {
                combined.push((name.clone(), header_value_string(value)));
                continue;
            }
            if self.entries[..index]
                .iter()
                .any(|(existing, _)| existing == name)
            {
                continue;
            }
            let value = self
                .entries
                .iter()
                .filter(|(existing, _)| existing == name)
                .map(|(_, value)| header_value_string(value))
                .collect::<Vec<_>>()
                .join(", ");
            combined.push((name.clone(), value));
        }
        combined
    }
}

fn parse_name(name: &str) -> Result<HeaderName, RequestError> {
    HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| RequestError::InvalidHeaderName(name.to_owned()))
}

fn parse_value(name: &str, value: &str) -> Result<HeaderValue, RequestError> {
    HeaderValue::try_from(value).map_err(|_| RequestError::InvalidHeaderValue(name.to_owned()))
}

fn normalize_value(value: HeaderValue) -> HeaderValue {
    let bytes = value.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t'))
        .map_or(start, |index| index + 1);
    HeaderValue::from_bytes(&bytes[start..end]).expect("trimmed header value remains valid")
}

fn header_value_string(value: &HeaderValue) -> String {
    String::from_utf8_lossy(value.as_bytes()).into_owned()
}

fn is_non_combinable(name: &HeaderName) -> bool {
    matches!(name.as_str(), "set-cookie" | "set-cookie2")
}

fn guard_transition_allowed(from: HeaderGuard, to: HeaderGuard) -> bool {
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

#[derive(Default)]
struct CorsExposure {
    wildcard: bool,
    names: Vec<HeaderName>,
}

impl CorsExposure {
    fn parse(headers: &HeaderMap, credentials_mode: CredentialsMode) -> Self {
        let mut names = Vec::new();
        let mut wildcard = false;
        for value in headers.get_all("access-control-expose-headers").iter() {
            let Ok(value) = value.to_str() else {
                return Self::default();
            };
            for item in value.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                if item == "*" {
                    wildcard = credentials_mode != CredentialsMode::Include;
                    continue;
                }
                let Ok(name) = HeaderName::from_bytes(item.as_bytes()) else {
                    return Self::default();
                };
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        Self { wildcard, names }
    }

    fn includes(&self, name: &HeaderName) -> bool {
        is_cors_safelisted_response_header(name) || self.wildcard || self.names.contains(name)
    }
}

fn is_cors_safelisted_response_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "cache-control"
            | "content-language"
            | "content-length"
            | "content-type"
            | "expires"
            | "last-modified"
            | "pragma"
    )
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
                .iter_raw()
                .map(|(name, _)| name.as_str().to_owned())
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
        headers.set_guard(HeaderGuard::RequestNoCors).unwrap();
        assert!(!headers.has("x-custom"));
        assert!(headers.has("accept"));
    }

    #[test]
    fn string_construction_validates_normalizes_and_trims() {
        let headers = Headers::try_from_strings([
            ("X-First", "  one\t"),
            ("x-first", "two"),
            ("Accept", "text/plain"),
        ])
        .unwrap();

        assert_eq!(headers.get("X-FIRST").as_deref(), Some("one, two"));
        assert_eq!(
            headers.iter().collect::<Vec<_>>(),
            [
                (HeaderName::from_static("x-first"), "one, two".to_owned()),
                (HeaderName::from_static("accept"), "text/plain".to_owned()),
            ]
        );
        assert!(matches!(
            Headers::try_from_strings([("bad name", "value")]),
            Err(RequestError::InvalidHeaderName(name)) if name == "bad name"
        ));
        assert!(matches!(
            Headers::try_from_strings([("x-test", "bad\r\nvalue")]),
            Err(RequestError::InvalidHeaderValue(name)) if name == "x-test"
        ));
    }

    #[test]
    fn map_and_headers_construction_preserve_duplicates_and_guard() {
        let mut map = HeaderMap::new();
        map.append("x-test", value("one"));
        map.append("x-test", value("two"));
        let headers = Headers::try_from_header_map(&map).unwrap();
        assert_eq!(headers.get("x-test").as_deref(), Some("one, two"));

        let cloned = Headers::from_headers(&headers);
        assert_eq!(cloned, headers);
        assert_eq!(cloned.guard(), HeaderGuard::Request);
    }

    #[test]
    fn public_iteration_combines_ordinary_values_but_not_set_cookie() {
        let mut headers = Headers::with_guard(HeaderGuard::None);
        headers.append_str("x-first", "one").unwrap();
        headers.append_str("set-cookie", "a=1").unwrap();
        headers.append_str("x-second", "middle").unwrap();
        headers.append_str("x-first", "two").unwrap();
        headers.append_str("set-cookie", "b=2").unwrap();

        assert_eq!(
            headers.iter().collect::<Vec<_>>(),
            [
                (HeaderName::from_static("x-first"), "one, two".to_owned()),
                (HeaderName::from_static("set-cookie"), "a=1".to_owned()),
                (HeaderName::from_static("x-second"), "middle".to_owned()),
                (HeaderName::from_static("set-cookie"), "b=2".to_owned()),
            ]
        );
        assert_eq!(headers.get_set_cookie(), ["a=1", "b=2"]);
        assert_eq!(headers.iter_raw().count(), 5);
    }

    #[test]
    fn guard_transitions_are_monotonic() {
        let allowed = [
            (HeaderGuard::None, HeaderGuard::Request),
            (HeaderGuard::None, HeaderGuard::RequestNoCors),
            (HeaderGuard::None, HeaderGuard::Response),
            (HeaderGuard::None, HeaderGuard::Immutable),
            (HeaderGuard::Request, HeaderGuard::RequestNoCors),
            (HeaderGuard::Request, HeaderGuard::Immutable),
            (HeaderGuard::RequestNoCors, HeaderGuard::Immutable),
            (HeaderGuard::Response, HeaderGuard::Immutable),
        ];
        for (from, to) in allowed {
            let mut headers = Headers::with_guard(from);
            headers.set_guard(to).unwrap();
            assert_eq!(headers.guard(), to);
        }

        let rejected = [
            (HeaderGuard::Request, HeaderGuard::None),
            (HeaderGuard::RequestNoCors, HeaderGuard::Request),
            (HeaderGuard::Response, HeaderGuard::Request),
            (HeaderGuard::Immutable, HeaderGuard::None),
            (HeaderGuard::Immutable, HeaderGuard::Request),
            (HeaderGuard::Immutable, HeaderGuard::Response),
        ];
        for (from, to) in rejected {
            let mut headers = Headers::with_guard(from);
            assert!(matches!(
                headers.set_guard(to),
                Err(RequestError::InvalidHeaderGuardTransition {
                    from: actual_from,
                    to: actual_to,
                }) if actual_from == from && actual_to == to
            ));
            assert_eq!(headers.guard(), from);
        }
    }

    #[test]
    fn immutable_headers_reject_typed_string_and_internal_mutations() {
        let mut headers = Headers::with_guard(HeaderGuard::Immutable);
        assert!(matches!(
            headers.set(HeaderName::from_static("x-test"), value("one")),
            Err(RequestError::ImmutableHeaders)
        ));
        assert!(matches!(
            headers.append_str("x-test", "one"),
            Err(RequestError::ImmutableHeaders)
        ));
        assert!(matches!(
            headers.set_str("x-test", "one"),
            Err(RequestError::ImmutableHeaders)
        ));
        assert!(matches!(
            headers.insert_internal_header(HeaderName::from_static("x-test"), value("one")),
            Err(RequestError::ImmutableHeaders)
        ));
    }

    #[test]
    fn cors_exposure_handles_wildcards_credentials_duplicates_and_malformed_values() {
        let mut headers = HeaderMap::new();
        headers.append("access-control-expose-headers", value("X-Visible"));
        headers.append("access-control-expose-headers", value("x-second"));
        headers.insert("x-visible", value("yes"));
        headers.insert("x-second", value("also"));
        headers.insert("x-secret", value("hidden"));
        headers.insert("content-type", value("text/plain"));
        headers.insert("set-cookie", value("sid=secret"));

        let exposed = Headers::exposed_response(&headers, true, CredentialsMode::SameOrigin);
        assert!(exposed.has("x-visible"));
        assert!(exposed.has("x-second"));
        assert!(exposed.has("content-type"));
        assert!(!exposed.has("x-secret"));
        assert!(!exposed.has("set-cookie"));

        headers.insert("access-control-expose-headers", value("*"));
        let wildcard = Headers::exposed_response(&headers, true, CredentialsMode::Omit);
        assert!(wildcard.has("x-secret"));
        assert!(!wildcard.has("set-cookie"));
        let credentialed = Headers::exposed_response(&headers, true, CredentialsMode::Include);
        assert!(!credentialed.has("x-secret"));

        headers.insert(
            "access-control-expose-headers",
            value("x-visible, bad name"),
        );
        let malformed = Headers::exposed_response(&headers, true, CredentialsMode::Omit);
        assert!(!malformed.has("x-visible"));
        assert!(malformed.has("content-type"));
    }

    #[test]
    fn basic_response_filter_removes_all_cookie_response_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("set-cookie", value("a=1"));
        headers.insert("set-cookie2", value("b=2"));
        headers.insert("x-visible", value("yes"));
        let exposed = Headers::exposed_response(&headers, false, CredentialsMode::SameOrigin);
        assert!(!exposed.has("set-cookie"));
        assert!(!exposed.has("set-cookie2"));
        assert!(exposed.has("x-visible"));
    }
}
