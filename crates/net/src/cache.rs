//! In-memory response cache and cache-policy helpers.

use std::{collections::HashMap, sync::{Arc, RwLock}, time::{Duration, Instant}};

use reqwest::{header::{HeaderMap, CACHE_CONTROL}, StatusCode};

use crate::{Request, Response};

/// Thread-safe process-local cache shared by cloned request controllers.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResponseCache {
    entries: Arc<RwLock<HashMap<String, CachedResponse>>>,
}

impl ResponseCache {
    /// Returns a fresh cached response for the request, if one exists.
    pub(crate) fn get(&self, request: &Request) -> Option<Response> {
        let key = cache_key(request);
        // Clone the response while the read lock is held, then release the lock
        // before returning so callers never hold cache state during network work.
        let entries = self.entries.read().expect("cache lock poisoned");
        let cached = entries.get(&key)?;
        (cached.expires_at > Instant::now()).then(|| cached.as_response())
    }

    /// Stores a successful response when its headers provide a positive TTL.
    pub(crate) fn insert(&self, request: &Request, response: &Response) {
        if !response.status.is_success() {
            return;
        }
        let Some(ttl) = cache_ttl(&response.headers) else {
            return;
        };
        self.entries.write().expect("cache lock poisoned").insert(
            cache_key(request),
            CachedResponse::from_response(response, ttl),
        );
    }

    /// Removes every response currently held by the cache.
    pub(crate) fn clear(&self) {
        self.entries.write().expect("cache lock poisoned").clear();
    }
}

/// Cached copy of a response and the time at which it becomes stale.
#[derive(Clone, Debug)]
pub(crate) struct CachedResponse {
    status: StatusCode,
    headers: HeaderMap,
    url: String,
    body: Vec<u8>,
    pub(crate) expires_at: Instant,
}

impl CachedResponse {
    /// Copies a response into the cache with a caller-supplied lifetime.
    pub(crate) fn from_response(response: &Response, ttl: Duration) -> Self {
        Self { 
            status: response.status.clone(), 
            headers: response.headers.clone(), 
            url: response.url.clone(), 
            body: response.body.clone(), 
            expires_at: Instant::now() + ttl 
        }
    }

    /// Reconstructs a public response and marks it as cache-served.
    pub(crate) fn as_response(&self) -> Response {
        Response { 
            status: self.status, 
            headers: self.headers.clone(), 
            url: self.url.clone(), 
            body: self.body.clone(), 
            from_cache: true 
        }
    }
}

/// Creates the cache key used to distinguish method and URL combinations.
pub(crate) fn cache_key(request: &Request) -> String {
    format!("{} {}", request.method, request.url)
}

/// Reads a conservative cache lifetime from the response's `Cache-Control` header.
/// Responses without `max-age`, or with `no-store`/`no-cache`, are not cached.
pub(crate) fn cache_ttl(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(CACHE_CONTROL)?.to_str().ok()?;
    if value.split(',').any(|directive| matches!(directive.trim().to_ascii_lowercase().as_str(), "no-store" | "no-cache")) { 
        return None; 
    }
    value.split(',').find_map(|directive| directive.trim().strip_prefix("max-age=")?.parse::<u64>().ok()).map(Duration::from_secs)
}
