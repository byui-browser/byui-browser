//! Non-failing request lifecycle diagnostics.

use std::{sync::Arc, time::Instant};

/// A point in a Fetch request's lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkPhase {
    /// The controller accepted the caller's request.
    Started,
    /// Fetch validation and browser-service preparation completed.
    Prepared,
    /// The HTTP response cache is being consulted.
    CacheLookup,
    /// A response was supplied by the HTTP response cache.
    CacheHit,
    /// The HTTP response cache had no usable response.
    CacheMiss,
    /// A CORS-preflight permission was reused.
    PreflightCacheHit,
    /// A CORS-preflight permission was unavailable.
    PreflightCacheMiss,
    /// A CORS-preflight transport request is beginning.
    PreflightStarted,
    /// A CORS-preflight response arrived and is being checked.
    PreflightCompleted,
    /// The request is waiting for a shared transport slot.
    QueueStarted,
    /// The request acquired a shared transport slot.
    QueueAdmitted,
    /// The HTTP transport is being polled for response headers.
    TransportStarted,
    /// The HTTP transport delivered response headers.
    ResponseHeaders,
    /// A redirect response will be followed.
    Redirect,
    /// Fetch response filtering and browser response policy completed.
    ResponseFinalized,
    /// The controller returned a response to its caller.
    Completed,
    /// The request ended with an error other than cancellation.
    Failed,
    /// The request was cancelled.
    Aborted,
}

/// One timestamped network diagnostic.
///
/// `request_id` is the stable ID assigned when [`crate::Request`] is created.
/// `url` is a redacted form of the active URL and can change after a redirect.
/// Userinfo is removed and long or `data:` URLs are summarized before the
/// event reaches an observer.
#[derive(Clone, Debug)]
pub struct NetworkEvent {
    /// Stable identifier shared by all events for a request.
    pub request_id: u64,
    /// Lifecycle point being reported.
    pub phase: NetworkPhase,
    /// Monotonic time at which the event occurred.
    pub at: Instant,
    /// Redacted active request URL at this lifecycle point.
    pub url: String,
    /// HTTP response status once known.
    pub status: Option<u16>,
    /// Whether a response was served from the process-local HTTP cache.
    pub from_cache: bool,
    /// Redirect hops already followed when the event occurred.
    pub redirect_count: usize,
}

impl NetworkEvent {
    /// Creates an event at the current monotonic time.
    pub fn now(
        request_id: u64,
        phase: NetworkPhase,
        url: impl Into<String>,
        status: Option<u16>,
        from_cache: bool,
        redirect_count: usize,
    ) -> Self {
        let url = url.into();
        Self {
            request_id,
            phase,
            at: Instant::now(),
            url: crate::log_url::loggable(&url).into_owned(),
            status,
            from_cache,
            redirect_count,
        }
    }
}

/// Receives best-effort network lifecycle diagnostics.
///
/// Observers run on Fetch's asynchronous paths, must remain quick and
/// non-blocking, and must not panic. They cannot change Fetch behavior.
pub trait NetworkObserver: Send + Sync {
    /// Observes one lifecycle event.
    fn observe(&self, event: NetworkEvent);
}

/// Shared optional observer retained by a request controller.
pub(crate) type SharedNetworkObserver = Option<Arc<dyn NetworkObserver>>;
