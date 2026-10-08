//! Fetch-style request state and transport preparation.

use reqwest::{
    Method, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};

use crate::cancellation::AbortSignal;

/// A request after Fetch policy validation has produced a parsed URL.
///
/// This is the transport-facing form of [`Request`]. The controller keeps the
/// Fetch request model separate from Reqwest's header and body types so policy
/// code can run before transport-specific conversion.
#[derive(Clone, Debug)]
pub(crate) struct PreparedRequest {
    /// Request metadata and caller-supplied state.
    pub(crate) request: Request,
    /// Parsed current URL used by the HTTP transport.
    pub(crate) url: Url,
}

/// A serialized origin associated with the environment that initiated a request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Origin(String);

impl Origin {
    /// Creates an origin from its serialized representation.
    pub fn new(serialized: impl Into<String>) -> Self {
        Self(serialized.into())
    }

    /// Returns the serialized origin used in policy headers and diagnostics.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An ordered HTTP header list.
///
/// Fetch treats headers as an ordered multimap. Duplicate entries are retained
/// here and converted to Reqwest's transport representation only immediately
/// before I/O.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeaderList {
    entries: Vec<(HeaderName, HeaderValue)>,
}

impl HeaderList {
    /// Creates an empty header list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether the list contains no headers.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Appends a header without removing an existing entry with the same name.
    pub fn append(&mut self, name: HeaderName, value: HeaderValue) {
        self.entries.push((name, value));
    }

    /// Replaces all entries with `name` with one header value.
    pub fn insert(&mut self, name: HeaderName, value: HeaderValue) {
        self.entries.retain(|(existing, _)| existing != name);
        self.entries.push((name, value));
    }

    /// Iterates over headers in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.entries.iter().map(|(name, value)| (name, value))
    }

    /// Converts the list to the representation expected by the HTTP transport.
    pub(crate) fn to_reqwest(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.entries {
            headers.append(name.clone(), value.clone());
        }
        headers
    }
}

impl From<HeaderMap> for HeaderList {
    /// Copies a Reqwest header map into an ordered Fetch header list.
    fn from(headers: HeaderMap) -> Self {
        let mut list = Self::new();
        for (name, value) in headers {
            if let Some(name) = name {
                list.append(name, value);
            }
        }
        list
    }
}

/// A request body that can later be extended with streaming and form-data sources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestBody {
    /// A replayable byte sequence suitable for redirects and retries.
    Bytes(Vec<u8>),
}

impl RequestBody {
    /// Creates a replayable byte body.
    pub fn bytes(body: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(body.into())
    }

    /// Returns the body bytes for transport conversion.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Bytes(body) => body,
        }
    }
}

/// Controls the browser context in which a request was initiated.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FetchContext {
    /// Origin of the document or worker that initiated the request.
    pub origin: Option<Origin>,
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
    /// Follow redirects subject to the controller's policy.
    Follow,
    /// Fail when the response would redirect.
    Error,
    /// Return a filtered manual-redirect response.
    Manual,
}

/// Controls which cache algorithm a request selects.
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
    /// Allow matching service workers to receive the fetch event.
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
    /// The original request URL. The controller parses it before I/O.
    pub url: String,
    /// The HTTP method.
    pub method: Method,
    /// The ordered Fetch header list.
    pub headers: HeaderList,
    /// Optional replayable request body.
    pub body: Option<RequestBody>,
    /// Controls the local HTTP cache algorithm.
    pub cache_mode: CacheMode,
    /// Controls redirect handling.
    pub redirect_mode: RedirectMode,
    /// Controls referrer generation.
    pub referrer: Referrer,
    /// Controls referrer reduction.
    pub referrer_policy: ReferrerPolicy,
    /// Fetch destination used by CSP, mixed-content, and service-worker policy.
    pub destination: RequestDestination,
    /// Whether matching service workers may intercept this request.
    pub service_workers: ServiceWorkersMode,
    /// Fetch initiator used by policy and timing integrations.
    pub initiator: InitiatorType,
    /// Optional subresource integrity metadata.
    pub integrity: Option<String>,
    /// Whether the request may outlive its initiating environment.
    pub keepalive: bool,
    /// Scheduler priority requested by the caller.
    pub priority: RequestPriority,
    /// Browser context used by security, cookie, and CORS policy modules.
    pub context: FetchContext,
    /// Cancellation signal for this request.
    pub signal: AbortSignal,
    /// URLs visited by this request, including the initial URL after preparation.
    pub(crate) url_list: Vec<Url>,
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
        }
    }

    /// Returns whether this request method may use the response cache.
    pub(crate) fn is_cacheable_method(&self) -> bool {
        matches!(self.method, Method::GET | Method::HEAD)
    }

    /// Returns the transport priority corresponding to the Fetch priority.
    pub(crate) fn transport_priority(&self) -> crate::scheduler::RequestPriority {
        match self.priority {
            RequestPriority::Low => crate::scheduler::RequestPriority::Low,
            RequestPriority::Auto => crate::scheduler::RequestPriority::Normal,
            RequestPriority::High => crate::scheduler::RequestPriority::High,
        }
    }

    /// Computes the `Referer` value for a request target.
    pub(crate) fn referrer_value(&self, target: &Url) -> Option<String> {
        let Referrer::Url(source) = &self.referrer else {
            return None;
        };
        let same_origin = source.origin() == target.origin();
        let downgrade = source.scheme() == "https" && target.scheme() == "http";
        let source_url = source.as_str().to_owned();
        let source_origin = source.origin().ascii_serialization();

        let policy = match self.referrer_policy {
            // Fetch's user-agent default is the strict-origin-when-cross-origin policy.
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
            // `Empty` is normalized above, but retaining this arm keeps the
            // match exhaustive if the policy normalization changes later.
            ReferrerPolicy::Empty => Some(source_origin),
        }
    }
}
