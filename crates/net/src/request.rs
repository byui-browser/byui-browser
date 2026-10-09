//! Fetch-style request state and transport preparation.

use std::{
    fmt, io,
    pin::Pin,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures_util::{Stream, TryStreamExt};
use http::{
    Method,
    header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
};
use reqwest::Body;
use url::Url;

use crate::{cancellation::AbortSignal, error::RequestError};

type RequestStream = Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>;

/// A request after Fetch policy validation has produced a parsed URL.
#[derive(Clone, Debug)]
pub(crate) struct PreparedRequest {
    /// Request metadata and caller-supplied state.
    pub(crate) request: Request,
    /// Parsed current URL used by the HTTP transport.
    pub(crate) url: Url,
}

/// Structured origin state for a client environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Origin {
    scheme: Option<String>,
    host: Option<String>,
    port: Option<u16>,
}

impl Origin {
    /// Creates the tuple origin for an HTTP(S) URL.
    pub fn from_url(url: &Url) -> Result<Self, RequestError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RequestError::UnsupportedScheme(url.scheme().to_owned()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| RequestError::InvalidUrl(url.as_str().to_owned()))?;
        Ok(Self {
            scheme: Some(url.scheme().to_ascii_lowercase()),
            host: Some(host.to_ascii_lowercase()),
            port: url.port_or_known_default(),
        })
    }

    /// Creates an opaque origin, which is unequal to every tuple origin.
    pub fn opaque() -> Self {
        Self {
            scheme: None,
            host: None,
            port: None,
        }
    }

    /// Parses a serialized HTTP(S) origin and rejects paths, credentials, and queries.
    pub fn parse(serialized: &str) -> Result<Self, RequestError> {
        let url = Url::parse(serialized)
            .map_err(|_| RequestError::InvalidOrigin(serialized.to_owned()))?;
        if url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(RequestError::InvalidOrigin(serialized.to_owned()));
        }
        Self::from_url(&url).map_err(|_| RequestError::InvalidOrigin(serialized.to_owned()))
    }

    /// Returns this origin's canonical serialization, or `"null"` for opaque origins.
    pub fn as_str(&self) -> String {
        let (Some(scheme), Some(host), Some(port)) = (&self.scheme, &self.host, self.port) else {
            return "null".to_owned();
        };
        let default_port = match scheme.as_str() {
            "http" => 80,
            "https" => 443,
            _ => port,
        };
        if port == default_port {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}:{port}")
        }
    }

    /// Returns whether this is an opaque origin.
    pub fn is_opaque(&self) -> bool {
        self.scheme.is_none()
    }

    /// Tests tuple-origin equality; opaque origins never match, including themselves.
    pub fn is_same_origin(&self, other: &Self) -> bool {
        !self.is_opaque() && !other.is_opaque() && self == other
    }
}

/// Top-level origin used to partition browser network state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPartitionKey {
    /// Origin of the top-level site containing the request's client.
    pub top_level_origin: Origin,
}

impl NetworkPartitionKey {
    /// Creates a partition key from the top-level site's structured origin.
    pub fn new(top_level_origin: Origin) -> Self {
        Self { top_level_origin }
    }
}

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
    entries: Vec<(HeaderName, HeaderValue)>,
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

fn is_cors_safelisted_header(name: &HeaderName, value: &HeaderValue) -> bool {
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

/// A request body, either repeatable bytes or a shared one-shot byte/stream source.
#[derive(Clone)]
pub struct RequestBody {
    source: BodySource,
    content_type: Option<HeaderValue>,
    length: Option<u64>,
}

#[derive(Clone)]
enum BodySource {
    Bytes(Bytes),
    OneShotBytes(Arc<Mutex<Option<Bytes>>>),
    OneShotStream(Arc<Mutex<Option<RequestStream>>>),
}

impl fmt::Debug for RequestBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestBody")
            .field("replayable", &self.is_replayable())
            .field("content_type", &self.content_type)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

impl PartialEq for RequestBody {
    fn eq(&self, other: &Self) -> bool {
        if self.content_type != other.content_type || self.length != other.length {
            return false;
        }
        match (&self.source, &other.source) {
            (BodySource::Bytes(a), BodySource::Bytes(b)) => a == b,
            (BodySource::OneShotBytes(a), BodySource::OneShotBytes(b)) => Arc::ptr_eq(a, b),
            (BodySource::OneShotStream(a), BodySource::OneShotStream(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for RequestBody {}

impl RequestBody {
    /// Creates a replayable byte body with no inferred media type.
    pub fn bytes(body: impl Into<Vec<u8>>) -> Self {
        let bytes = Bytes::from(body.into());
        let length = Some(bytes.len() as u64);
        Self {
            source: BodySource::Bytes(bytes),
            content_type: None,
            length,
        }
    }

    /// Creates a replayable UTF-8 text body with the Fetch default text media type.
    pub fn text(body: impl Into<String>) -> Self {
        Self::with_content_type(
            body.into().into_bytes(),
            HeaderValue::from_static("text/plain;charset=UTF-8"),
        )
    }

    /// Creates replayable bytes with caller-selected content type.
    pub fn with_content_type(body: impl Into<Vec<u8>>, content_type: HeaderValue) -> Self {
        let bytes = Bytes::from(body.into());
        let length = Some(bytes.len() as u64);
        Self {
            source: BodySource::Bytes(bytes),
            content_type: Some(content_type),
            length,
        }
    }

    /// Creates a URL-encoded form body and sets its standard media type.
    pub fn url_encoded(body: impl Into<Vec<u8>>) -> Self {
        Self::with_content_type(
            body,
            HeaderValue::from_static("application/x-www-form-urlencoded;charset=UTF-8"),
        )
    }

    /// Creates a one-shot body from bytes. Clones share consumption state.
    pub fn one_shot_bytes(body: impl Into<Vec<u8>>) -> Self {
        let bytes = Bytes::from(body.into());
        let length = Some(bytes.len() as u64);
        Self {
            source: BodySource::OneShotBytes(Arc::new(Mutex::new(Some(bytes)))),
            content_type: None,
            length,
        }
    }

    /// Creates a one-shot streaming body. Its byte length is unknown.
    pub fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Vec<u8>, io::Error>> + Send + 'static,
    {
        Self {
            source: BodySource::OneShotStream(Arc::new(Mutex::new(Some(Box::pin(
                stream.map_ok(Bytes::from),
            ))))),
            content_type: None,
            length: None,
        }
    }

    /// Returns true when this body can be sent repeatedly, such as after a redirect.
    pub fn is_replayable(&self) -> bool {
        matches!(self.source, BodySource::Bytes(_))
    }

    /// Returns the known body length in bytes.
    pub fn length(&self) -> Option<u64> {
        self.length
    }

    /// Returns the media type inferred or assigned to this body.
    pub fn content_type(&self) -> Option<&HeaderValue> {
        self.content_type.as_ref()
    }

    pub(crate) fn apply_metadata(&self, headers: &mut HeaderList) {
        if let Some(content_type) = &self.content_type
            && !headers.entries.iter().any(|(name, _)| name == CONTENT_TYPE)
        {
            headers.insert_internal_header(CONTENT_TYPE, content_type.clone());
        }
    }

    pub(crate) fn into_reqwest_body(self) -> Result<Body, RequestError> {
        match self.source {
            BodySource::Bytes(bytes) => Ok(Body::from(bytes)),
            BodySource::OneShotBytes(shared) => shared
                .lock()
                .map_err(|_| RequestError::BodyAlreadyConsumed)?
                .take()
                .map(Body::from)
                .ok_or(RequestError::BodyAlreadyConsumed),
            BodySource::OneShotStream(shared) => {
                let stream = shared
                    .lock()
                    .map_err(|_| RequestError::BodyAlreadyConsumed)?
                    .take()
                    .ok_or(RequestError::BodyAlreadyConsumed)?;
                Ok(Body::wrap_stream(stream.map_err(|error| error)))
            }
        }
    }
}

/// Client environment used to resolve URLs and derive request origin/referrer state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FetchEnvironment {
    /// Base URL used to resolve relative request URLs.
    pub base_url: Option<Url>,
    /// Structured origin of the document or worker that initiated the request.
    pub origin: Option<Origin>,
    /// Environment URL used as the default referrer source.
    pub referrer: Option<Url>,
    /// Top-level site key used to partition cookies, caches, and connection state.
    pub network_partition_key: Option<NetworkPartitionKey>,
}

impl FetchEnvironment {
    /// Builds an environment whose base URL, origin, and referrer come from one document URL.
    pub fn from_url(url: Url) -> Result<Self, RequestError> {
        let origin = Origin::from_url(&url)?;
        Ok(Self {
            base_url: Some(url.clone()),
            origin: Some(origin.clone()),
            referrer: Some(url),
            network_partition_key: Some(NetworkPartitionKey::new(origin)),
        })
    }
}

/// Controls the browser context in which a request was initiated.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FetchContext {
    /// Environment state used by URL, origin, and referrer algorithms.
    pub environment: FetchEnvironment,
    /// Cross-origin mode used when making the request.
    pub mode: RequestMode,
    /// Whether credentials such as cookies may be included.
    pub credentials: CredentialsMode,
}

/// Controls how a request may cross origins.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequestMode {
    /// Used for document navigation requests.
    Navigate,
    #[default]
    /// Apply CORS processing to the request and response.
    Cors,
    /// Permit the request only when it stays within the initiating origin.
    SameOrigin,
    /// Use the restricted no-CORS fetch behavior.
    NoCors,
}

/// Controls whether credentials may be sent with a request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CredentialsMode {
    /// Never send credentials.
    Omit,
    #[default]
    /// Send credentials only for same-origin requests.
    SameOrigin,
    /// Permit credentials for cross-origin requests when policy allows them.
    Include,
}

/// Controls how redirects are exposed to the caller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RedirectMode {
    #[default]
    /// Requests redirect following. Redirect responses currently fail with
    /// `RedirectFailure` until the controller-owned redirect loop exists.
    Follow,
    /// Fail when the response would redirect.
    Error,
    /// Return a filtered manual-redirect response.
    Manual,
}

/// Controls which process-local cache behavior a request selects.
///
/// This is a documented HTTP cache subset. `NoCache` is rejected until
/// revalidation exists; full Fetch cache matching belongs to a later step.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CacheMode {
    /// Use the normal HTTP cache algorithm.
    #[default]
    Default,
    /// Skip the cache and do not store the response.
    NoStore,
    /// Skip an existing cached response, but permit the new response to be cached.
    Reload,
    /// Revalidate a cached response before using it.
    NoCache,
    /// Use a cached response when available, even if stale.
    ForceCache,
    /// Return a cached response or fail without making a network request.
    OnlyIfCached,
}

/// The destination type associated with a fetch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequestDestination {
    /// No destination was supplied.
    #[default]
    Empty,
    /// A document navigation.
    Document,
    /// An image resource.
    Image,
    /// A script resource.
    Script,
    /// A stylesheet resource.
    Style,
    /// A font resource.
    Font,
    /// A JSON resource.
    Json,
    /// A text resource.
    Text,
    /// A worker resource.
    Worker,
    /// An audio resource.
    Audio,
    /// A video resource.
    Video,
}

/// Controls the referrer sent with a request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Referrer {
    /// Do not send a referrer.
    NoReferrer,
    /// Derive the referrer from the client environment.
    #[default]
    Client,
    /// Send this explicit referrer URL.
    Url(Url),
}

/// Controls referrer reduction across origins.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReferrerPolicy {
    /// Use the user agent default policy.
    #[default]
    Empty,
    /// Send no referrer.
    NoReferrer,
    /// Send the full referrer except on HTTPS-to-HTTP downgrades.
    NoReferrerWhenDowngrade,
    /// Send only the origin.
    Origin,
    /// Send the full URL same-origin and only the origin cross-origin.
    OriginWhenCrossOrigin,
    /// Send a referrer only for same-origin requests.
    SameOrigin,
    /// Send only the origin, except on downgrades.
    StrictOrigin,
    /// Send the full URL same-origin and the origin cross-origin, except on downgrades.
    StrictOriginWhenCrossOrigin,
    /// Send the full URL.
    UnsafeUrl,
}

/// Controls whether matching service workers may intercept a request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ServiceWorkersMode {
    /// Ask the browser-owned provider whether a worker matches.
    /// Interception currently fails closed until worker responses are supported.
    #[default]
    All,
    /// Bypass service workers for this request.
    None,
}

/// The source that initiated a request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InitiatorType {
    /// No initiator type was supplied.
    #[default]
    Other,
    /// JavaScript `fetch()`.
    Fetch,
    /// A document navigation.
    Document,
    /// A stylesheet or CSS resource.
    Css,
    /// An image resource.
    Image,
    /// A script resource.
    Script,
}

/// A browser request before conversion to a transport request.
#[derive(Clone, Debug)]
pub struct Request {
    pub(crate) url: String,
    pub(crate) method: Method,
    pub(crate) headers: HeaderList,
    pub(crate) body: Option<RequestBody>,
    pub(crate) cache_mode: CacheMode,
    pub(crate) redirect_mode: RedirectMode,
    pub(crate) referrer: Referrer,
    pub(crate) referrer_policy: ReferrerPolicy,
    pub(crate) destination: RequestDestination,
    pub(crate) service_workers: ServiceWorkersMode,
    pub(crate) initiator: InitiatorType,
    pub(crate) integrity: Option<String>,
    pub(crate) keepalive: bool,
    pub(crate) priority: RequestPriority,
    pub(crate) context: FetchContext,
    pub(crate) signal: AbortSignal,
    pub(crate) url_list: Vec<Url>,
    pub(crate) current_url: Option<Url>,
    pub(crate) redirect_count: usize,
    pub(crate) cache_partition: Option<String>,
}

/// Relative importance assigned to a request by the network scheduler.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum RequestPriority {
    /// Background work.
    Low,
    /// Ordinary network work.
    #[default]
    Auto,
    /// User-visible work.
    High,
}

impl Request {
    /// Creates a GET request with Fetch's default request state.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: Method::GET,
            headers: HeaderList::new(),
            body: None,
            cache_mode: CacheMode::Default,
            redirect_mode: RedirectMode::Follow,
            referrer: Referrer::Client,
            referrer_policy: ReferrerPolicy::Empty,
            destination: RequestDestination::Empty,
            service_workers: ServiceWorkersMode::All,
            initiator: InitiatorType::Other,
            integrity: None,
            keepalive: false,
            priority: RequestPriority::Auto,
            context: FetchContext::default(),
            signal: AbortSignal::new(),
            url_list: Vec::new(),
            current_url: None,
            redirect_count: 0,
            cache_partition: None,
        }
    }

    /// Creates a request after normalizing and validating its method.
    pub fn new(url: impl Into<String>, method: impl AsRef<str>) -> Result<Self, RequestError> {
        let mut request = Self::get(url);
        request.set_method(method)?;
        Ok(request)
    }

    /// Returns the original URL string supplied for this request.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the normalized HTTP method.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Returns the guarded request header list.
    pub fn headers(&self) -> &HeaderList {
        &self.headers
    }

    /// Returns this request's optional body.
    pub fn body(&self) -> Option<&RequestBody> {
        self.body.as_ref()
    }

    /// Returns the resolved current URL once request preparation has run.
    pub fn current_url(&self) -> Option<&Url> {
        self.current_url.as_ref()
    }

    /// Returns the configured request mode.
    pub fn mode(&self) -> RequestMode {
        self.context.mode
    }

    /// Returns the configured credentials mode.
    pub fn credentials_mode(&self) -> CredentialsMode {
        self.context.credentials
    }

    /// Returns the client environment used to derive URL and origin state.
    pub fn environment(&self) -> &FetchEnvironment {
        &self.context.environment
    }

    /// Returns all URLs visited by this request, in order.
    /// The supported single-exchange path never follows a redirect.
    pub fn url_list(&self) -> &[Url] {
        &self.url_list
    }

    /// Returns the number of redirects followed so far.
    /// This remains zero until controller-owned redirects are implemented.
    pub fn redirect_count(&self) -> usize {
        self.redirect_count
    }

    /// Replaces the method after Fetch token and forbidden-method validation.
    pub fn set_method(&mut self, method: impl AsRef<str>) -> Result<(), RequestError> {
        let method = method.as_ref();
        let parsed = Method::from_bytes(method.as_bytes())
            .map_err(|_| RequestError::InvalidMethod(method.to_owned()))?;
        let normalized = match parsed.as_str().to_ascii_uppercase().as_str() {
            "DELETE" => Method::DELETE,
            "GET" => Method::GET,
            "HEAD" => Method::HEAD,
            "OPTIONS" => Method::OPTIONS,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            _ => parsed,
        };
        if matches!(normalized.as_str(), "CONNECT" | "TRACE" | "TRACK") {
            return Err(RequestError::ForbiddenMethod(normalized.to_string()));
        }
        if self.context.mode == RequestMode::NoCors
            && !matches!(normalized, Method::GET | Method::HEAD | Method::POST)
        {
            return Err(RequestError::NoCorsMethod(normalized.to_string()));
        }
        self.method = normalized;
        Ok(())
    }

    /// Appends a caller-controlled request header under the active Fetch guard.
    pub fn append_header(
        &mut self,
        name: HeaderName,
        value: HeaderValue,
    ) -> Result<(), RequestError> {
        self.headers.append(name, value)
    }

    /// Replaces caller-controlled values for a header under the active Fetch guard.
    pub fn set_header(&mut self, name: HeaderName, value: HeaderValue) -> Result<(), RequestError> {
        self.headers.insert(name, value)
    }

    /// Changes request mode and applies the corresponding request-header guard.
    pub fn set_mode(&mut self, mode: RequestMode) -> Result<(), RequestError> {
        if mode == RequestMode::NoCors
            && !matches!(self.method, Method::GET | Method::HEAD | Method::POST)
        {
            return Err(RequestError::NoCorsMethod(self.method.to_string()));
        }
        self.context.mode = mode;
        self.headers.set_guard(if mode == RequestMode::NoCors {
            HeaderGuard::RequestNoCors
        } else {
            HeaderGuard::Request
        });
        Ok(())
    }

    /// Sets the credentials mode used by cookie and authentication policy.
    pub fn set_credentials_mode(&mut self, mode: CredentialsMode) {
        self.context.credentials = mode;
    }

    /// Sets the initiating client environment.
    pub fn set_environment(&mut self, environment: FetchEnvironment) {
        self.context.environment = environment;
    }

    /// Sets or clears the body and derives its content type when absent.
    pub fn set_body(&mut self, body: Option<RequestBody>) -> Result<(), RequestError> {
        if body.is_some() && matches!(self.method, Method::GET | Method::HEAD) {
            return Err(RequestError::BodyNotAllowed(self.method.to_string()));
        }
        self.body = body;
        if let Some(body) = &self.body {
            body.apply_metadata(&mut self.headers);
        }
        Ok(())
    }

    /// Sets the Fetch cache mode.
    pub fn set_cache_mode(&mut self, mode: CacheMode) {
        self.cache_mode = mode;
    }

    /// Sets the Fetch redirect mode.
    /// `Manual` and `Error` are rejected before network I/O in this slice.
    pub fn set_redirect_mode(&mut self, mode: RedirectMode) {
        self.redirect_mode = mode;
    }

    /// Sets the referrer source and reduction policy.
    pub fn set_referrer(&mut self, referrer: Referrer, policy: ReferrerPolicy) {
        self.referrer = referrer;
        self.referrer_policy = policy;
    }

    /// Sets the request destination used by browser policy integrations.
    pub fn set_destination(&mut self, destination: RequestDestination) {
        self.destination = destination;
    }

    /// Sets whether service workers may intercept this request.
    pub fn set_service_workers_mode(&mut self, mode: ServiceWorkersMode) {
        self.service_workers = mode;
    }

    /// Sets the source that initiated this request.
    pub fn set_initiator(&mut self, initiator: InitiatorType) {
        self.initiator = initiator;
    }

    /// Sets optional subresource integrity metadata.
    pub fn set_integrity(&mut self, integrity: Option<String>) {
        self.integrity = integrity;
    }

    /// Sets the scheduler priority for this request.
    pub fn set_priority(&mut self, priority: RequestPriority) {
        self.priority = priority;
    }

    /// Sets the cancellation signal.
    pub fn set_signal(&mut self, signal: AbortSignal) {
        self.signal = signal;
    }

    /// Requests a Fetch keepalive lifetime.
    ///
    /// The engine checks the configured byte limit but currently rejects
    /// keepalive execution with `UnsupportedFeature` before network I/O.
    pub fn set_keepalive(&mut self, keepalive: bool) {
        self.keepalive = keepalive;
    }

    /// Returns whether this request method may use the response cache.
    pub(crate) fn is_cacheable_method(&self) -> bool {
        matches!(self.method, Method::GET | Method::HEAD)
    }

    /// Returns whether the credentials mode permits credentials for this URL.
    pub(crate) fn credentials_allowed(&self, url: &Url) -> bool {
        match self.context.credentials {
            CredentialsMode::Omit => false,
            CredentialsMode::Include => true,
            CredentialsMode::SameOrigin => {
                self.context
                    .environment
                    .origin
                    .as_ref()
                    .is_some_and(|origin| {
                        Origin::from_url(url).is_ok_and(|target| origin.is_same_origin(&target))
                    })
            }
        }
    }

    pub(crate) fn requires_cors_preflight(&self) -> bool {
        !matches!(self.method, Method::GET | Method::HEAD | Method::POST)
            || self
                .headers
                .iter()
                .any(|(name, value)| !is_cors_safelisted_header(name, value))
    }

    /// Returns the transport priority corresponding to the Fetch priority.
    pub(crate) fn transport_priority(&self) -> crate::scheduler::RequestPriority {
        match self.priority {
            RequestPriority::Low => crate::scheduler::RequestPriority::Low,
            RequestPriority::Auto => crate::scheduler::RequestPriority::Normal,
            RequestPriority::High => crate::scheduler::RequestPriority::High,
        }
    }

    pub(crate) fn prepare_url(&mut self, url: Url) {
        if self.url_list.is_empty() {
            self.url_list.push(url.clone());
        }
        self.current_url = Some(url);
    }

    pub(crate) fn apply_fetch_headers(&mut self) -> Result<(), RequestError> {
        if let Some(body) = &self.body {
            body.apply_metadata(&mut self.headers);
        }
        let needs_origin = self.context.mode == RequestMode::Cors
            || !matches!(self.method, Method::GET | Method::HEAD);
        if needs_origin && let Some(origin) = &self.context.environment.origin {
            let value = HeaderValue::try_from(origin.as_str())
                .map_err(|_| RequestError::InvalidOrigin(origin.as_str()))?;
            self.headers
                .insert_internal_header(HeaderName::from_static("origin"), value);
        }
        Ok(())
    }

    pub(crate) fn record_redirect(&mut self, url: Url) {
        self.redirect_count += 1;
        self.current_url = Some(url.clone());
        self.url_list.push(url);
    }

    /// Computes the `Referer` value for a request target.
    pub(crate) fn referrer_value(&self, target: &Url) -> Option<String> {
        let source = match &self.referrer {
            Referrer::NoReferrer => return None,
            Referrer::Client => self.context.environment.referrer.as_ref()?,
            Referrer::Url(source) => source,
        };
        let same_origin = source.origin() == target.origin();
        let downgrade = source.scheme() == "https" && target.scheme() == "http";
        let source_url = source.as_str().to_owned();
        let source_origin = source.origin().ascii_serialization();
        let policy = match self.referrer_policy {
            ReferrerPolicy::Empty => ReferrerPolicy::StrictOriginWhenCrossOrigin,
            policy => policy,
        };
        match policy {
            ReferrerPolicy::UnsafeUrl => Some(source_url),
            ReferrerPolicy::NoReferrer => None,
            ReferrerPolicy::NoReferrerWhenDowngrade if downgrade => None,
            ReferrerPolicy::NoReferrerWhenDowngrade => Some(source_url),
            ReferrerPolicy::Origin => Some(source_origin),
            ReferrerPolicy::OriginWhenCrossOrigin if same_origin => Some(source_url),
            ReferrerPolicy::OriginWhenCrossOrigin => Some(source_origin),
            ReferrerPolicy::SameOrigin if same_origin => Some(source_url),
            ReferrerPolicy::SameOrigin => None,
            ReferrerPolicy::StrictOrigin if downgrade => None,
            ReferrerPolicy::StrictOrigin => Some(source_origin),
            ReferrerPolicy::StrictOriginWhenCrossOrigin if downgrade => None,
            ReferrerPolicy::StrictOriginWhenCrossOrigin if same_origin => Some(source_url),
            ReferrerPolicy::StrictOriginWhenCrossOrigin => Some(source_origin),
            ReferrerPolicy::Empty => Some(source_origin),
        }
    }

    pub(crate) fn validate_body_state(
        &self,
        max_keepalive_body_size: u64,
    ) -> Result<(), RequestError> {
        if self.body.is_some() && matches!(self.method, Method::GET | Method::HEAD) {
            return Err(RequestError::BodyNotAllowed(self.method.to_string()));
        }
        if self.context.mode == RequestMode::NoCors
            && let Some(body) = &self.body
            && let Some(content_type) = body.content_type()
            && !is_cors_safelisted_header(&CONTENT_TYPE, content_type)
        {
            return Err(RequestError::ForbiddenHeader(
                CONTENT_TYPE.as_str().to_owned(),
            ));
        }
        if self.keepalive {
            let body_size = match &self.body {
                None => Some(0),
                Some(body) => body.length(),
            };
            if body_size.is_none_or(|size| size > max_keepalive_body_size) {
                return Err(RequestError::KeepaliveBodyTooLarge {
                    limit: max_keepalive_body_size,
                });
            }
        }
        Ok(())
    }
}
