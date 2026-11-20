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
    key::{cache_key, has_authorization, invalidation_key},
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
    /// data may use it after applying their cache-mode policy.
    Stale {
        response: StoredResponse,
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

    /// Looks up a matching representation using the complete outgoing headers.
    pub(crate) fn lookup_with_headers(
        &self,
        request: &Request,
        url: &Url,
        request_headers: &crate::Headers,
    ) -> Option<CacheLookup> {
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
                .all(|(name, original_value)| request_headers.get(name) == *original_value)
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
                validator: cached.validator.clone(),
            })
        }
    }

    /// Returns a matching stored representation using the complete outgoing headers.
    ///
    /// `force-cache` and `only-if-cached` select an entry even when its
    /// response directives otherwise require revalidation. Normal cache lookup
    /// must continue to use [`Self::lookup_with_headers`] to enforce freshness
    /// and validator rules.
    pub(crate) fn lookup_matching_with_headers(
        &self,
        request: &Request,
        url: &Url,
        request_headers: &crate::Headers,
    ) -> Option<StoredResponse> {
        let key = cache_key(request, url);
        let mut entries = self.entries.write().expect("cache lock poisoned");
        let access = entries.next_access();
        let cached = entries.entries.get_mut(&key)?.iter_mut().find(|entry| {
            entry
                .vary_headers
                .iter()
                .all(|(name, original_value)| request_headers.get(name) == *original_value)
        })?;
        cached.last_access = access;
        Some(cached.as_response())
    }

    /// Returns a matching representation and validator using complete outgoing headers.
    ///
    /// This is used for `no-cache`, which must revalidate even a fresh entry.
    pub(crate) fn lookup_for_revalidation_with_headers(
        &self,
        request: &Request,
        url: &Url,
        request_headers: &crate::Headers,
    ) -> Option<(StoredResponse, CacheValidator)> {
        let key = cache_key(request, url);
        let mut entries = self.entries.write().expect("cache lock poisoned");
        let access = entries.next_access();
        let cached = entries.entries.get_mut(&key)?.iter_mut().find(|entry| {
            entry
                .vary_headers
                .iter()
                .all(|(name, original_value)| request_headers.get(name) == *original_value)
        })?;
        cached.last_access = access;
        cached
            .validator
            .clone()
            .map(|validator| (cached.as_response(), validator))
    }

    /// Merges a 304 response into its matching stored representation.
    ///
    /// Merges a 304 response using the headers that were sent for validation.
    pub(crate) fn merge_not_modified_with_headers(
        &self,
        request: &Request,
        url: &Url,
        request_headers: &crate::Headers,
        headers: &reqwest::header::HeaderMap,
    ) -> Option<StoredResponse> {
        let key = cache_key(request, url);
        let now = self.clock.now();
        let mut entries = self.entries.write().expect("cache lock poisoned");
        let access = entries.next_access();
        let (response, removed_body_size) = {
            let variants = entries.entries.get_mut(&key)?;
            let cached_index = variants.iter().position(|entry| {
                entry
                    .vary_headers
                    .iter()
                    .all(|(name, original_value)| request_headers.get(name) == *original_value)
            })?;
            let mut merged_headers = variants[cached_index].response.headers.clone();
            for (name, value) in headers {
                if name != http::header::CONTENT_LENGTH {
                    merged_headers.insert(name.clone(), value.clone());
                }
            }
            let freshness = FreshnessMetadata::from_headers(&merged_headers, now)?;
            let mut cached = variants.remove(cached_index);
            let response = cached.merge_not_modified(request_headers, headers, freshness, access);
            let vary_names = cached.vary_names();
            let cached_vary_headers = &cached.vary_headers;
            let mut removed_body_size = 0_usize;
            variants.retain(|entry| {
                let retain =
                    entry.vary_names() == vary_names && entry.vary_headers != *cached_vary_headers;
                if !retain {
                    removed_body_size = removed_body_size.saturating_add(entry.body_size());
                }
                retain
            });
            variants.push(cached);
            (response, removed_body_size)
        };
        entries.total_body_size = entries.total_body_size.saturating_sub(removed_body_size);
        Some(response)
    }

    /// Stores a response with the complete headers sent for its request.
    pub(crate) fn insert_with_headers(
        &self,
        request: &Request,
        url: &Url,
        request_headers: &crate::Headers,
        response: StoredResponse,
    ) {
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
        let cached_response =
            CachedResponse::from_response(request_headers, response, freshness, access);
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

    /// Removes every cached representation for `url` in the request's cache partition.
    ///
    /// This intentionally spans methods and `Vary` variants because an unsafe
    /// request can change any representation of its target. Other cache-key
    /// attributes remain isolated through the browser-selected partition.
    pub(crate) fn invalidate(&self, request: &Request, url: &Url) {
        let invalidation = invalidation_key(request, url);
        let mut entries = self.entries.write().expect("cache lock poisoned");
        let removed: usize = entries
            .entries
            .extract_if(|key, _| key.matches_invalidation(&invalidation))
            .map(|(_, variants)| {
                variants
                    .iter()
                    .map(CachedResponse::body_size)
                    .sum::<usize>()
            })
            .sum();
        entries.total_body_size = entries.total_body_size.saturating_sub(removed);
        if removed > 0 {
            log::debug!(
                "cache invalidated {} bytes for {}",
                removed,
                crate::log_url::loggable(url.as_str())
            );
        }
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
