//! Bounded in-memory cache storage and eviction.

use std::collections::HashMap;

use super::{entry::CachedResponse, freshness::CacheTime, key::CacheKey};

/// Limits enforced across all partitions held by one response cache.
#[derive(Clone, Copy, Debug)]
pub(super) struct CacheLimits {
    pub(super) max_entries: usize,
    pub(super) max_body_size: usize,
}

/// Mutable cache state protected by the response-cache lock.
#[derive(Debug, Default)]
pub(super) struct CacheEntries {
    pub(super) entries: HashMap<CacheKey, Vec<CachedResponse>>,
    pub(super) total_body_size: usize,
    next_access: u64,
}

impl CacheEntries {
    pub(super) fn next_access(&mut self) -> u64 {
        let access = self.next_access;
        self.next_access = self.next_access.wrapping_add(1);
        access
    }

    pub(super) fn body_size_for(&self, key: &CacheKey) -> usize {
        self.entries
            .get(key)
            .into_iter()
            .flatten()
            .map(CachedResponse::body_size)
            .sum()
    }

    /// Drops stale data before insertion, retaining directive-authorized stale
    /// representations for `stale-while-revalidate` and `stale-if-error`.
    pub(super) fn prune_stale(&mut self, now: CacheTime) {
        self.entries.retain(|_, variants| {
            variants.retain(|entry| {
                entry.freshness.is_fresh(now) || entry.freshness.retains_stale_representation(now)
            });
            !variants.is_empty()
        });
        self.total_body_size = self
            .entries
            .values()
            .flatten()
            .map(CachedResponse::body_size)
            .sum();
    }

    pub(super) fn evict_to_limits(&mut self, limits: CacheLimits) {
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
