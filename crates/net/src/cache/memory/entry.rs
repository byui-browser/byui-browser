//! Cached response representations and `Vary` matching.

use bytes::Bytes;
use reqwest::{
    StatusCode,
    header::{ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED},
};

use crate::api::request::Request;

use super::freshness::FreshnessMetadata;

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

/// Cache metadata paired with the one owned stored response representation.
///
/// `StoredResponse` is moved into this entry on insertion. This intentionally
/// avoids a second header/body representation while the cache retains it.
#[derive(Clone, Debug)]
pub(crate) struct CachedResponse {
    pub(super) response: StoredResponse,
    pub(super) freshness: FreshnessMetadata,
    pub(super) vary_headers: Vec<(String, Option<String>)>,
    pub(super) last_access: u64,
    pub(super) validator: Option<CacheValidator>,
}

/// A validator retained with a cached representation for conditional requests.
#[derive(Clone, Debug)]
pub(crate) enum CacheValidator {
    /// Revalidate using the response entity tag.
    EntityTag(HeaderValue),
    /// Revalidate using the response modification time when no entity tag exists.
    LastModified(HeaderValue),
}

impl CacheValidator {
    /// Adds this validator to internal transport headers.
    pub(crate) fn apply(&self, headers: &mut crate::Headers) -> Result<(), crate::RequestError> {
        match self {
            Self::EntityTag(value) => headers.insert_internal_header(IF_NONE_MATCH, value.clone()),
            Self::LastModified(value) => {
                headers.insert_internal_header(IF_MODIFIED_SINCE, value.clone())
            }
        }
    }
}

impl CachedResponse {
    /// Moves a response into the cache with parsed freshness metadata.
    pub(super) fn from_response(
        request: &Request,
        response: StoredResponse,
        freshness: FreshnessMetadata,
        last_access: u64,
    ) -> Self {
        let vary_headers = capture_vary_headers(request, &response.headers);
        Self {
            validator: response_validator(&response.headers),
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

    pub(super) fn body_size(&self) -> usize {
        self.response.body.len()
    }

    pub(super) fn vary_names(&self) -> Vec<&str> {
        self.vary_headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Merges successful revalidation metadata while retaining the representation body.
    pub(super) fn merge_not_modified(
        &mut self,
        headers: &HeaderMap,
        freshness: FreshnessMetadata,
        access: u64,
    ) -> StoredResponse {
        for (name, value) in headers {
            // A 304 has no representation body, so its content length cannot
            // describe the retained body.
            if name != http::header::CONTENT_LENGTH {
                self.response.headers.insert(name.clone(), value.clone());
            }
        }
        self.validator = response_validator(&self.response.headers);
        self.freshness = freshness;
        self.last_access = access;
        self.as_response()
    }
}

/// Prefers an entity tag over a date validator, as required for conditional GET.
fn response_validator(headers: &HeaderMap) -> Option<CacheValidator> {
    headers
        .get(ETAG)
        .cloned()
        .map(CacheValidator::EntityTag)
        .or_else(|| {
            headers
                .get(LAST_MODIFIED)
                .cloned()
                .map(CacheValidator::LastModified)
        })
}

/// Returns the request header names listed in the response's `Vary` header.
pub(super) fn vary_header_names(headers: &HeaderMap) -> Vec<String> {
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

/// Captures the original request values for headers listed in `Vary`.
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
