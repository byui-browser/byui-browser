//! Controller construction and shared engine state.

use std::sync::Arc;

use crate::{
    api::{
        config::Config,
        diagnostics::{NetworkEvent, NetworkPhase, SharedNetworkObserver},
        error::RequestError,
        providers::SchemeProviders,
        services::{
            CachePartitionService, CookieService, RequestPolicyService, ResponsePolicyService,
            ServiceWorkerService,
        },
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
/// A builder may omit browser capabilities for standalone HTTP and
/// engine-owned local requests. Browser client context requires the applicable
/// focused capabilities, and provider-backed local schemes require their
/// matching [`SchemeProviders`] entry when a request is fetched. Building a
/// controller only initializes the shared HTTP transport; it does not invoke
/// browser capabilities or scheme providers.
pub struct RequestControllerBuilder {
    config: Config,
    request_policy: Option<Arc<dyn RequestPolicyService>>,
    response_policy: Option<Arc<dyn ResponsePolicyService>>,
    cookies: Option<Arc<dyn CookieService>>,
    service_workers: Option<Arc<dyn ServiceWorkerService>>,
    cache_partitions: Option<Arc<dyn CachePartitionService>>,
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
    /// Browser-owned request authorization and generated headers.
    pub(crate) request_policy: Option<Arc<dyn RequestPolicyService>>,
    /// Browser-owned response authorization and diagnostics.
    pub(crate) response_policy: Option<Arc<dyn ResponsePolicyService>>,
    /// Browser-owned HTTP cookie selection and storage.
    pub(crate) cookies: Option<Arc<dyn CookieService>>,
    /// Browser-owned initial service-worker interception decision.
    pub(crate) service_workers: Option<Arc<dyn ServiceWorkerService>>,
    /// Browser-owned HTTP cache partition selection.
    pub(crate) cache_partitions: Option<Arc<dyn CachePartitionService>>,
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
            request_policy: None,
            response_policy: None,
            cookies: None,
            service_workers: None,
            cache_partitions: None,
            providers: SchemeProviders::default(),
            observer: None,
        }
    }
}

impl RequestControllerBuilder {
    /// Installs one value that implements every browser-owned Fetch capability.
    ///
    /// Call the focused setters instead when capabilities have different
    /// owners or lifetimes.
    pub fn with_browser_capabilities<T>(mut self, capabilities: Arc<T>) -> Self
    where
        T: RequestPolicyService
            + ResponsePolicyService
            + CookieService
            + ServiceWorkerService
            + CachePartitionService
            + 'static,
    {
        self.request_policy = Some(capabilities.clone());
        self.response_policy = Some(capabilities.clone());
        self.cookies = Some(capabilities.clone());
        self.service_workers = Some(capabilities.clone());
        self.cache_partitions = Some(capabilities);
        self
    }

    /// Installs browser-owned request authorization and generated headers.
    pub fn with_request_policy(mut self, policy: Arc<dyn RequestPolicyService>) -> Self {
        self.request_policy = Some(policy);
        self
    }

    /// Installs browser-owned response authorization and diagnostics.
    pub fn with_response_policy(mut self, policy: Arc<dyn ResponsePolicyService>) -> Self {
        self.response_policy = Some(policy);
        self
    }

    /// Installs browser-owned HTTP cookie selection and storage.
    pub fn with_cookie_service(mut self, cookies: Arc<dyn CookieService>) -> Self {
        self.cookies = Some(cookies);
        self
    }

    /// Installs the browser-owned initial service-worker decision capability.
    pub fn with_service_worker_service(
        mut self,
        service_workers: Arc<dyn ServiceWorkerService>,
    ) -> Self {
        self.service_workers = Some(service_workers);
        self
    }

    /// Installs browser-owned HTTP cache partition selection.
    pub fn with_cache_partition_service(
        mut self,
        cache_partitions: Arc<dyn CachePartitionService>,
    ) -> Self {
        self.cache_partitions = Some(cache_partitions);
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
            request_policy,
            response_policy,
            cookies,
            service_workers,
            cache_partitions,
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
             browser_capabilities={}, max_keepalive_body_size={max_keepalive_body_size}, max_cache_body_size={max_cache_body_size}",
            [
                request_policy.is_some(),
                response_policy.is_some(),
                cookies.is_some(),
                service_workers.is_some(),
                cache_partitions.is_some(),
            ]
            .into_iter()
            .filter(|installed| *installed)
            .count()
        );
        Ok(RequestController {
            inner: Arc::new(ControllerInner {
                max_keepalive_body_size,
                max_cache_body_size,
                cache: ResponseCache::default(),
                preflight_cache: PreflightCache::default(),
                max_preflight_cache_age,
                request_policy,
                response_policy,
                cookies,
                service_workers,
                cache_partitions,
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
            request.redirect_count(),
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
