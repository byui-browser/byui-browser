//! In-memory response cache and cache-policy helpers.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use bytes::Bytes;
use reqwest::{
    StatusCode, Url,
    header::{AUTHORIZATION, CACHE_CONTROL, HeaderMap, PROXY_AUTHORIZATION},
};

use crate::api::request::{CredentialsMode, RedirectTaint, Request, RequestMode};

/// Complete buffered response retained for cache policy and later filtering.
#[derive(Clone, Debug)]
pub(crate) struct StoredResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) url: String,
    /// Immutable buffered bytes. Cloning this value shares the allocation so a
    /// cache hit can create an independent body reader without copying it.
    pub(crate) body: Bytes,
    pub(crate) cookie_headers_processed: bool,
}

/// Thread-safe, bounded process-local cache shared by cloned request clients.
#[derive(Clone, Debug)]
pub(crate) struct ResponseCache {
    entries: Arc<RwLock<CacheEntries>>,
    limits: CacheLimits,
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new(256, 64 * 1024 * 1024)
    }
}

impl ResponseCache {
    /// Creates a bounded process-local response cache.
    pub(crate) fn new(max_entries: usize, max_body_size: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(CacheEntries::default())),
            limits: CacheLimits {
                max_entries,
                max_body_size,
            },
        }
    }
}

/// Limits enforced across all partitions held by one response cache.
#[derive(Clone, Copy, Debug)]
struct CacheLimits {
    max_entries: usize,
    max_body_size: usize,
}

/// Mutable cache state protected by the response-cache lock.
#[derive(Debug, Default)]
struct CacheEntries {
    entries: HashMap<CacheKey, Vec<CachedResponse>>,
    total_body_size: usize,
    next_access: u64,
}

/// Structured identity for one cacheable request representation.
///
/// Authorization values are deliberately not retained. Authorized requests
/// are currently excluded from storage until complete authorization caching is
/// implemented, while this marker prevents any future lookup from sharing an
/// unauthenticated representation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CacheKey {
    partition: CachePartition,
    method: http::Method,
    url: String,
    credentials_mode: CredentialsMode,
    credentials_allowed: bool,
    request_mode: RequestMode,
    redirect_taint: RedirectTaint,
    has_authorization: bool,
}

/// Browser-provided or standalone storage scope for a cache key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum CachePartition {
    Browser(String),
    Standalone,
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
        let mut entries = self.entries.write().expect("cache lock poisoned");
        let access = entries.next_access();
        let Some(cached) = entries.entries.get_mut(&key) else {
            log::trace!(
                "cache lookup found no entry for {} {}",
                request.method(),
                crate::log_url::loggable(url.as_str())
            );
            return None;
        };

        // Find the cached response that matches this request's Vary headers.
        let Some(cached) = cached.iter_mut().find(|entry| {
            entry
                .vary_headers
                .iter()
                .all(|(name, original_value)| request.headers().get(name) == *original_value)
        }) else {
            log::trace!("cache miss: no matching Vary response");
            return None;
        };
        let usable = allow_stale || cached.expires_at > Instant::now();
        if !usable {
            log::trace!(
                "cache entry expired for {} {}",
                request.method(),
                crate::log_url::loggable(url.as_str())
            );
        }
        if usable {
            cached.last_access = access;
            Some(cached.as_response())
        } else {
            None
        }
    }

    /// Stores a successful response when its headers provide a positive TTL.
    pub(crate) fn insert(&self, request: &Request, url: &Url, response: StoredResponse) {
        if !request.is_cacheable_method() || has_authorization(request) {
            log::trace!(
                "cache store skipped for {}: method or authorization is not cacheable",
                crate::log_url::loggable(url.as_str())
            );
            return;
        }
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
            request.method(),
            crate::log_url::loggable(url.as_str()),
            response.body.len(),
            ttl.as_secs()
        );
        let key = cache_key(request, url);
        let body_size = response.body.len();
        if self.limits.max_entries == 0
            || self.limits.max_body_size == 0
            || body_size > self.limits.max_body_size
        {
            log::trace!("cache store skipped: cache limits do not admit response");
            return;
        }

        let mut entries = self.entries.write().expect("cache lock poisoned");
        entries.prune_expired();
        let access = entries.next_access();
        let cached_response = CachedResponse::from_response(request, response, ttl, access);
        let new_vary_names = cached_response.vary_names();
        let replaced_body_size = entries.body_size_for(&key);
        let existing = entries.entries.remove(&key).unwrap_or_default();
        let retained: Vec<_> = existing
            .into_iter()
            .filter(|entry| {
                entry.vary_names() == new_vary_names
                    && entry.vary_headers != cached_response.vary_headers
            })
            .collect();
        entries.total_body_size = entries.total_body_size.saturating_sub(replaced_body_size);
        entries.total_body_size = entries
            .total_body_size
            .saturating_add(
                retained
                    .iter()
                    .map(CachedResponse::body_size)
                    .sum::<usize>(),
            )
            .saturating_add(body_size);
        let mut variants = retained;
        variants.push(cached_response);
        entries.entries.insert(key, variants);
        entries.evict_to_limits(self.limits);
    }

    /// Removes every response currently held by the cache.
    pub(crate) fn clear(&self) {
        *self.entries.write().expect("cache lock poisoned") = CacheEntries::default();
    }
}

/// Cache metadata paired with the one owned stored response representation.
///
/// `StoredResponse` is moved into this entry on insertion. This intentionally
/// avoids a second header/body representation while the cache retains it.
#[derive(Clone, Debug)]
pub(crate) struct CachedResponse {
    response: StoredResponse,
    pub(crate) expires_at: Instant,
    vary_headers: Vec<(String, Option<String>)>,
    last_access: u64,
}

impl CachedResponse {
    /// Moves a response into the cache with a caller-supplied lifetime.
    pub(crate) fn from_response(
        request: &Request,
        response: StoredResponse,
        ttl: Duration,
        last_access: u64,
    ) -> Self {
        let vary_headers = capture_vary_headers(request, &response.headers);
        Self {
            response,
            expires_at: Instant::now() + ttl,
            vary_headers,
            last_access,
        }
    }

    /// Reconstructs a response for finalization.
    ///
    /// Headers remain independently owned for response filtering, while the
    /// immutable body bytes share their backing allocation with the cache.
    pub(crate) fn as_response(&self) -> StoredResponse {
        self.response.clone()
    }

    fn body_size(&self) -> usize {
        self.response.body.len()
    }

    fn vary_names(&self) -> Vec<&str> {
        self.vary_headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect()
    }
}

/// Creates the cache key used to distinguish method and URL combinations.
pub(crate) fn cache_key(request: &Request, url: &Url) -> CacheKey {
    let mut url = url.clone();
    url.set_fragment(None);
    CacheKey {
        partition: match request.cache_partition() {
            Some(partition) => CachePartition::Browser(partition.to_owned()),
            None => CachePartition::Standalone,
        },
        method: request.method().clone(),
        url: url.to_string(),
        credentials_mode: request.credentials_mode(),
        credentials_allowed: request.credentials_allowed(&url),
        request_mode: request.mode(),
        redirect_taint: request.redirect_taint(),
        has_authorization: has_authorization(request),
    }
}

/// Reports whether a request carries an authorization-bearing header.
fn has_authorization(request: &Request) -> bool {
    request.headers().has(AUTHORIZATION.as_str())
        || request.headers().has(PROXY_AUTHORIZATION.as_str())
}

impl CacheEntries {
    fn next_access(&mut self) -> u64 {
        let access = self.next_access;
        self.next_access = self.next_access.wrapping_add(1);
        access
    }

    fn body_size_for(&self, key: &CacheKey) -> usize {
        self.entries
            .get(key)
            .into_iter()
            .flatten()
            .map(CachedResponse::body_size)
            .sum()
    }

    /// Drops expired data before an insertion; stale data may still be served
    /// by `ForceCache` until a later insertion makes it eligible for eviction.
    fn prune_expired(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, variants| {
            variants.retain(|entry| entry.expires_at > now);
            !variants.is_empty()
        });
        self.total_body_size = self
            .entries
            .values()
            .flatten()
            .map(CachedResponse::body_size)
            .sum();
    }

    fn evict_to_limits(&mut self, limits: CacheLimits) {
        while self.entries.len() > limits.max_entries || self.total_body_size > limits.max_body_size
        {
            let Some(key) = self
                .entries
                .iter()
                .min_by_key(|(_, variants)| {
                    variants
                        .iter()
                        .map(|entry| entry.last_access)
                        .max()
                        .unwrap_or_default()
                })
                .map(|(key, _)| key.clone())
            else {
                return;
            };
            if let Some(variants) = self.entries.remove(&key) {
                self.total_body_size = self
                    .total_body_size
                    .saturating_sub(variants.iter().map(CachedResponse::body_size).sum());
            }
        }
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
            let value = request.headers().get(&name);
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
        english
            .set_header("Accept-Language".parse().unwrap(), "en".parse().unwrap())
            .unwrap();

        // Server says the response varies by language.
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        headers.insert("vary", "Accept-Language".parse().unwrap());

        let response = StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from_static(b"Hello"),
            cookie_headers_processed: false,
        };

        cache.insert(&english, &url, response);

        // Same language should produce a cache hit.
        assert!(cache.get(&english, &url).is_some());

        // Different language should produce a cache miss.
        let mut spanish = Request::get(url.as_str());
        spanish
            .set_header("Accept-Language".parse().unwrap(), "es".parse().unwrap())
            .unwrap();

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
            body: Bytes::from_static(b"Hello"),
            cookie_headers_processed: false,
        };

        cache.insert(&request, &url, response);

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
            url: request.url().to_owned(),
            body: Bytes::from_static(b"missing"),
            cookie_headers_processed: false,
        };

        let url = Url::parse(request.url()).unwrap();
        cache.insert(&request, &url, response);

        assert!(cache.get(&request, &url).is_none());
    }

    #[test]
    /// Verifies cache keys distinguish request methods.
    fn cache_key_distinguishes_http_methods() {
        let get = Request::get("https://example.test/resource");
        let mut head = get.try_clone().unwrap();
        head.set_method("HEAD").unwrap();

        let url = Url::parse(get.url()).unwrap();
        assert_ne!(cache_key(&get, &url), cache_key(&head, &url));
    }

    #[test]
    /// Verifies browser cache partitions remain isolated.
    fn browser_cache_partitions_do_not_share_entries() {
        let url = Url::parse("https://example.test/resource").unwrap();
        let mut first = Request::get(url.as_str());
        first.set_cache_partition("site-a".into());
        let mut second = first.try_clone().unwrap();
        second.set_cache_partition("site-b".into());
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
        let with_fragment_url = Url::parse(with_fragment.url()).unwrap();
        let without_fragment_url = Url::parse(without_fragment.url()).unwrap();

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
        let uppercase_url = Url::parse(uppercase.url()).unwrap();
        let normalized_url = Url::parse(normalized.url()).unwrap();

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
            url: request.url().to_owned(),
            body: Bytes::from_static(b"stale"),
            cookie_headers_processed: false,
        };
        let cached = CachedResponse::from_response(&request, response, Duration::ZERO, 0);

        cache.entries.write().unwrap().entries.insert(
            cache_key(&request, &Url::parse(request.url()).unwrap()),
            vec![cached],
        );

        assert!(
            cache
                .get(&request, &Url::parse(request.url()).unwrap())
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
            url: request.url().to_owned(),
            body: Bytes::from_static(b"stale"),
            cookie_headers_processed: false,
        };
        let cached = CachedResponse::from_response(&request, response, Duration::ZERO, 0);

        cache.entries.write().unwrap().entries.insert(
            cache_key(&request, &Url::parse(request.url()).unwrap()),
            vec![cached],
        );

        assert!(
            cache
                .get(&request, &Url::parse(request.url()).unwrap())
                .is_none()
        );
    }

    #[test]
    fn multiple_vary_variants_are_cached() {
        let cache = ResponseCache::default();
        let url = Url::parse("https://example.test/page").unwrap();

        // Create requests for two different languages.
        let mut english = Request::get(url.as_str());
        english
            .set_header("Accept-Language".parse().unwrap(), "en".parse().unwrap())
            .unwrap();

        let mut spanish = Request::get(url.as_str());
        spanish
            .set_header("Accept-Language".parse().unwrap(), "es".parse().unwrap())
            .unwrap();

        // Both responses vary by the Accept-Language header.
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        headers.insert("vary", "Accept-Language".parse().unwrap());

        let english_response = StoredResponse {
            status: StatusCode::OK,
            headers: headers.clone(),
            url: url.to_string(),
            body: Bytes::from_static(b"Hello"),
            cookie_headers_processed: false,
        };

        let spanish_response = StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from_static(b"Hola"),
            cookie_headers_processed: false,
        };

        // Store both responses under the same URL.
        cache.insert(&english, &url, english_response.clone());
        cache.insert(&spanish, &url, spanish_response);

        // Each language should retrieve its own response.
        assert_eq!(cache.get(&english, &url).unwrap().body.as_ref(), b"Hello");
        assert_eq!(cache.get(&spanish, &url).unwrap().body.as_ref(), b"Hola");

        // Update the English response with new content.
        let updated_english_response = StoredResponse {
            body: Bytes::from_static(b"Hello again"),
            ..english_response.clone()
        };

        cache.insert(&english, &url, updated_english_response);

        // English should return the updated response.
        assert_eq!(
            cache.get(&english, &url).unwrap().body.as_ref(),
            b"Hello again"
        );

        // Spanish should still return its original response.
        assert_eq!(cache.get(&spanish, &url).unwrap().body.as_ref(), b"Hola");
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
        request
            .set_header("Accept-Language".parse().unwrap(), "en".parse().unwrap())
            .unwrap();
        request
            .set_header("Accept".parse().unwrap(), "text/html".parse().unwrap())
            .unwrap();

        let mut language_headers = HeaderMap::new();
        language_headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        language_headers.insert("vary", "Accept-Language".parse().unwrap());

        let language_response = StoredResponse {
            status: StatusCode::OK,
            headers: language_headers,
            url: url.to_string(),
            body: Bytes::from_static(b"English response"),
            cookie_headers_processed: false,
        };

        cache.insert(&request, &url, language_response.clone());

        let mut encoding_headers = HeaderMap::new();
        encoding_headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        encoding_headers.insert("vary", "Accept".parse().unwrap());

        let encoding_response = StoredResponse {
            headers: encoding_headers,
            body: Bytes::from_static(b"Gzip response"),
            ..language_response.clone()
        };

        cache.insert(&request, &url, encoding_response);

        // The cache should contain only the new Vary rule.
        let key = cache_key(&request, &url);
        let entries = cache.entries.read().unwrap();
        let variants = entries.entries.get(&key).unwrap();

        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].response.body.as_ref(), b"Gzip response");
    }

    #[test]
    /// Verifies a cache hit shares immutable body storage with its cache entry.
    fn cache_hits_share_immutable_body_bytes() {
        let cache = ResponseCache::default();
        let request = Request::get("https://example.test/shared-body");
        let url = Url::parse(request.url()).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());

        cache.insert(
            &request,
            &url,
            StoredResponse {
                status: StatusCode::OK,
                headers,
                url: url.to_string(),
                body: Bytes::from(vec![1_u8; 1024]),
                cookie_headers_processed: false,
            },
        );

        let hit = cache.get(&request, &url).unwrap();
        let entries = cache.entries.read().unwrap();
        let cached = &entries.entries[&cache_key(&request, &url)][0];

        assert_eq!(hit.body.as_ref(), cached.response.body.as_ref());
        assert_eq!(hit.body.as_ptr(), cached.response.body.as_ptr());
    }

    #[test]
    fn cache_keys_isolate_credentials_modes_request_modes_and_authorization() {
        let url = Url::parse("https://example.test/identity").unwrap();
        let omit = Request::get(url.as_str());

        let mut include = omit.try_clone().unwrap();
        include.set_credentials_mode(CredentialsMode::Include);
        assert_ne!(cache_key(&omit, &url), cache_key(&include, &url));

        let mut no_cors = omit.try_clone().unwrap();
        no_cors.set_mode(RequestMode::NoCors).unwrap();
        assert_ne!(cache_key(&omit, &url), cache_key(&no_cors, &url));

        let mut authorized = omit.try_clone().unwrap();
        authorized
            .set_header(AUTHORIZATION, "Bearer cache-key-test".parse().unwrap())
            .unwrap();
        assert_ne!(cache_key(&omit, &url), cache_key(&authorized, &url));
    }

    #[test]
    fn cache_evicts_the_least_recently_used_primary_entry() {
        let cache = ResponseCache::new(2, 64);
        let first = Url::parse("https://example.test/first").unwrap();
        let second = Url::parse("https://example.test/second").unwrap();
        let third = Url::parse("https://example.test/third").unwrap();
        let first_request = Request::get(first.as_str());
        let second_request = Request::get(second.as_str());
        let third_request = Request::get(third.as_str());

        cache.insert(&first_request, &first, cached_response(&first, b"first"));
        cache.insert(
            &second_request,
            &second,
            cached_response(&second, b"second"),
        );
        assert!(cache.get(&first_request, &first).is_some());
        cache.insert(&third_request, &third, cached_response(&third, b"third"));

        assert!(cache.get(&first_request, &first).is_some());
        assert!(cache.get(&second_request, &second).is_none());
        assert!(cache.get(&third_request, &third).is_some());
    }

    #[test]
    fn cache_evicts_across_partitions_to_stay_within_the_body_budget() {
        let cache = ResponseCache::new(4, 8);
        let url = Url::parse("https://example.test/budget").unwrap();
        let mut first = Request::get(url.as_str());
        first.set_cache_partition("one".into());
        let mut second = first.try_clone().unwrap();
        second.set_cache_partition("two".into());

        cache.insert(&first, &url, cached_response(&url, b"first"));
        cache.insert(&second, &url, cached_response(&url, b"second"));

        assert!(cache.get(&first, &url).is_none());
        assert!(cache.get(&second, &url).is_some());
        assert_eq!(cache.entries.read().unwrap().total_body_size, 6);
    }

    #[test]
    fn insertion_removes_expired_entries_before_admitting_new_data() {
        let cache = ResponseCache::new(4, 64);
        let expired_url = Url::parse("https://example.test/expired-prune").unwrap();
        let fresh_url = Url::parse("https://example.test/fresh-prune").unwrap();
        let expired_request = Request::get(expired_url.as_str());
        let fresh_request = Request::get(fresh_url.as_str());
        let expired_key = cache_key(&expired_request, &expired_url);

        cache.insert(
            &expired_request,
            &expired_url,
            cached_response(&expired_url, b"expired"),
        );
        cache
            .entries
            .write()
            .unwrap()
            .entries
            .get_mut(&expired_key)
            .unwrap()[0]
            .expires_at = Instant::now();

        cache.insert(
            &fresh_request,
            &fresh_url,
            cached_response(&fresh_url, b"fresh"),
        );

        let entries = cache.entries.read().unwrap();
        assert!(!entries.entries.contains_key(&expired_key));
        assert_eq!(entries.entries.len(), 1);
        assert_eq!(entries.total_body_size, 5);
    }

    #[test]
    fn authorized_requests_are_never_stored() {
        let cache = ResponseCache::default();
        let url = Url::parse("https://example.test/authorized").unwrap();
        let mut request = Request::get(url.as_str());
        request
            .set_header(AUTHORIZATION, "Bearer do-not-store".parse().unwrap())
            .unwrap();

        cache.insert(&request, &url, cached_response(&url, b"private"));

        assert!(cache.get(&request, &url).is_none());
    }

    fn cached_response(url: &Url, body: &'static [u8]) -> StoredResponse {
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from_static(body),
            cookie_headers_processed: false,
        }
    }
}
