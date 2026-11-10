//! Process-local CORS-preflight permission cache.

use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use http::HeaderName;
use reqwest::{Method, Url};

use crate::api::request::{CredentialsMode, Request};

/// Method and header-name permissions granted by one preflight response.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreflightPermissions {
    /// Methods named by `Access-Control-Allow-Methods`, including `*`.
    pub(crate) methods: Vec<String>,
    /// Header names named by `Access-Control-Allow-Headers`, including `*`.
    pub(crate) header_names: Vec<String>,
}

/// Thread-safe preflight cache shared by cloned request controllers.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreflightCache {
    entries: Arc<RwLock<Vec<PreflightEntry>>>,
}

impl PreflightCache {
    /// Reports whether cached entries authorize the current method and headers.
    pub(crate) fn allows(&self, request: &Request, url: &Url, unsafe_names: &[HeaderName]) -> bool {
        let now = Instant::now();
        let key = PreflightKey::new(request, url);
        let include_credentials = request.context.credentials == CredentialsMode::Include;
        let mut entries = self.entries.write().expect("preflight cache lock poisoned");
        entries.retain(|entry| entry.expires_at > now);

        let matches_base = |entry: &&PreflightEntry| {
            entry.key == key && (entry.credentials || !include_credentials)
        };
        let method_allowed = is_cors_safelisted_method(request.method())
            || entries.iter().filter(matches_base).any(|entry| {
                entry.permission == PreflightPermission::Method(request.method().to_string())
                    || (!include_credentials
                        && entry.permission == PreflightPermission::Method("*".into()))
            });
        let headers_allowed = unsafe_names.iter().all(|name| {
            entries.iter().filter(matches_base).any(|entry| {
                entry.permission == PreflightPermission::HeaderName(name.as_str().into())
                    || (!include_credentials
                        && name != http::header::AUTHORIZATION
                        && entry.permission == PreflightPermission::HeaderName("*".into()))
            })
        });
        method_allowed && headers_allowed
    }

    /// Stores the permissions granted by a successful preflight response.
    pub(crate) fn insert(
        &self,
        request: &Request,
        url: &Url,
        permissions: &PreflightPermissions,
        ttl: Duration,
    ) {
        let now = Instant::now();
        let expires_at = now.checked_add(ttl).unwrap_or(now);
        let key = PreflightKey::new(request, url);
        let credentials = request.context.credentials == CredentialsMode::Include;
        let mut entries = self.entries.write().expect("preflight cache lock poisoned");
        entries.retain(|entry| entry.expires_at > now);

        for permission in permissions
            .methods
            .iter()
            .cloned()
            .map(PreflightPermission::Method)
            .chain(
                permissions
                    .header_names
                    .iter()
                    .cloned()
                    .map(PreflightPermission::HeaderName),
            )
        {
            if let Some(entry) = entries.iter_mut().find(|entry| {
                entry.key == key
                    && entry.credentials == credentials
                    && entry.permission == permission
            }) {
                entry.expires_at = expires_at;
            } else {
                entries.push(PreflightEntry {
                    key: key.clone(),
                    credentials,
                    permission,
                    expires_at,
                });
            }
        }
    }

    /// Removes every cached preflight permission.
    pub(crate) fn clear(&self) {
        self.entries
            .write()
            .expect("preflight cache lock poisoned")
            .clear();
    }
}

/// Partition, origin, and resource identity shared by permission entries.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PreflightKey {
    partition: Option<String>,
    origin: String,
    url: String,
}

impl PreflightKey {
    /// Builds the Fetch cache identity for the current request URL.
    fn new(request: &Request, url: &Url) -> Self {
        let mut normalized_url = url.clone();
        normalized_url.set_fragment(None);
        Self {
            partition: request.cache_partition.clone(),
            origin: request.serialized_origin().unwrap_or_default(),
            url: normalized_url.to_string(),
        }
    }
}

/// One independently expiring method or header-name authorization.
#[derive(Clone, Debug)]
struct PreflightEntry {
    key: PreflightKey,
    credentials: bool,
    permission: PreflightPermission,
    expires_at: Instant,
}

/// Permission category represented by an entry.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PreflightPermission {
    Method(String),
    HeaderName(String),
}

/// Reports whether a method belongs to Fetch's CORS safelist.
fn is_cors_safelisted_method(method: &Method) -> bool {
    matches!(method, &Method::GET | &Method::HEAD | &Method::POST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FetchEnvironment, Origin};

    /// Creates a cross-origin request with an explicit cache partition.
    fn request(method: &str) -> Request {
        let mut request = Request::new("https://remote.test/data", method).unwrap();
        request.context.environment = FetchEnvironment {
            origin: Some(Origin::parse("https://client.test").unwrap()),
            ..FetchEnvironment::default()
        };
        request.cache_partition = Some("top-level-site".into());
        request
    }

    #[test]
    fn cached_permissions_match_method_headers_and_credentials() {
        let cache = PreflightCache::default();
        let url = Url::parse("https://remote.test/data").unwrap();
        let mut request = request("PUT");
        let names = [HeaderName::from_static("x-example")];
        let permissions = PreflightPermissions {
            methods: vec!["PUT".into()],
            header_names: vec!["x-example".into()],
        };
        cache.insert(&request, &url, &permissions, Duration::from_secs(60));
        assert!(cache.allows(&request, &url, &names));

        request.context.credentials = CredentialsMode::Include;
        assert!(!cache.allows(&request, &url, &names));
        cache.insert(&request, &url, &permissions, Duration::from_secs(60));
        assert!(cache.allows(&request, &url, &names));
    }

    #[test]
    fn wildcard_does_not_authorize_credentialed_or_authorization_requests() {
        let cache = PreflightCache::default();
        let url = Url::parse("https://remote.test/data").unwrap();
        let request = request("PUT");
        let permissions = PreflightPermissions {
            methods: vec!["*".into()],
            header_names: vec!["*".into()],
        };
        cache.insert(&request, &url, &permissions, Duration::from_secs(60));

        assert!(cache.allows(&request, &url, &[HeaderName::from_static("x-example")]));
        assert!(!cache.allows(&request, &url, &[http::header::AUTHORIZATION]));
    }

    #[test]
    fn expired_permissions_are_cache_misses() {
        let cache = PreflightCache::default();
        let url = Url::parse("https://remote.test/data").unwrap();
        let request = request("PUT");
        cache.insert(
            &request,
            &url,
            &PreflightPermissions {
                methods: vec!["PUT".into()],
                header_names: Vec::new(),
            },
            Duration::ZERO,
        );
        assert!(!cache.allows(&request, &url, &[]));
    }

    #[test]
    fn cache_identity_separates_partition_origin_and_url() {
        let cache = PreflightCache::default();
        let url = Url::parse("https://remote.test/data").unwrap();
        let request = request("PUT");
        let permissions = PreflightPermissions {
            methods: vec!["PUT".into()],
            header_names: Vec::new(),
        };
        cache.insert(&request, &url, &permissions, Duration::from_secs(60));

        assert!(!cache.allows(
            &request,
            &Url::parse("https://remote.test/other").unwrap(),
            &[]
        ));
        let mut other_partition = request.try_clone().unwrap();
        other_partition.cache_partition = Some("other-site".into());
        assert!(!cache.allows(&other_partition, &url, &[]));
        let mut other_origin = request.try_clone().unwrap();
        other_origin.context.environment.origin =
            Some(Origin::parse("https://other-client.test").unwrap());
        assert!(!cache.allows(&other_origin, &url, &[]));
    }
}
