//! Conservative browser-owned services used by the Fetch controller.

use net::{
    CachePartitionService, CookieService, Request, RequestError, RequestPolicyService,
    ResponseInfo, ResponsePolicyService, ServiceWorkerDecision, ServiceWorkerService,
};
use security::{
    CookieJar, Origin,
    permissions::{PermissionKind, PermissionPolicy},
};
use std::sync::{Arc, Mutex, RwLock};

/// Browser-owned Fetch policy defaults for the current composition slice.
///
/// The service permits requests and responses after the engine's own Fetch
/// validation. Cookies and service workers are not implemented by browser
/// composition yet, so it selects no cookies, accepts no-op cookie storage,
/// and always continues on the network path. HTTP cache partitions combine a
/// process-local namespace with the request's network partition key; they are
/// not durable cache identities.
#[derive(Clone, Debug)]
pub struct BrowserFetchCapabilities {
    cache_partition_namespace: String,
    cookies: Arc<Mutex<CookieJar>>,
    permission_policy: Arc<RwLock<PermissionPolicy>>,
}

impl BrowserFetchCapabilities {
    /// Creates browser services using `cache_partition_namespace` to scope
    /// network-key-derived HTTP response and CORS-preflight cache partitions.
    pub fn new(cache_partition_namespace: impl Into<String>) -> Self {
        Self {
            cache_partition_namespace: cache_partition_namespace.into(),
            cookies: Arc::new(Mutex::new(CookieJar::new())),
            permission_policy: Arc::new(RwLock::new(PermissionPolicy::new())),
        }
    }

    /// Creates browser services backed by `permission_policy` for cookie
    /// decisions. The policy is shared so permission changes affect future
    /// requests without rebuilding the network controller.
    pub fn with_permission_policy(
        cache_partition_namespace: impl Into<String>,
        permission_policy: Arc<RwLock<PermissionPolicy>>,
    ) -> Self {
        Self {
            cache_partition_namespace: cache_partition_namespace.into(),
            cookies: Arc::new(Mutex::new(CookieJar::new())),
            permission_policy,
        }
    }

    fn request_origin(&self, request: &Request) -> Option<Origin> {
        let url = request.environment().base_url.as_ref()?;
        let host = url.host_str()?;
        Some(Origin::new(url.scheme(), host, url.port()))
    }

    fn cookies_allowed(&self, request: &Request) -> bool {
        self.request_origin(request).is_some_and(|origin| {
            self.permission_policy
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .check(&origin, PermissionKind::Cookies)
                == security::permissions::PermissionState::Granted
        })
    }
}

impl Default for BrowserFetchCapabilities {
    /// Creates services for the browser's default process-local partition.
    fn default() -> Self {
        Self::new("browser-default")
    }
}

impl RequestPolicyService for BrowserFetchCapabilities {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }
}

impl ResponsePolicyService for BrowserFetchCapabilities {
    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        Ok(())
    }
}

impl CookieService for BrowserFetchCapabilities {
    fn cookie_header(&self, request: &Request) -> Result<Option<String>, RequestError> {
        if !self.cookies_allowed(request) {
            return Ok(None);
        }
        let Some(url) = net::Url::parse(request.active_url()).ok() else {
            return Ok(None);
        };
        let Some(host) = url.host_str() else {
            return Ok(None);
        };
        let origin = Origin::new(url.scheme(), host, url.port());
        Ok(self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .header_for(&origin))
    }

    fn store_set_cookie(
        &self,
        request: &Request,
        values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        if !self.cookies_allowed(request) {
            return Ok(());
        }
        let Some(url) = net::Url::parse(request.active_url()).ok() else {
            return Ok(());
        };
        let Some(host) = url.host_str() else {
            return Ok(());
        };
        let origin = Origin::new(url.scheme(), host, url.port());
        let mut jar = self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for value in values {
            let Ok(value) = value.to_str() else { continue };
            let mut parts = value.split(';').map(str::trim);
            let Some((name, cookie_value)) = parts.next().and_then(|part| part.split_once('='))
            else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let mut domain = host.to_ascii_lowercase();
            let mut secure = false;
            for attribute in parts {
                if attribute.eq_ignore_ascii_case("secure") {
                    secure = true;
                } else if let Some((key, value)) = attribute.split_once('=')
                    && key.trim().eq_ignore_ascii_case("domain")
                {
                    domain = value.trim().trim_start_matches('.').to_ascii_lowercase();
                }
            }
            if domain != host {
                continue;
            }
            jar.store(
                &origin,
                security::Cookie {
                    name: name.trim().to_owned(),
                    value: cookie_value.trim().to_owned(),
                    domain,
                    secure,
                },
            );
        }
        Ok(())
    }
}

impl ServiceWorkerService for BrowserFetchCapabilities {
    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }
}

impl CachePartitionService for BrowserFetchCapabilities {
    fn cache_partition(&self, request: &Request) -> Result<String, RequestError> {
        Ok(request
            .environment()
            .network_partition_key
            .as_ref()
            .map(|key| {
                format!(
                    "{}|{}",
                    self.cache_partition_namespace,
                    key.top_level_origin.as_str()
                )
            })
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::{FetchEnvironment, Url};
    use security::permissions::PermissionState;

    #[test]
    fn cache_partitions_follow_the_network_partition_key() {
        let capabilities = BrowserFetchCapabilities::default();
        let mut first = Request::get("https://resource.test/data");
        first.set_environment(
            FetchEnvironment::from_url(Url::parse("https://first.test/page").unwrap()).unwrap(),
        );
        let mut second = Request::get("https://resource.test/data");
        second.set_environment(
            FetchEnvironment::from_url(Url::parse("https://second.test/page").unwrap()).unwrap(),
        );

        assert_ne!(
            capabilities.cache_partition(&first).unwrap(),
            capabilities.cache_partition(&second).unwrap()
        );
    }

    #[test]
    fn missing_network_partition_key_disables_the_http_cache() {
        let capabilities = BrowserFetchCapabilities::default();

        assert_eq!(
            capabilities
                .cache_partition(&Request::get("https://resource.test/data"))
                .unwrap(),
            ""
        );
    }

    #[test]
    fn cookies_require_the_initiating_origin_permission() {
        let policy = Arc::new(RwLock::new(PermissionPolicy::new()));
        let capabilities = BrowserFetchCapabilities::with_permission_policy("test", policy.clone());
        let site = security::Origin::new("https", "site.test", None);
        let mut request = Request::get("https://site.test/data");
        request.set_environment(
            FetchEnvironment::from_url(Url::parse("https://site.test/").unwrap()).unwrap(),
        );
        capabilities
            .store_set_cookie(&request, &[net::HeaderValue::from_static("sid=one")])
            .unwrap();
        assert_eq!(capabilities.cookie_header(&request).unwrap(), None);

        policy.write().unwrap().grant(site, PermissionKind::Cookies);
        capabilities
            .store_set_cookie(&request, &[net::HeaderValue::from_static("sid=one")])
            .unwrap();
        assert_eq!(
            capabilities.cookie_header(&request).unwrap().as_deref(),
            Some("sid=one")
        );
        assert_eq!(
            PermissionState::Granted,
            policy.read().unwrap().check(
                &security::Origin::new("https", "site.test", None),
                PermissionKind::Cookies
            )
        );
    }
}
