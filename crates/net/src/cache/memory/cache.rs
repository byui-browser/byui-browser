//! Thread-safe response-cache operations.

use std::{
    fmt,
    sync::{Arc, RwLock},
    time::Instant,
};

use reqwest::Url;

use crate::api::request::Request;

use super::{
    entry::{CacheValidator, CachedResponse, StoredResponse, vary_header_names},
    freshness::{CacheClock, FreshnessMetadata, SystemCacheClock},
    key::{cache_key, has_authorization},
    storage::{CacheEntries, CacheLimits},
};

/// Thread-safe, bounded process-local cache shared by cloned request clients.
#[derive(Clone)]
pub(crate) struct ResponseCache {
    pub(super) entries: Arc<RwLock<CacheEntries>>,
    limits: CacheLimits,
    clock: Arc<dyn CacheClock>,
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
        /// Validator available for a conditional network request.
        validator: Option<CacheValidator>,
    },
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

    #[cfg(test)]
    pub(super) fn with_clock(
        max_entries: usize,
        max_body_size: usize,
        clock: Arc<dyn CacheClock>,
    ) -> Self {
        Self {
            entries: Arc::new(RwLock::new(CacheEntries::default())),
            limits: CacheLimits {
                max_entries,
                max_body_size,
            },
            clock,
        }
    }

    #[cfg(not(test))]
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
                validator: cached.validator.clone(),
            })
        }
    }

    /// Returns a matching representation and its validator, regardless of freshness.
    ///
    /// This is used for `no-cache`, which must revalidate even a fresh entry.
    pub(crate) fn lookup_for_revalidation(
        &self,
        request: &Request,
        url: &Url,
    ) -> Option<(StoredResponse, CacheValidator)> {
        let key = cache_key(request, url);
        let mut entries = self.entries.write().expect("cache lock poisoned");
        let access = entries.next_access();
        let cached = entries.entries.get_mut(&key)?.iter_mut().find(|entry| {
            entry
                .vary_headers
                .iter()
                .all(|(name, original_value)| request.headers().get(name) == *original_value)
        })?;
        cached.last_access = access;
        cached
            .validator
            .clone()
            .map(|validator| (cached.as_response(), validator))
    }

    /// Merges a 304 response into its matching stored representation.
    ///
    /// The stored status and body remain authoritative; 304 metadata replaces
    /// corresponding representation metadata and establishes new freshness.
    pub(crate) fn merge_not_modified(
        &self,
        request: &Request,
        url: &Url,
        headers: &reqwest::header::HeaderMap,
    ) -> Option<StoredResponse> {
        let key = cache_key(request, url);
        let now = self.clock.now();
        let mut entries = self.entries.write().expect("cache lock poisoned");
        let access = entries.next_access();
        let cached = entries.entries.get_mut(&key)?.iter_mut().find(|entry| {
            entry
                .vary_headers
                .iter()
                .all(|(name, original_value)| request.headers().get(name) == *original_value)
        })?;
        let mut merged_headers = cached.response.headers.clone();
        for (name, value) in headers {
            if name != http::header::CONTENT_LENGTH {
                merged_headers.insert(name.clone(), value.clone());
            }
        }
        let freshness = FreshnessMetadata::from_headers(&merged_headers, now)?;
        Some(cached.merge_not_modified(headers, freshness, access))
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

impl fmt::Debug for ResponseCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponseCache")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
