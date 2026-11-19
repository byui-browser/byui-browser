//! In-memory response cache and cache-policy helpers.

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use reqwest::{
    StatusCode, Url,
    header::{AGE, AUTHORIZATION, CACHE_CONTROL, DATE, EXPIRES, HeaderMap, PROXY_AUTHORIZATION},
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
#[derive(Clone)]
pub(crate) struct ResponseCache {
    entries: Arc<RwLock<CacheEntries>>,
    limits: CacheLimits,
    clock: Arc<dyn CacheClock>,
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new(256, 64 * 1024 * 1024)
    }
}

impl ResponseCache {
    /// Creates a bounded process-local response cache.
    pub(crate) fn new(max_entries: usize, max_body_size: usize) -> Self {
        Self::with_clock(
            max_entries,
            max_body_size,
            Arc::new(SystemCacheClock {
                started: Instant::now(),
            }),
        )
    }

    fn with_clock(max_entries: usize, max_body_size: usize, clock: Arc<dyn CacheClock>) -> Self {
        Self {
            entries: Arc::new(RwLock::new(CacheEntries::default())),
            limits: CacheLimits {
                max_entries,
                max_body_size,
            },
            clock,
        }
    }
}

impl fmt::Debug for ResponseCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponseCache")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// Clock reading used for cache freshness and deterministic tests.
#[derive(Clone, Copy, Debug)]
struct CacheTime {
    wall: Duration,
    elapsed: Duration,
}

/// Source of wall-clock and monotonic time for cache freshness calculations.
trait CacheClock: fmt::Debug + Send + Sync {
    fn now(&self) -> CacheTime;
}

/// Production clock with a monotonic origin shared by one cache instance.
#[derive(Debug)]
struct SystemCacheClock {
    started: Instant,
}

impl CacheClock for SystemCacheClock {
    fn now(&self) -> CacheTime {
        CacheTime {
            wall: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO),
            elapsed: self.started.elapsed(),
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

/// Freshness result for a matching cache entry.
#[derive(Debug)]
pub(crate) enum CacheLookup {
    /// The cached representation may be used without revalidation.
    Fresh(StoredResponse),
    /// The cached representation is stale; only callers allowed to use stale
    /// data may use it, and must honor `may_serve_stale`.
    Stale {
        response: StoredResponse,
        may_serve_stale: bool,
    },
}

impl ResponseCache {
    /// Looks up a matching cached representation and reports its freshness.
    pub(crate) fn lookup(&self, request: &Request, url: &Url) -> Option<CacheLookup> {
        let key = cache_key(request, url);
        let now = self.clock.now();
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

        let Some(cached) = cached.iter_mut().find(|entry| {
            entry
                .vary_headers
                .iter()
                .all(|(name, original_value)| request.headers().get(name) == *original_value)
        }) else {
            log::trace!("cache miss: no matching Vary response");
            return None;
        };
        cached.last_access = access;
        if cached.freshness.is_fresh(now) {
            Some(CacheLookup::Fresh(cached.as_response()))
        } else {
            Some(CacheLookup::Stale {
                response: cached.as_response(),
                may_serve_stale: cached.freshness.may_serve_stale(),
            })
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
        let now = self.clock.now();
        let Some(freshness) = FreshnessMetadata::from_headers(&response.headers, now) else {
            log::trace!(
                "cache store skipped for {}: no usable max-age",
                crate::log_url::loggable(url.as_str())
            );
            return;
        };
        log::debug!(
            "cache store: {} {} ({} bytes, freshness {}s)",
            request.method(),
            crate::log_url::loggable(url.as_str()),
            response.body.len(),
            freshness.lifetime.as_secs()
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
        entries.prune_stale(now);
        let access = entries.next_access();
        let cached_response = CachedResponse::from_response(request, response, freshness, access);
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
    freshness: FreshnessMetadata,
    vary_headers: Vec<(String, Option<String>)>,
    last_access: u64,
}

impl CachedResponse {
    /// Moves a response into the cache with parsed freshness metadata.
    fn from_response(
        request: &Request,
        response: StoredResponse,
        freshness: FreshnessMetadata,
        last_access: u64,
    ) -> Self {
        let vary_headers = capture_vary_headers(request, &response.headers);
        Self {
            response,
            freshness,
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

    /// Drops stale data before an insertion; stale data may still be served
    /// by `ForceCache` until a later insertion makes it eligible for eviction.
    fn prune_stale(&mut self, now: CacheTime) {
        self.entries.retain(|_, variants| {
            variants.retain(|entry| entry.freshness.is_fresh(now));
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

/// Parsed response directives that affect the supported freshness subset.
#[derive(Clone, Copy, Debug, Default)]
struct CacheControl {
    max_age: Option<Duration>,
    no_cache: bool,
    no_store: bool,
    must_revalidate: bool,
}

/// Stored age and freshness state for one cached response.
#[derive(Clone, Copy, Debug)]
struct FreshnessMetadata {
    response_time: CacheTime,
    date: Option<Duration>,
    age: Duration,
    lifetime: Duration,
    requires_revalidation: bool,
    must_revalidate: bool,
}

impl FreshnessMetadata {
    /// Builds cache metadata for the supported explicit-freshness subset.
    ///
    /// Responses without `max-age` or a valid `Expires` value are not stored;
    /// heuristic freshness is intentionally deferred.
    fn from_headers(headers: &HeaderMap, response_time: CacheTime) -> Option<Self> {
        let directives = cache_control(headers);
        if directives.no_store {
            return None;
        }
        let date = parse_http_date(headers.get(DATE));
        let expires = parse_http_date(headers.get(EXPIRES));
        let lifetime = if directives.no_cache {
            Duration::ZERO
        } else if let Some(max_age) = directives.max_age {
            max_age
        } else {
            expires?.saturating_sub(date.unwrap_or(response_time.wall))
        };
        Some(Self {
            response_time,
            date,
            age: parse_age(headers),
            lifetime,
            requires_revalidation: directives.no_cache,
            must_revalidate: directives.must_revalidate,
        })
    }

    fn is_fresh(&self, now: CacheTime) -> bool {
        !self.requires_revalidation && self.current_age(now) < self.lifetime
    }

    fn may_serve_stale(&self) -> bool {
        !self.requires_revalidation && !self.must_revalidate
    }

    fn current_age(&self, now: CacheTime) -> Duration {
        let apparent_age = self
            .date
            .map(|date| self.response_time.wall.saturating_sub(date))
            .unwrap_or(Duration::ZERO);
        let corrected_received_age = apparent_age.max(self.age);
        corrected_received_age
            .saturating_add(now.elapsed.saturating_sub(self.response_time.elapsed))
    }
}

/// Parses the supported `Cache-Control` directives without retaining raw values.
fn cache_control(headers: &HeaderMap) -> CacheControl {
    let mut directives = CacheControl::default();
    for value in headers.get_all(CACHE_CONTROL).iter() {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for directive in value.split(',') {
            let directive = directive.trim();
            if directive.eq_ignore_ascii_case("no-cache") {
                directives.no_cache = true;
            } else if directive.eq_ignore_ascii_case("no-store") {
                directives.no_store = true;
            } else if directive.eq_ignore_ascii_case("must-revalidate") {
                directives.must_revalidate = true;
            } else if let Some((name, value)) = directive.split_once('=')
                && name.trim().eq_ignore_ascii_case("max-age")
                && let Ok(seconds) = value.trim().trim_matches('"').parse::<u64>()
            {
                directives.max_age = Some(Duration::from_secs(seconds));
            }
        }
    }
    directives
}

/// Parses an HTTP-date header into seconds since the Unix epoch.
fn parse_http_date(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let value = value?.to_str().ok()?;
    httpdate::parse_http_date(value)
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
}

/// Parses the non-negative seconds in an `Age` header.
fn parse_age(headers: &HeaderMap) -> Duration {
    headers
        .get(AGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::ZERO)
}

/// Reads an explicit `max-age` lifetime for compatibility tests.
/// Responses with `no-store` or `no-cache` have no reusable max-age lifetime.
#[cfg(test)]
pub(crate) fn cache_ttl(headers: &HeaderMap) -> Option<Duration> {
    let directives = cache_control(headers);
    if directives.no_store || directives.no_cache {
        return None;
    }
    directives.max_age
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
        assert!(fresh_response(&cache, &english, &url).is_some());

        // Different language should produce a cache miss.
        let mut spanish = Request::get(url.as_str());
        spanish
            .set_header("Accept-Language".parse().unwrap(), "es".parse().unwrap())
            .unwrap();

        assert!(fresh_response(&cache, &spanish, &url).is_none());
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
        assert!(fresh_response(&cache, &request, &url).is_none());
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
    fn freshness_uses_age_and_resident_time() {
        let date = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT")
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap();
        let clock = Arc::new(TestClock::new(date + Duration::from_secs(10)));
        let cache = ResponseCache::with_clock(4, 64, clock.clone());
        let url = Url::parse("https://example.test/freshness").unwrap();
        let request = Request::get(url.as_str());
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        headers.insert(DATE, "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap());
        headers.insert(AGE, "30".parse().unwrap());

        cache.insert(
            &request,
            &url,
            StoredResponse {
                status: StatusCode::OK,
                headers,
                url: url.to_string(),
                body: Bytes::from_static(b"fresh"),
                cookie_headers_processed: false,
            },
        );
        clock.advance(Duration::from_secs(29));
        assert!(matches!(
            cache.lookup(&request, &url),
            Some(CacheLookup::Fresh(_))
        ));

        clock.advance(Duration::from_secs(1));
        assert!(matches!(
            cache.lookup(&request, &url),
            Some(CacheLookup::Stale { .. })
        ));
    }

    #[test]
    fn expires_is_used_when_max_age_is_absent() {
        let date = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT")
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap();
        let clock = Arc::new(TestClock::new(date));
        let cache = ResponseCache::with_clock(4, 64, clock.clone());
        let url = Url::parse("https://example.test/expires").unwrap();
        let request = Request::get(url.as_str());
        let mut headers = HeaderMap::new();
        headers.insert(DATE, "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap());
        headers.insert(EXPIRES, "Sun, 06 Nov 1994 08:50:37 GMT".parse().unwrap());

        cache.insert(
            &request,
            &url,
            StoredResponse {
                status: StatusCode::OK,
                headers,
                url: url.to_string(),
                body: Bytes::from_static(b"expires"),
                cookie_headers_processed: false,
            },
        );
        clock.advance(Duration::from_secs(59));
        assert!(matches!(
            cache.lookup(&request, &url),
            Some(CacheLookup::Fresh(_))
        ));
        clock.advance(Duration::from_secs(1));
        assert!(matches!(
            cache.lookup(&request, &url),
            Some(CacheLookup::Stale { .. })
        ));
    }

    #[test]
    fn no_cache_and_must_revalidate_entries_require_revalidation_when_stale() {
        let clock = Arc::new(TestClock::new(Duration::ZERO));
        let cache = ResponseCache::with_clock(4, 64, clock);
        let url = Url::parse("https://example.test/revalidate").unwrap();
        let request = Request::get(url.as_str());

        let mut no_cache_headers = HeaderMap::new();
        no_cache_headers.insert(CACHE_CONTROL, "no-cache, max-age=60".parse().unwrap());
        cache.insert(
            &request,
            &url,
            StoredResponse {
                status: StatusCode::OK,
                headers: no_cache_headers,
                url: url.to_string(),
                body: Bytes::from_static(b"no-cache"),
                cookie_headers_processed: false,
            },
        );
        assert!(matches!(
            cache.lookup(&request, &url),
            Some(CacheLookup::Stale {
                may_serve_stale: false,
                ..
            })
        ));

        let mut must_revalidate_headers = HeaderMap::new();
        must_revalidate_headers
            .insert(CACHE_CONTROL, "max-age=0, must-revalidate".parse().unwrap());
        cache.insert(
            &request,
            &url,
            StoredResponse {
                status: StatusCode::OK,
                headers: must_revalidate_headers,
                url: url.to_string(),
                body: Bytes::from_static(b"must-revalidate"),
                cookie_headers_processed: false,
            },
        );
        assert!(matches!(
            cache.lookup(&request, &url),
            Some(CacheLookup::Stale {
                may_serve_stale: false,
                ..
            })
        ));
    }

    #[test]
    fn malformed_expiry_and_negative_age_do_not_extend_freshness() {
        let now = CacheTime {
            wall: Duration::from_secs(1_000),
            elapsed: Duration::ZERO,
        };
        let mut invalid_expiry = HeaderMap::new();
        invalid_expiry.insert(EXPIRES, "not a date".parse().unwrap());
        assert!(FreshnessMetadata::from_headers(&invalid_expiry, now).is_none());

        let mut negative_age = HeaderMap::new();
        negative_age.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        negative_age.insert(AGE, "-1".parse().unwrap());
        assert_eq!(parse_age(&negative_age), Duration::ZERO);
        assert!(
            FreshnessMetadata::from_headers(&negative_age, now)
                .unwrap()
                .is_fresh(now)
        );
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

        assert!(fresh_response(&cache, &request, &url).is_none());
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
        let cached = CachedResponse::from_response(&request, response, expired_freshness(), 0);

        cache.entries.write().unwrap().entries.insert(
            cache_key(&request, &Url::parse(request.url()).unwrap()),
            vec![cached],
        );

        assert!(matches!(
            cache.lookup(&request, &Url::parse(request.url()).unwrap()),
            Some(CacheLookup::Stale { .. })
        ));
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
        let cached = CachedResponse::from_response(&request, response, expired_freshness(), 0);

        cache.entries.write().unwrap().entries.insert(
            cache_key(&request, &Url::parse(request.url()).unwrap()),
            vec![cached],
        );

        assert!(matches!(
            cache.lookup(&request, &Url::parse(request.url()).unwrap()),
            Some(CacheLookup::Stale { .. })
        ));
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
        assert_eq!(
            fresh_response(&cache, &english, &url)
                .unwrap()
                .body
                .as_ref(),
            b"Hello"
        );
        assert_eq!(
            fresh_response(&cache, &spanish, &url)
                .unwrap()
                .body
                .as_ref(),
            b"Hola"
        );

        // Update the English response with new content.
        let updated_english_response = StoredResponse {
            body: Bytes::from_static(b"Hello again"),
            ..english_response.clone()
        };

        cache.insert(&english, &url, updated_english_response);

        // English should return the updated response.
        assert_eq!(
            fresh_response(&cache, &english, &url)
                .unwrap()
                .body
                .as_ref(),
            b"Hello again"
        );

        // Spanish should still return its original response.
        assert_eq!(
            fresh_response(&cache, &spanish, &url)
                .unwrap()
                .body
                .as_ref(),
            b"Hola"
        );
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

        let hit = fresh_response(&cache, &request, &url).unwrap();
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
        assert!(fresh_response(&cache, &first_request, &first).is_some());
        cache.insert(&third_request, &third, cached_response(&third, b"third"));

        assert!(fresh_response(&cache, &first_request, &first).is_some());
        assert!(fresh_response(&cache, &second_request, &second).is_none());
        assert!(fresh_response(&cache, &third_request, &third).is_some());
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

        assert!(fresh_response(&cache, &first, &url).is_none());
        assert!(fresh_response(&cache, &second, &url).is_some());
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
            .freshness = expired_freshness();

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

        assert!(fresh_response(&cache, &request, &url).is_none());
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

    fn fresh_response(
        cache: &ResponseCache,
        request: &Request,
        url: &Url,
    ) -> Option<StoredResponse> {
        match cache.lookup(request, url) {
            Some(CacheLookup::Fresh(response)) => Some(response),
            Some(CacheLookup::Stale { .. }) | None => None,
        }
    }

    fn expired_freshness() -> FreshnessMetadata {
        FreshnessMetadata {
            response_time: CacheTime {
                wall: Duration::ZERO,
                elapsed: Duration::ZERO,
            },
            date: None,
            age: Duration::ZERO,
            lifetime: Duration::ZERO,
            requires_revalidation: false,
            must_revalidate: false,
        }
    }

    #[derive(Debug)]
    struct TestClock {
        now: RwLock<CacheTime>,
    }

    impl TestClock {
        fn new(wall: Duration) -> Self {
            Self {
                now: RwLock::new(CacheTime {
                    wall,
                    elapsed: Duration::ZERO,
                }),
            }
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.now.write().unwrap();
            now.wall = now.wall.saturating_add(duration);
            now.elapsed = now.elapsed.saturating_add(duration);
        }
    }

    impl CacheClock for TestClock {
        fn now(&self) -> CacheTime {
            *self.now.read().unwrap()
        }
    }
}
