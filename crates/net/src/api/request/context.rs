use url::Url;

use crate::api::error::RequestError;

use super::{NetworkPartitionKey, Origin};

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
    #[default]
    /// Use the normal HTTP cache algorithm.
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
    #[default]
    /// No destination was supplied.
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
    #[default]
    /// Derive the referrer from the client environment.
    Client,
    /// Send this explicit referrer URL.
    Url(Url),
}
/// Controls referrer reduction across origins.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReferrerPolicy {
    #[default]
    /// Use the user agent default policy.
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
    #[default]
    /// Ask the browser-owned provider whether a worker matches.
    /// Interception currently fails closed until worker responses are supported.
    All,
    /// Bypass service workers for this request.
    None,
}
/// The source that initiated a request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InitiatorType {
    #[default]
    /// No initiator type was supplied.
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
