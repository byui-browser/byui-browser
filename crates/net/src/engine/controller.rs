//! Controller construction and shared engine state.

use std::sync::Arc;

use crate::{
    api::{
        config::Config, error::RequestError, providers::SchemeProviders, services::FetchServices,
    },
    cache::ResponseCache,
    policy::{CorsChecker, RequestPolicy},
    scheduling::RequestScheduler,
    transport::reqwest_transport,
};

/// Shared, stateful entry point for browser network requests.
///
/// A controller owns the process-local cache and scheduler while sharing the
/// connection pool held by its transport. Cloning a controller is therefore
/// inexpensive and preserves cache and connection reuse. Use
/// [`RequestController::builder`] to compose optional browser services and
/// scheme providers.
#[derive(Clone)]
pub struct RequestController {
    pub(crate) inner: Arc<ControllerInner>,
}

/// Configures the optional capabilities used by a [`RequestController`].
///
/// A builder may omit both capabilities for standalone HTTP and engine-owned
/// local requests. Browser client context still requires
/// [`FetchServices`], and provider-backed local schemes still require their
/// matching [`SchemeProviders`] entry when a request is fetched. Building a
/// controller only initializes the shared HTTP transport; it does not invoke
/// browser services or scheme providers.
pub struct RequestControllerBuilder {
    config: Config,
    services: Option<Arc<dyn FetchServices>>,
    providers: SchemeProviders,
}

/// Shared controller state used by fetch execution and policy stages.
pub(crate) struct ControllerInner {
    /// Maximum accepted body size for keepalive requests.
    pub(crate) max_keepalive_body_size: u64,
    /// Process-local response cache shared by controller clones.
    pub(crate) cache: ResponseCache,
    /// Browser-owned policy, credential, cache-partition, and worker decisions.
    pub(crate) services: Option<Arc<dyn FetchServices>>,
    /// Browser-owned scheme providers selected by the dispatcher.
    pub(crate) providers: SchemeProviders,
    /// Cross-origin response validation boundary.
    pub(crate) cors: CorsChecker,
    /// Request and response policy validation boundary.
    pub(crate) policy: RequestPolicy,
    /// Shared transport admission and concurrency controller.
    pub(crate) scheduler: RequestScheduler,
}

impl std::fmt::Debug for RequestController {
    /// Formats the controller without exposing shared services or transport state.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestController").finish_non_exhaustive()
    }
}

impl RequestController {
    /// Starts construction of a Fetch-oriented controller.
    pub fn builder(config: Config) -> RequestControllerBuilder {
        RequestControllerBuilder {
            config,
            services: None,
            providers: SchemeProviders::default(),
        }
    }
}

impl RequestControllerBuilder {
    /// Installs browser-owned service decisions for requests carrying client
    /// context, credentials, storage, or service-worker state.
    ///
    /// The service is retained by the controller and shared by its clones.
    pub fn with_fetch_services(mut self, services: Arc<dyn FetchServices>) -> Self {
        self.services = Some(services);
        self
    }

    /// Installs browser-owned providers for `blob:`, `file:`, and `byui:`
    /// scheme requests.
    ///
    /// Providers are retained by the controller and selected by URL scheme at
    /// fetch time. Installing them does not invoke provider code.
    pub fn with_scheme_providers(mut self, providers: SchemeProviders) -> Self {
        self.providers = providers;
        self
    }

    /// Constructs shared state and initializes the reusable HTTP transport.
    pub fn build(self) -> Result<RequestController, RequestError> {
        let Self {
            config,
            services,
            providers,
        } = self;
        let max_in_flight = config.max_in_flight;
        let max_keepalive_body_size = config.max_keepalive_body_size;
        let transport = reqwest_transport(config.clone())
            .map_err(|error| RequestError::Initialization(error.to_string()))?;
        Ok(RequestController {
            inner: Arc::new(ControllerInner {
                max_keepalive_body_size,
                cache: ResponseCache::default(),
                services,
                providers,
                cors: CorsChecker,
                policy: RequestPolicy,
                scheduler: RequestScheduler::new(transport, max_in_flight),
            }),
        })
    }
}

impl RequestController {
    /// Removes all currently stored responses from this controller's cache.
    pub fn clear_cache(&self) {
        self.inner.cache.clear();
    }
}
