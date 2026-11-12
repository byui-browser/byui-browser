//! Controller construction and shared engine state.

use std::sync::Arc;

use crate::{
    api::{
        config::Config,
        diagnostics::{NetworkEvent, NetworkPhase, SharedNetworkObserver},
        error::RequestError,
        providers::SchemeProviders,
        services::FetchServices,
    },
    cache::{PreflightCache, ResponseCache},
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
    observer: SharedNetworkObserver,
}

/// Shared controller state used by fetch execution and policy stages.
pub(crate) struct ControllerInner {
    /// Maximum accepted body size for keepalive requests.
    pub(crate) max_keepalive_body_size: u64,
    /// Maximum bytes retained while capturing one response for the cache.
    pub(crate) max_cache_body_size: usize,
    /// Process-local response cache shared by controller clones.
    pub(crate) cache: ResponseCache,
    /// Process-local CORS-preflight permission cache.
    pub(crate) preflight_cache: PreflightCache,
    /// Maximum lifetime accepted for one preflight permission.
    pub(crate) max_preflight_cache_age: std::time::Duration,
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
    /// Best-effort lifecycle observer, separate from browser policy services.
    pub(crate) observer: SharedNetworkObserver,
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
            observer: None,
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

    /// Installs a non-failing observer for request lifecycle diagnostics.
    ///
    /// The observer is shared by controller clones. It receives no bodies,
    /// headers, credentials, or error strings.
    pub fn with_network_observer(mut self, observer: Arc<dyn crate::NetworkObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Constructs shared state and initializes the reusable HTTP transport.
    pub fn build(self) -> Result<RequestController, RequestError> {
        let Self {
            config,
            services,
            providers,
            observer,
        } = self;
        let max_in_flight = config.max_in_flight;
        let max_keepalive_body_size = config.max_keepalive_body_size;
        let max_cache_body_size = config.max_cache_body_size;
        let max_preflight_cache_age = config.max_preflight_cache_age;
        let transport = reqwest_transport(config.clone()).map_err(|error| {
            log::error!("http transport initialization failed: {error}");
            RequestError::Initialization(error.to_string())
        })?;
        log::debug!(
            "request controller built: max_in_flight={max_in_flight}, \
             fetch_services={}, max_keepalive_body_size={max_keepalive_body_size}, max_cache_body_size={max_cache_body_size}",
            services.is_some()
        );
        Ok(RequestController {
            inner: Arc::new(ControllerInner {
                max_keepalive_body_size,
                max_cache_body_size,
                cache: ResponseCache::default(),
                preflight_cache: PreflightCache::default(),
                max_preflight_cache_age,
                services,
                providers,
                cors: CorsChecker,
                policy: RequestPolicy,
                scheduler: RequestScheduler::new(transport, max_in_flight, observer.clone()),
                observer,
            }),
        })
    }
}

impl RequestController {
    /// Emits a diagnostic event without allowing an observer to affect Fetch.
    pub(crate) fn observe(
        &self,
        request: &crate::Request,
        phase: NetworkPhase,
        url: &reqwest::Url,
        status: Option<u16>,
        from_cache: bool,
    ) {
        self.observe_parts(
            request.id(),
            phase,
            url.as_str(),
            status,
            from_cache,
            request.redirect_count,
        );
    }

    /// Emits an event after ownership of the request has moved into a scheme handler.
    pub(crate) fn observe_parts(
        &self,
        request_id: u64,
        phase: NetworkPhase,
        url: &str,
        status: Option<u16>,
        from_cache: bool,
        redirect_count: usize,
    ) {
        if let Some(observer) = &self.inner.observer {
            observer.observe(NetworkEvent::now(
                request_id,
                phase,
                url,
                status,
                from_cache,
                redirect_count,
            ));
        }
    }

    /// Removes all stored HTTP responses and CORS-preflight permissions.
    pub fn clear_cache(&self) {
        self.inner.cache.clear();
        self.inner.preflight_cache.clear();
    }
}
