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

use crate::api::request::Request;

/// Complete buffered response retained for cache policy and later filtering.
#[derive(Clone, Debug)]
pub(crate) struct StoredResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) url: String,
    pub(crate) body: Vec<u8>,
    pub(crate) cookie_headers_processed: bool,
}

/// Thread-safe process-local cache shared by cloned request clients.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResponseCache {
    entries: Arc<RwLock<HashMap<String, Vec<CachedResponse>>>>,
}

impl ResponseCache {
    /// Returns a fresh cached response for the request, if one exists.
    #[cfg(test)]
    pub(crate) fn get(&self, request: &Request, url: &Url) -> Option<StoredResponse> {
        self.get_with_staleness(request, url, false)
    }

    /// Returns a cached response, optionally allowing an expired entry.
    pub(crate) fn get_with_staleness(
        &self,
        request: &Request,
        url: &Url,
        allow_stale: bool,
    ) -> Option<StoredResponse> {
        let key = cache_key(request, url);
        // Clone the response while the read lock is held, then release the lock
        // before returning so callers never hold cache state during network work.
        let entries = self.entries.read().expect("cache lock poisoned");
        let Some(cached) = entries.get(&key) else {
            log::trace!(
                "cache lookup found no entry for {} {}",
                request.method,
                crate::log_url::loggable(url.as_str())
            );
            return None;
        };

        // Find the cached response that matches this request's Vary headers.
        let Some(cached) = cached.iter().find(|entry| {
            entry
                .vary_headers
                .iter()
                .all(|(name, original_value)| request.headers.get(name) == *original_value)
        }) else {
            log::trace!("cache miss: no matching Vary response");
            return None;
        };
        let usable = allow_stale || cached.expires_at > Instant::now();
        if !usable {
            log::trace!(
                "cache entry expired for {} {}",
                request.method,
                crate::log_url::loggable(url.as_str())
            );
        }
        usable.then(|| cached.as_response())
    }

    /// Stores a successful response when its headers provide a positive TTL.
    pub(crate) fn insert(&self, request: &Request, url: &Url, response: &StoredResponse) {
        if !response.status.is_success() {
            log::trace!(
                "cache store skipped for {}: status {} is not successful",
                crate::log_url::loggable(url.as_str()),
                response.status
            );
            return;
        }
        // Vary: * means the response cannot be reused from cache.
        if vary_header_names(&response.headers)
            .iter()
            .any(|name| name == "*")
        {
            log::trace!(
                "cache store skipped for {}: Vary header contains *",
                crate::log_url::loggable(url.as_str())
            );
            return;
        }
        let Some(ttl) = cache_ttl(&response.headers) else {
            log::trace!(
                "cache store skipped for {}: no usable max-age",
                crate::log_url::loggable(url.as_str())
            );
            return;
        };
        log::debug!(
            "cache store: {} {} ({} bytes, ttl {}s)",
            request.method,
            crate::log_url::loggable(url.as_str()),
            response.body.len(),
            ttl.as_secs()
        );
        let key = cache_key(request, url);
        let cached_response = CachedResponse::from_response(request, response, ttl);

        let mut entries = self.entries.write().expect("cache lock poisoned");
        let variants = entries.entry(key).or_default();

        // Remove cached responses that use a different set of Vary headers.
        variants.retain(|existing| {
            let existing_names: Vec<_> =
                existing.vary_headers.iter().map(|(name, _)| name).collect();

            let new_names: Vec<_> = cached_response
                .vary_headers
                .iter()
                .map(|(name, _)| name)
                .collect();

            existing_names == new_names
        });

        variants.retain(|existing| existing.vary_headers != cached_response.vary_headers);

        variants.push(cached_response);
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
    cookie_headers_processed: bool,
    pub(crate) expires_at: Instant,
    vary_headers: Vec<(String, Option<String>)>,
}

impl CachedResponse {
    /// Copies a response into the cache with a caller-supplied lifetime.
    pub(crate) fn from_response(
        request: &Request,
        response: &StoredResponse,
        ttl: Duration,
    ) -> Self {
        Self {
            status: response.status,
            headers: response.headers.clone(),
            url: response.url.clone(),
            body: response.body.clone(),
            cookie_headers_processed: response.cookie_headers_processed,
            expires_at: Instant::now() + ttl,
            vary_headers: capture_vary_headers(request, &response.headers),
        }
    }

    /// Reconstructs a public response and marks it as cache-served.
    pub(crate) fn as_response(&self) -> StoredResponse {
        StoredResponse {
            status: self.status,
            headers: self.headers.clone(),
            url: self.url.clone(),
            body: self.body.clone(),
            cookie_headers_processed: self.cookie_headers_processed,
        }
    }
}

/// Creates the cache key used to distinguish method and URL combinations.
pub(crate) fn cache_key(request: &Request, url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    match request.cache_partition.as_deref() {
        Some(partition) => format!(
            "browser:{}:{partition} {} {url}",
            partition.len(),
            request.method
        ),
        None => format!("standalone {} {url}", request.method),
    }
}

/// Returns the request header names listed in the response's Vary header.
fn vary_header_names(headers: &HeaderMap) -> Vec<String> {
    let mut names: Vec<String> = headers
        .get_all("vary")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();

    names.sort();
    names.dedup();

    names
}

/// Captures the original request values for headers listed in Vary.
fn capture_vary_headers(
    request: &Request,
    response_headers: &HeaderMap,
) -> Vec<(String, Option<String>)> {
    vary_header_names(response_headers)
        .into_iter()
        .map(|name| {
            let value = request.headers.get(&name);
            (name, value)
        })
        .collect()
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
    /// Verifies max-age is parsed alongside other cache directives.
    fn cache_control_parses_max_age_with_other_directives() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            "public, max-age=60, must-revalidate".parse().unwrap(),
        );

        assert_eq!(cache_ttl(&headers), Some(Duration::from_secs(60)));
    }

    #[test]
    fn vary_header_matches_original_request() {
        let cache = ResponseCache::default();
        let url = Url::parse("https://example.test/page").unwrap();

        // Original request asks for English.
        let mut english = Request::get(url.as_str());
        english.headers.set_str("Accept-Language", "en").unwrap();

        // Server says the response varies by language.
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        headers.insert("vary", "Accept-Language".parse().unwrap());

        let response = StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: b"Hello".to_vec(),
            cookie_headers_processed: false,
        };

        cache.insert(&english, &url, &response);

        // Same language should produce a cache hit.
        assert!(cache.get(&english, &url).is_some());

        // Different language should produce a cache miss.
        let mut spanish = Request::get(url.as_str());
        spanish.headers.set_str("Accept-Language", "es").unwrap();

        assert!(cache.get(&spanish, &url).is_none());
    }

    #[test]
    fn vary_star_responses_are_not_cached() {
        let cache = ResponseCache::default();
        let url = Url::parse("https://example.test/page").unwrap();
        let request = Request::get(url.as_str());

        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        headers.insert("vary", "*".parse().unwrap());

        let response = StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: b"Hello".to_vec(),
            cookie_headers_processed: false,
        };

        cache.insert(&request, &url, &response);

        // Vary: * responses must not be stored.
        assert!(cache.get(&request, &url).is_none());
    }

    #[test]
    /// Verifies non-cacheable directives prevent insertion.
    fn cache_control_rejects_non_cacheable_directives() {
        for value in ["no-store", "public, no-cache, max-age=60"] {
            let mut headers = HeaderMap::new();
            headers.insert(CACHE_CONTROL, value.parse().unwrap());
            assert_eq!(cache_ttl(&headers), None, "directive: {value}");
        }
    }

    #[test]
    /// Verifies directive names and quoted max-age values are accepted.
    fn cache_control_accepts_case_insensitive_and_quoted_max_age() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            "PUBLIC, MAX-AGE = \"60\", must-revalidate".parse().unwrap(),
        );

        assert_eq!(cache_ttl(&headers), Some(Duration::from_secs(60)));
    }

    #[test]
    /// Verifies only successful responses enter the cache.
    fn cache_does_not_store_non_success_responses() {
        let cache = ResponseCache::default();
        let request = Request::get("https://example.test/missing");
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        let response = StoredResponse {
            status: StatusCode::NOT_FOUND,
            headers,
            url: request.url.clone(),
            body: b"missing".to_vec(),
            cookie_headers_processed: false,
        };

        let url = Url::parse(&request.url).unwrap();
        cache.insert(&request, &url, &response);

        assert!(cache.get(&request, &url).is_none());
    }

    #[test]
    /// Verifies cache keys distinguish request methods.
    fn cache_key_distinguishes_http_methods() {
        let get = Request::get("https://example.test/resource");
        let mut head = get.try_clone().unwrap();
        head.method = reqwest::Method::HEAD;

        let url = Url::parse(&get.url).unwrap();
        assert_ne!(cache_key(&get, &url), cache_key(&head, &url));
    }

    #[test]
    /// Verifies browser cache partitions remain isolated.
    fn browser_cache_partitions_do_not_share_entries() {
        let url = Url::parse("https://example.test/resource").unwrap();
        let mut first = Request::get(url.as_str());
        first.cache_partition = Some("site-a".into());
        let mut second = first.try_clone().unwrap();
        second.cache_partition = Some("site-b".into());
        assert_ne!(cache_key(&first, &url), cache_key(&second, &url));
        assert_ne!(
            cache_key(&first, &url),
            cache_key(&Request::get(url.as_str()), &url)
        );
    }

    #[test]
    /// Verifies URL fragments do not affect cache identity.
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
    /// Verifies equivalent normalized URLs share a cache key.
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
    /// Verifies expired entries are treated as cache misses.
    fn expired_entries_are_not_returned() {
        let cache = ResponseCache::default();
        let request = Request::get("https://example.test/expired");
        let response = StoredResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            url: request.url.clone(),
            body: b"stale".to_vec(),
            cookie_headers_processed: false,
        };
        let cached = CachedResponse::from_response(&request, &response, Duration::ZERO);

        cache.entries.write().unwrap().insert(
            cache_key(&request, &Url::parse(&request.url).unwrap()),
            vec![cached],
        );

        assert!(
            cache
                .get(&request, &Url::parse(&request.url).unwrap())
                .is_none()
        );
    }

    #[test]
    fn expired_entries_are_not_returned2() {
        let cache = ResponseCache::default();
        let request = Request::get("https://example.test/expired2");
        let response = StoredResponse {
            status: StatusCode::FOUND,
            headers: HeaderMap::new(),
            url: request.url.clone(),
            body: b"stale".to_vec(),
            cookie_headers_processed: false,
        };
        let cached = CachedResponse::from_response(&request, &response, Duration::ZERO);

        cache.entries.write().unwrap().insert(
            cache_key(&request, &Url::parse(&request.url).unwrap()),
            vec![cached],
        );

        assert!(
            cache
                .get(&request, &Url::parse(&request.url).unwrap())
                .is_none()
        );
    }

    #[test]
    fn multiple_vary_variants_are_cached() {
        let cache = ResponseCache::default();
        let url = Url::parse("https://example.test/page").unwrap();

        // Create requests for two different languages.
        let mut english = Request::get(url.as_str());
        english.headers.set_str("Accept-Language", "en").unwrap();

        let mut spanish = Request::get(url.as_str());
        spanish.headers.set_str("Accept-Language", "es").unwrap();

        // Both responses vary by the Accept-Language header.
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        headers.insert("vary", "Accept-Language".parse().unwrap());

        let english_response = StoredResponse {
            status: StatusCode::OK,
            headers: headers.clone(),
            url: url.to_string(),
            body: b"Hello".to_vec(),
            cookie_headers_processed: false,
        };

        let spanish_response = StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: b"Hola".to_vec(),
            cookie_headers_processed: false,
        };

        // Store both responses under the same URL.
        cache.insert(&english, &url, &english_response);
        cache.insert(&spanish, &url, &spanish_response);

        // Each language should retrieve its own response.
        assert_eq!(cache.get(&english, &url).unwrap().body, b"Hello");
        assert_eq!(cache.get(&spanish, &url).unwrap().body, b"Hola");

        // Update the English response with new content.
        let updated_english_response = StoredResponse {
            body: b"Hello again".to_vec(),
            ..english_response.clone()
        };

        cache.insert(&english, &url, &updated_english_response);

        // English should return the updated response.
        assert_eq!(cache.get(&english, &url).unwrap().body, b"Hello again");

        // Spanish should still return its original response.
        assert_eq!(cache.get(&spanish, &url).unwrap().body, b"Hola");
    }

    #[test]
    fn vary_header_order_does_not_matter() {
        let mut first = HeaderMap::new();
        first.insert("vary", "Accept-Language, Accept-Encoding".parse().unwrap());

        let mut second = HeaderMap::new();
        second.insert("vary", "Accept-Encoding, Accept-Language".parse().unwrap());

        assert_eq!(vary_header_names(&first), vary_header_names(&second));
    }

    #[test]
    fn changing_vary_rules_removes_old_variants() {
        let cache = ResponseCache::default();
        let url = Url::parse("https://example.test/page").unwrap();

        let mut request = Request::get(url.as_str());
        request.headers.set_str("Accept-Language", "en").unwrap();
        request.headers.set_str("Accept", "text/html").unwrap();

        let mut language_headers = HeaderMap::new();
        language_headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        language_headers.insert("vary", "Accept-Language".parse().unwrap());

        let language_response = StoredResponse {
            status: StatusCode::OK,
            headers: language_headers,
            url: url.to_string(),
            body: b"English response".to_vec(),
            cookie_headers_processed: false,
        };

        cache.insert(&request, &url, &language_response);

        let mut encoding_headers = HeaderMap::new();
        encoding_headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        encoding_headers.insert("vary", "Accept".parse().unwrap());

        let encoding_response = StoredResponse {
            headers: encoding_headers,
            body: b"Gzip response".to_vec(),
            ..language_response.clone()
        };

        cache.insert(&request, &url, &encoding_response);

        // The cache should contain only the new Vary rule.
        let key = cache_key(&request, &url);
        let entries = cache.entries.read().unwrap();
        let variants = entries.get(&key).unwrap();

        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].body, b"Gzip response");
    }
}
