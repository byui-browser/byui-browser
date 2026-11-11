use http::header::{HeaderMap, HeaderName, HeaderValue};

use crate::api::{error::RequestError, request::CredentialsMode};

use super::{
    HeaderGuard,
    cors::CorsExposure,
    guard::{allowed_header, guard_transition_allowed},
    validation::{
        header_value_string, is_non_combinable, normalize_value, parse_name, parse_value,
    },
};

/// An ordered HTTP header collection with a Fetch mutation guard.
///
/// `Headers` is used for both request and response headers. The guard controls
/// which operations are available: request headers are caller-mutable,
/// response headers are filtered, and exposed response headers are immutable.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Headers {
    pub(in crate::api) entries: Vec<(HeaderName, HeaderValue)>,
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

    /// Creates a header collection for browser-owned generated request fields.
    ///
    /// This constructor is intended for [`crate::FetchServices`] implementations
    /// that generate headers such as Fetch Metadata fields, which ordinary
    /// callers are forbidden from setting. The controller accepts this list
    /// only through its internal transport channel and protects Fetch-owned
    /// `Origin`, referrer, body, cookie, and preflight fields from replacement.
    pub fn generated_request_headers() -> Self {
        Self::with_guard(HeaderGuard::None)
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
            self.entries.retain(|(name, value)| {
                let keep = allowed_header(guard, name, value);
                if !keep {
                    log::trace!("header {name} dropped by {guard:?} guard");
                }
                keep
            });
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
            } else {
                log::trace!("response header {name} hidden from caller");
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

    /// Validates a mutation against the current guard and Fetch header rules.
    fn check(&self, name: &HeaderName, value: &HeaderValue) -> Result<(), RequestError> {
        if self.guard == HeaderGuard::Immutable {
            log::trace!("mutation of header {name} refused: headers are immutable");
            return Err(RequestError::ImmutableHeaders);
        }
        if allowed_header(self.guard, name, value) {
            Ok(())
        } else {
            log::debug!("header {name} refused by {:?} guard", self.guard);
            Err(RequestError::ForbiddenHeader(name.as_str().to_owned()))
        }
    }

    /// Inserts an engine-managed header while retaining the current guard.
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

    /// Merges browser-generated fields while retaining Fetch ownership of
    /// request metadata and preflight fields.
    pub(crate) fn merge_generated_request_headers(
        &mut self,
        headers: &Self,
    ) -> Result<(), RequestError> {
        for (name, value) in headers.iter_raw() {
            if is_fetch_owned_generated_header(name) {
                return Err(RequestError::ForbiddenHeader(name.as_str().to_owned()));
            }
            self.insert_internal_header(name.clone(), value.clone())?;
        }
        Ok(())
    }

    pub(crate) fn remove_internal_header(&mut self, name: HeaderName) {
        self.entries.retain(|(existing, _)| existing != name);
    }

    /// Combines ordinary duplicate values for the public iterator.
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

/// Reports whether Fetch, rather than browser identity policy, owns a field.
fn is_fetch_owned_generated_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "origin"
            | "referer"
            | "cookie"
            | "content-type"
            | "content-length"
            | "access-control-request-headers"
            | "access-control-request-method"
    )
}
