//! Request execution, redirect, connection-pool, and cache orchestration.

use reqwest::{Client, Method};

use crate::{cache::ResponseCache, CacheMode, Config, Request, RequestError, Response};

/// Executes browser HTTP requests using one reusable client and shared cache.
#[derive(Clone)]
pub struct RequestController {
    client: Client,
    cache: ResponseCache,
}

impl std::fmt::Debug for RequestController {
    /// Formats the controller without exposing its client or cache internals.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestController").finish_non_exhaustive()
    }
}

impl RequestController {
    /// Creates a controller using [`Config::default`].
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::with_config(Config::default())
    }

    /// Creates a controller with explicit redirect and connection-pool settings.
    pub fn with_config(config: Config) -> Result<Self, reqwest::Error> {
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::limited(config.max_redirects))
            .pool_idle_timeout(config.pool_idle_timeout)
            .pool_max_idle_per_host(config.pool_max_idle_per_host);
        if let Some(user_agent) = config.user_agent {
            builder = builder.user_agent(user_agent);
        }

        Ok(Self { client: builder.build()?, cache: ResponseCache::default() })
    }

    /// Executes a request, following configured redirects and reusing pooled connections.
    pub async fn execute(&self, request: Request) -> Result<Response, RequestError> {
        // Parse URL before consulting the cache so invalid input fails consistently.
        let url = reqwest::Url::parse(&request.url)
            .map_err(|_| RequestError::InvalidUrl(request.url.clone()))?;
        let cacheable = matches!(request.method, Method::GET | Method::HEAD);
        let can_read_cache = cacheable && matches!(request.cache_mode, CacheMode::Default | CacheMode::OnlyIfCached);

        if can_read_cache {
            if let Some(cached) = self.cache.get(&request) {
                return Ok(cached);
            }
            if request.cache_mode == CacheMode::OnlyIfCached {
                return Err(RequestError::CacheMiss);
            }
        }

        // Keep `request` available for cache insertion after the network call.
        // The reqwest builder owns the values passed into it, so clone the
        // request components rather than partially moving `request`.
        let mut builder = self
            .client
            .request(request.method.clone(), url)
            .headers(request.headers.clone());
        if let Some(body) = request.body.as_ref() {
            builder = builder.body(body.clone());
        }
        // reqwest follows redirects and manages connection reuse through this client.
        let response = builder.send().await?;
        let result = Response { 
            status: response.status(), 
            headers: response.headers().clone(), 
            url: response.url().to_string(), 
            body: response.bytes().await?.to_vec(), 
            from_cache: false 
        };

        if cacheable && request.cache_mode != CacheMode::NoStore {
            // Only responses with an explicit positive freshness lifetime are cached.
            self.cache.insert(&request, &result);
        }
        Ok(result)
    }

    /// Removes all entries from this controller's in-memory cache.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}
