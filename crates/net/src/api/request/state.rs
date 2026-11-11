use http::Method;
use url::Url;

use crate::api::{Headers, body::Body, cancellation::AbortSignal};

use super::{
    CacheMode, FetchContext, InitiatorType, Origin, RedirectMode, Referrer, ReferrerPolicy,
    RequestDestination, ServiceWorkersMode,
};

/// A browser request before conversion to a transport request.
#[derive(Debug)]
pub struct Request {
    pub(crate) id: u64,
    pub(crate) url: String,
    pub(crate) method: Method,
    pub(crate) headers: Headers,
    pub(crate) body: Option<Body>,
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
    pub(crate) duplex: Option<RequestDuplex>,
    pub(crate) context: FetchContext,
    pub(crate) request_origin: Option<Origin>,
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
    #[default]
    /// Ordinary network work.
    Auto,
    /// User-visible work.
    High,
}

/// Fetch request streaming mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestDuplex {
    /// The response is not exposed until the request body has begun sending.
    Half,
}
