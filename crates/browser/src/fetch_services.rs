//! Conservative browser-owned services used by the Fetch controller.

use net::{FetchServices, Request, RequestError, ResponseInfo, ServiceWorkerDecision};

/// Browser-owned Fetch policy defaults for the current composition slice.
///
/// The service permits requests and responses after the engine's own Fetch
/// validation. Cookies and service workers are not implemented by browser
/// composition yet, so it selects no cookies, accepts no-op cookie storage,
/// and always continues on the network path. The cache partition is an
/// explicit process-local browser partition, not a durable cache identity.
#[derive(Clone, Debug)]
pub struct BrowserFetchServices {
    cache_partition: String,
}

impl BrowserFetchServices {
    /// Creates browser services using `cache_partition` for process-local HTTP
    /// cache isolation.
    pub fn new(cache_partition: impl Into<String>) -> Self {
        Self {
            cache_partition: cache_partition.into(),
        }
    }
}

impl Default for BrowserFetchServices {
    /// Creates services for the browser's default process-local partition.
    fn default() -> Self {
        Self::new("browser-default")
    }
}

impl FetchServices for BrowserFetchServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }

    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        Ok(())
    }

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

    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }

    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok(self.cache_partition.clone())
    }
}
