//! Cached response representations and `Vary` matching.

use bytes::Bytes;
use reqwest::{
    StatusCode,
    header::{ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED},
};

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

/// Validators retained with a cached representation for conditional requests.
///
/// A response can provide both an entity tag and a modification time. Fetch
/// sends both corresponding conditions on a cache-generated revalidation so a
/// server can apply its normal precedence rules.
#[derive(Clone, Debug)]
pub(crate) struct CacheValidator {
    entity_tag: Option<HeaderValue>,
    last_modified: Option<HeaderValue>,
}

impl CacheValidator {
    /// Adds this validator to internal transport headers.
    pub(crate) fn apply(&self, headers: &mut crate::Headers) -> Result<(), crate::RequestError> {
        if let Some(value) = &self.entity_tag {
            headers.insert_internal_header(IF_NONE_MATCH, value.clone())?;
        }
        if let Some(value) = &self.last_modified {
            headers.insert_internal_header(IF_MODIFIED_SINCE, value.clone())?;
        }
        Ok(())
    }
}

impl CachedResponse {
    /// Moves a response into the cache with parsed freshness metadata.
    pub(super) fn from_response(
        request_headers: &crate::Headers,
        response: StoredResponse,
        freshness: FreshnessMetadata,
        last_access: u64,
    ) -> Self {
        let vary_headers = capture_vary_headers(request_headers, &response.headers);
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

    /// Reports whether this entry is still the representation that a
    /// conditional request validated.
    ///
    /// A concurrent fetch can replace a variant between validator selection
    /// and receipt of its `304 Not Modified`. A late `304` must not freshen
    /// that newer representation.
    pub(super) fn matches_revalidation(&self, response: &StoredResponse) -> bool {
        self.response.status == response.status
            && self.response.headers == response.headers
            && self.response.url == response.url
            && self.response.body == response.body
            && self.response.cookie_headers_processed == response.cookie_headers_processed
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

    /// Applies `304` header updates while retaining the representation body.
    pub(super) fn merge_not_modified_headers(
        &mut self,
        request_headers: &crate::Headers,
        headers: &HeaderMap,
    ) {
        // RFC 9111 requires removal of stale warning fields after successful
        // validation. Warnings in the 1xx range describe a stale response and
        // must not survive a `304` refresh.
        let retained_warnings: Vec<_> = self
            .response
            .headers
            .get_all(http::header::WARNING)
            .iter()
            .filter(|value| {
                let bytes = value.as_bytes();
                !(bytes.len() >= 3
                    && bytes[0] == b'1'
                    && bytes[1].is_ascii_digit()
                    && bytes[2].is_ascii_digit())
            })
            .cloned()
            .collect();
        self.response.headers.remove(http::header::WARNING);
        for warning in retained_warnings {
            self.response.headers.append(http::header::WARNING, warning);
        }
        let mut replaced_names = std::collections::HashSet::new();
        for (name, _) in headers {
            // A 304 has no representation body, so its content length cannot
            // describe the retained body. `Set-Cookie` is a response-side
            // effect rather than representation metadata; it is handled by
            // the validating response path and must not be replayed by later
            // cache hits.
            if name != http::header::CONTENT_LENGTH && name != http::header::SET_COOKIE {
                replaced_names.insert(name.clone());
            }
        }
        for name in replaced_names {
            self.response.headers.remove(&name);
            for value in headers.get_all(&name) {
                self.response.headers.append(name.clone(), value.clone());
            }
        }
        self.validator = response_validator(&self.response.headers);
        self.vary_headers = capture_vary_headers(request_headers, &self.response.headers);
    }

    /// Records cache metadata after successful `304` header freshening.
    pub(super) fn refresh_not_modified(
        &mut self,
        freshness: FreshnessMetadata,
        access: u64,
    ) -> StoredResponse {
        self.freshness = freshness;
        self.last_access = access;
        self.as_response()
    }
}

fn response_validator(headers: &HeaderMap) -> Option<CacheValidator> {
    let validator = CacheValidator {
        entity_tag: headers.get(ETAG).cloned(),
        last_modified: headers.get(LAST_MODIFIED).cloned(),
    };
    (validator.entity_tag.is_some() || validator.last_modified.is_some()).then_some(validator)
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
    request_headers: &crate::Headers,
    response_headers: &HeaderMap,
) -> Vec<(String, Option<String>)> {
    vary_header_names(response_headers)
        .into_iter()
        .map(|name| {
            let value = request_headers.get(&name);
            (name, value)
        })
        .collect()
}
