//! Controller construction and shared engine state.

use std::sync::Arc;

use crate::{
    api::{config::Config, error::RequestError, services::FetchServices},
    cache::ResponseCache,
    policy::{CorsChecker, RequestPolicy},
    scheduling::RequestScheduler,
    transport::reqwest_transport,
};

/// Shared, stateful entry point for browser network requests.
///
/// A controller owns the process-local cache and scheduler while sharing the
/// connection pool held by its transport. Cloning a controller is therefore
/// inexpensive and preserves cache and connection reuse. Use `with_services`
/// for requests carrying a browser client origin; `new` is for standalone
/// requests without browser-owned policy or storage context.
#[derive(Clone)]
pub struct RequestController {
    pub(crate) inner: Arc<ControllerInner>,
}

pub(crate) struct ControllerInner {
    /// Maximum accepted body size for keepalive requests.
    pub(crate) max_keepalive_body_size: u64,
    /// Process-local response cache shared by controller clones.
    pub(crate) cache: ResponseCache,
    /// Browser-owned policy, credential, cache-partition, and worker decisions.
    pub(crate) services: Option<Arc<dyn FetchServices>>,
    /// Cross-origin response validation boundary.
    pub(crate) cors: CorsChecker,
    /// Request and response policy validation boundary.
    pub(crate) policy: RequestPolicy,
    /// Shared transport admission and concurrency controller.
    pub(crate) scheduler: RequestScheduler,
}

impl std::fmt::Debug for RequestController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestController").finish_non_exhaustive()
    }
}

impl RequestController {
    /// Creates a controller with the supplied transport, cache, and scheduling
    /// configuration.
    ///
    /// The controller does not perform network I/O during construction. A
    /// [`RequestError::Initialization`] is returned if the underlying HTTP
    /// client cannot be initialized.
    pub fn new(config: Config) -> Result<Self, RequestError> {
        Self::build(config, None)
    }

    /// Creates a controller with browser-owned service decisions.
    ///
    /// The service is shared by controller clones and must own any durable
    /// storage or policy data outside this process-local Fetch engine.
    pub fn with_services(
        config: Config,
        services: Arc<dyn FetchServices>,
    ) -> Result<Self, RequestError> {
        Self::build(config, Some(services))
    }

    fn build(
        config: Config,
        services: Option<Arc<dyn FetchServices>>,
    ) -> Result<Self, RequestError> {
        let max_in_flight = config.max_in_flight;
        let max_keepalive_body_size = config.max_keepalive_body_size;
        let transport = reqwest_transport(config.clone())
            .map_err(|error| RequestError::Initialization(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(ControllerInner {
                max_keepalive_body_size,
                cache: ResponseCache::default(),
                services,
                cors: CorsChecker,
                policy: RequestPolicy,
                scheduler: RequestScheduler::new(transport, max_in_flight),
            }),
        })
    }

    /// Removes all currently stored responses from this controller's cache.
    pub fn clear_cache(&self) {
        self.inner.cache.clear();
    }
}
