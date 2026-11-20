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
    pub(super) id: u64,
    pub(super) url: String,
    pub(super) method: Method,
    pub(super) headers: Headers,
    pub(super) body: Option<Body>,
    pub(super) cache_mode: CacheMode,
    pub(super) redirect_mode: RedirectMode,
    pub(super) referrer: Referrer,
    pub(super) referrer_policy: ReferrerPolicy,
    pub(super) destination: RequestDestination,
    pub(super) service_workers: ServiceWorkersMode,
    pub(super) initiator: InitiatorType,
    pub(super) integrity: Option<String>,
    pub(super) keepalive: bool,
    pub(super) priority: RequestPriority,
    pub(super) duplex: Option<RequestDuplex>,
    pub(super) context: FetchContext,
    pub(super) signal: AbortSignal,
    pub(super) execution: RequestExecution,
}

/// Controller-owned state created and mutated only while Fetch executes.
///
/// Keeping this state separate from caller configuration prevents cache and
/// redirect bookkeeping from being mistaken for request initialization data.
#[derive(Debug, Default)]
pub(super) struct RequestExecution {
    pub(super) request_origin: Option<Origin>,
    pub(super) url_list: Vec<Url>,
    pub(super) current_url: Option<Url>,
    pub(super) redirect_count: usize,
    pub(super) cache_partition: Option<String>,
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
