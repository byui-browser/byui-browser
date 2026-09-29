//! Public request orchestration boundary.

use std::sync::Arc;

use crate::{
    cache::ResponseCache,
    config::Config,
    cookies::CookieStore,
    cors::CorsChecker,
    error::RequestError,
    policy::RequestPolicy,
    request::{CacheMode, Request},
    response::Response,
    scheduler::{RequestPriority, RequestScheduler},
    transport::reqwest_transport,
};

/// Shared, stateful entry point for browser network requests.
///
/// A controller owns the process-local cache and scheduler while sharing the
/// connection pool held by its transport. Cloning a controller is therefore
/// inexpensive and preserves cache and connection reuse.
#[derive(Clone)]
pub struct RequestController {
    inner: Arc<ControllerInner>,
}

struct ControllerInner {
    cache: ResponseCache,
    cookies: CookieStore,
    cors: CorsChecker,
    policy: RequestPolicy,
    scheduler: RequestScheduler,
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
    /// transport-construction error is returned if the underlying HTTP client
    /// cannot be initialized.
    pub fn new(config: Config) -> Result<Self, reqwest::Error> {
        let transport = reqwest_transport(config.clone())?;
        Ok(Self {
            inner: Arc::new(ControllerInner {
                cache: ResponseCache::default(),
                cookies: CookieStore,
                cors: CorsChecker,
                policy: RequestPolicy,
                scheduler: RequestScheduler::new(transport, config.max_in_flight),
            }),
        })
    }

    /// Validates and executes one browser request.
    ///
    /// Cache hits return before a scheduler permit is acquired. Network-bound
    /// requests pass through policy validation, cookie attachment, scheduling,
    /// transport, response validation, cookie processing, and cache insertion.
    pub async fn fetch(&self, request: Request) -> Result<Response, RequestError> {
        self.inner.policy.validate_request(&request)?;

        let cacheable = request.is_cacheable_method();
        if cacheable
            && matches!(
                request.cache_mode,
                CacheMode::Default | CacheMode::OnlyIfCached
            )
        {
            if let Some(response) = self.inner.cache.get(&request) {
                return Ok(response);
            }
            if request.cache_mode == CacheMode::OnlyIfCached {
                return Err(RequestError::CacheMiss);
            }
        }

        // Cookie and policy modules operate before transport sees the request;
        // this keeps browser behavior out of the low-level HTTP implementation.
        let mut request = request;
        self.inner.cookies.attach(&mut request)?;
        // The scheduler owns concurrency admission. Transport remains focused
        // on HTTP I/O and connection pooling.
        let response = self
            .inner
            .scheduler
            .submit(request.clone(), RequestPriority::Normal)
            .await?;

        self.inner.cors.validate(&request, &response)?;
        self.inner.policy.validate_response(&request, &response)?;
        self.inner
            .cookies
            .process_response(&request, &response.headers)?;

        if cacheable && request.cache_mode != CacheMode::NoStore {
            self.inner.cache.insert(&request, &response);
        }
        Ok(response)
    }

    /// Removes all currently stored responses from this controller's cache.
    pub fn clear_cache(&self) {
        self.inner.cache.clear();
    }
}
