//! Conservative browser-owned services used by the Fetch controller.

use net::{
    CachePartitionService, CookieService, Request, RequestError, RequestPolicyService,
    ResponseInfo, ResponsePolicyService, ServiceWorkerDecision, ServiceWorkerService,
};

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
}

impl BrowserFetchCapabilities {
    /// Creates browser services using `cache_partition_namespace` to scope
    /// network-key-derived HTTP response and CORS-preflight cache partitions.
    pub fn new(cache_partition_namespace: impl Into<String>) -> Self {
        Self {
            cache_partition_namespace: cache_partition_namespace.into(),
        }
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
    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        Ok(None)
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
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
}
