//! Cache request identity and partitioning.

use reqwest::{
    Url,
    header::{AUTHORIZATION, PROXY_AUTHORIZATION},
};

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

/// Browser-provided or standalone storage scope for a cache key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum CachePartition {
    Browser(String),
    Standalone,
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
pub(super) fn has_authorization(request: &Request) -> bool {
    request.headers().has(AUTHORIZATION.as_str())
        || request.headers().has(PROXY_AUTHORIZATION.as_str())
}
