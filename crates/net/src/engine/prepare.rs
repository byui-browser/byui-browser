//! Request validation and service preparation.

use crate::{
    api::{
        error::RequestError,
        request::{
            CredentialsMode, InitiatorType, PreparedRequest, Request, RequestDestination,
            ServiceWorkersMode,
        },
        services::ServiceWorkerDecision,
    },
    engine::RequestController,
};

impl RequestController {
    /// Validates a request and resolves browser-owned services before execution.
    pub(crate) fn prepare_request(
        &self,
        request: Request,
    ) -> Result<(PreparedRequest, crate::policy::FetchScheme), RequestError> {
        if request.signal().is_aborted() {
            log::debug!(
                "request aborted before preparation: {}",
                crate::log_url::loggable(request.url())
            );
            return Err(RequestError::Aborted);
        }
        let validated_url = self
            .inner
            .policy
            .validate_request(&request, self.inner.max_keepalive_body_size)
            .inspect_err(|error| {
                log::debug!(
                    "request rejected by policy: {} ({})",
                    crate::log_url::loggable(request.url()),
                    crate::log_url::loggable_error(error)
                );
            })?;
        let mut prepared = PreparedRequest::new(request, validated_url.url.clone());
        let request = &mut prepared.request;
        let requires_browser_capabilities = request.environment().origin.is_some()
            || request.credentials_mode() == CredentialsMode::Include
            || request.destination() != RequestDestination::Empty
            || request.initiator() != InitiatorType::Other;

        if requires_browser_capabilities && self.inner.request_policy.is_none() {
            return Err(missing_capability(request, "request policy service"));
        }
        if requires_browser_capabilities && self.inner.response_policy.is_none() {
            return Err(missing_capability(request, "response policy service"));
        }
        if requires_browser_capabilities
            && validated_url.scheme.is_http()
            && self.inner.cookies.is_none()
        {
            return Err(missing_capability(request, "cookie service"));
        }
        if requires_browser_capabilities
            && validated_url.scheme.is_http()
            && self.inner.cache_partitions.is_none()
        {
            return Err(missing_capability(request, "cache partition service"));
        }
        if requires_browser_capabilities
            && validated_url.scheme != crate::policy::FetchScheme::File
            && request.service_workers_mode() == ServiceWorkersMode::All
            && self.inner.service_workers.is_none()
        {
            return Err(missing_capability(request, "service-worker service"));
        }

        if let Some(policy) = &self.inner.request_policy {
            policy.check_request(request)?;
            if validated_url.scheme != crate::policy::FetchScheme::File
                && request.service_workers_mode() == ServiceWorkersMode::All
                && let Some(service_workers) = &self.inner.service_workers
                && service_workers.service_worker(request)? == ServiceWorkerDecision::Intercept
            {
                log::warn!(
                    "service-worker interception is unsupported: {}",
                    crate::log_url::loggable(request.url())
                );
                return Err(RequestError::UnsupportedFeature(
                    "service-worker interception",
                ));
            }
        }

        if validated_url.scheme.is_http() {
            let partition = self
                .inner
                .cache_partitions
                .as_ref()
                .map(|service| service.cache_partition(request))
                .transpose()?
                .filter(|partition| !partition.is_empty());
            if let Some(partition) = partition {
                request.set_cache_partition(partition);
            } else if request.cache_mode() != crate::CacheMode::OnlyIfCached {
                // Fetch treats a missing HTTP cache partition as `no-store`.
                // Preserve `only-if-cached` so it still fails without network
                // access rather than being rewritten into a normal request.
                request.set_cache_mode(crate::CacheMode::NoStore);
            }
        }
        log::trace!(
            "request prepared: {} scheme={:?} cacheable={}",
            crate::log_url::loggable(validated_url.url.as_str()),
            validated_url.scheme,
            request.is_cacheable_method()
        );
        Ok((prepared, validated_url.scheme))
    }
}

/// Builds the fail-closed error for an absent browser capability.
fn missing_capability(request: &Request, capability: &'static str) -> RequestError {
    log::warn!(
        "request needs {capability}, but none is installed: {}",
        crate::log_url::loggable(request.url())
    );
    RequestError::UnsupportedFeature(capability)
}
