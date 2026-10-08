//! In-memory response cache and cache-policy helpers.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use reqwest::{
    StatusCode, Url,
    header::{CACHE_CONTROL, HeaderMap},
};

use crate::{request::Request, response::Response};

/// Thread-safe process-local cache shared by cloned request clients.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResponseCache {
    entries: Arc<RwLock<HashMap<String, CachedResponse>>>,
}

impl ResponseCache {
    /// Returns a fresh cached response for the request, if one exists.
    pub(crate) fn get(&self, request: &Request, url: &Url) -> Option<Response> {
        let key = cache_key(request, url);
        // Clone the response while the read lock is held, then release the lock
        // before returning so callers never hold cache state during network work.
        let entries = self.entries.read().expect("cache lock poisoned");
        let cached = entries.get(&key)?;
        (cached.expires_at > Instant::now()).then(|| cached.as_response())
    }

    /// Stores a successful response when its headers provide a positive TTL.
    pub(crate) fn insert(&self, request: &Request, url: &Url, response: &Response) {
        if !response.status.is_success() {
            return;
        }
        let Some(ttl) = cache_ttl(&response.headers) else {
            return;
        };
        self.entries.write().expect("cache lock poisoned").insert(
            cache_key(request, url),
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
            status: response.status,
            headers: response.headers.clone(),
            url: response.url.clone(),
            body: response.body.clone(),
            expires_at: Instant::now() + ttl,
        }
    }

    /// Reconstructs a public response and marks it as cache-served.
    pub(crate) fn as_response(&self) -> Response {
        Response {
            status: self.status,
            headers: self.headers.clone(),
            url: self.url.clone(),
            body: self.body.clone(),
            from_cache: true,
        }
    }
}

/// Creates the cache key used to distinguish method and URL combinations.
pub(crate) fn cache_key(request: &Request, url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    format!("{} {url}", request.method)
}

/// Reads a conservative cache lifetime from the response's `Cache-Control` header.
/// Responses without `max-age`, or with `no-store`/`no-cache`, are not cached.
pub(crate) fn cache_ttl(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(CACHE_CONTROL)?.to_str().ok()?;
    if value.split(',').any(|directive| {
        matches!(
            directive.trim().to_ascii_lowercase().as_str(),
            "no-store" | "no-cache"
        )
    }) {
        return None;
    }
    value
        .split(',')
        .find_map(|directive| {
            let (name, value) = directive.trim().split_once('=')?;
            if !name.trim().eq_ignore_ascii_case("max-age") {
                return None;
            }
            value.trim().trim_matches('"').parse::<u64>().ok()
        })
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_control_parses_max_age_with_other_directives() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            "public, max-age=60, must-revalidate".parse().unwrap(),
        );

        assert_eq!(cache_ttl(&headers), Some(Duration::from_secs(60)));
    }

    #[test]
    fn cache_control_rejects_non_cacheable_directives() {
        for value in ["no-store", "public, no-cache, max-age=60"] {
            let mut headers = HeaderMap::new();
            headers.insert(CACHE_CONTROL, value.parse().unwrap());
            assert_eq!(cache_ttl(&headers), None, "directive: {value}");
        }
    }

    #[test]
    fn cache_control_accepts_case_insensitive_and_quoted_max_age() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            "PUBLIC, MAX-AGE = \"60\", must-revalidate".parse().unwrap(),
        );

        assert_eq!(cache_ttl(&headers), Some(Duration::from_secs(60)));
    }

    #[test]
    fn cache_does_not_store_non_success_responses() {
        let cache = ResponseCache::default();
        let request = Request::get("https://example.test/missing");
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        let response = Response {
            status: StatusCode::NOT_FOUND,
            headers,
            url: request.url.clone(),
            body: b"missing".to_vec(),
            from_cache: false,
        };

        let url = Url::parse(&request.url).unwrap();
        cache.insert(&request, &url, &response);

        assert!(cache.get(&request, &url).is_none());
    }

    #[test]
    fn cache_key_distinguishes_http_methods() {
        let get = Request::get("https://example.test/resource");
        let mut head = get.clone();
        head.method = reqwest::Method::HEAD;

        let url = Url::parse(&get.url).unwrap();
        assert_ne!(cache_key(&get, &url), cache_key(&head, &url));
    }

    #[test]
    fn cache_key_ignores_url_fragments() {
        let with_fragment = Request::get("https://example.test/resource#section");
        let without_fragment = Request::get("https://example.test/resource");
        let with_fragment_url = Url::parse(&with_fragment.url).unwrap();
        let without_fragment_url = Url::parse(&without_fragment.url).unwrap();

        assert_eq!(
            cache_key(&with_fragment, &with_fragment_url),
            cache_key(&without_fragment, &without_fragment_url)
        );
    }

    #[test]
    fn cache_key_uses_normalized_url_strings() {
        let uppercase = Request::get("HTTP://EXAMPLE.TEST/resource");
        let normalized = Request::get("http://example.test/resource");
        let uppercase_url = Url::parse(&uppercase.url).unwrap();
        let normalized_url = Url::parse(&normalized.url).unwrap();

        assert_eq!(
            cache_key(&uppercase, &uppercase_url),
            cache_key(&normalized, &normalized_url)
        );
    }

    #[test]
    fn expired_entries_are_not_returned() {
        let cache = ResponseCache::default();
        let request = Request::get("https://example.test/expired");
        let response = Response {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            url: request.url.clone(),
            body: b"stale".to_vec(),
            from_cache: false,
        };
        let cached = CachedResponse::from_response(&response, Duration::ZERO);

        cache.entries.write().unwrap().insert(
            cache_key(&request, &Url::parse(&request.url).unwrap()),
            cached,
        );

        assert!(
            cache
                .get(&request, &Url::parse(&request.url).unwrap())
                .is_none()
        );
    }
}
