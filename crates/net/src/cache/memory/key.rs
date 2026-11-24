//! Cache request identity and partitioning.

use reqwest::Url;

use crate::api::request::{CredentialsMode, RedirectTaint, Request, RequestMode};

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

/// The partition and normalized URL shared by every representation invalidated
/// by one unsafe request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct InvalidationKey {
    partition: CachePartition,
    url: String,
}

/// Browser-provided or standalone storage scope for a cache key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum CachePartition {
    Browser(String),
    Standalone,
}

/// Creates the cache key used to distinguish method and URL combinations.
#[cfg(test)]
pub(crate) fn cache_key(request: &Request, url: &Url) -> CacheKey {
    cache_key_with_headers(request, url, request.headers())
}

/// Creates a cache key from the effective headers sent on the HTTP hop.
pub(crate) fn cache_key_with_headers(
    request: &Request,
    url: &Url,
    request_headers: &crate::Headers,
) -> CacheKey {
    let credentials_allowed = request.credentials_allowed(url);
    let url = normalized_url(url);
    CacheKey {
        partition: match request.cache_partition() {
            Some(partition) => CachePartition::Browser(partition.to_owned()),
            None => CachePartition::Standalone,
        },
        method: request.method().clone(),
        url,
        credentials_mode: request.credentials_mode(),
        credentials_allowed,
        request_mode: request.mode(),
        redirect_taint: request.redirect_taint(),
        has_authorization: has_authorization(request_headers),
    }
}

/// Creates the partition-and-URL scope used to invalidate cached responses.
///
/// Deliberately omits request-specific matching attributes such as method and
/// credentials: an unsafe mutation invalidates every cached representation of
/// its target within the selected partition.
pub(super) fn invalidation_key(request: &Request, url: &Url) -> InvalidationKey {
    InvalidationKey {
        partition: match request.cache_partition() {
            Some(partition) => CachePartition::Browser(partition.to_owned()),
            None => CachePartition::Standalone,
        },
        url: normalized_url(url),
    }
}

impl CacheKey {
    /// Reports whether this representation belongs to an invalidation scope.
    pub(super) fn matches_invalidation(&self, invalidation: &InvalidationKey) -> bool {
        self.partition == invalidation.partition && self.url == invalidation.url
    }
}

/// Serializes a URL for cache identity after removing its non-network fragment.
fn normalized_url(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    url.to_string()
}

/// Reports whether a request carries an authorization-bearing header.
pub(super) fn has_authorization(request_headers: &crate::Headers) -> bool {
    request_headers.has(http::header::AUTHORIZATION.as_str())
        || request_headers.has(http::header::PROXY_AUTHORIZATION.as_str())
}
