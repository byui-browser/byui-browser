//! Request validation and service preparation.

use crate::{
    api::{
        error::RequestError,
        request::{
            CacheMode, CredentialsMode, InitiatorType, Request, RequestDestination,
            ServiceWorkersMode,
        },
        services::ServiceWorkerDecision,
    },
    engine::RequestController,
    policy::ValidatedUrl,
};

impl RequestController {
    /// Validates a request and resolves browser-owned services before execution.
    pub(crate) fn prepare_request(
        &self,
        mut request: Request,
    ) -> Result<(Request, ValidatedUrl), RequestError> {
        if request.signal.is_aborted() {
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
        request.prepare_url(validated_url.url.clone());

        if let Some(services) = &self.inner.services {
            services.check_request(&request)?;
            if validated_url.scheme != crate::policy::FetchScheme::File
                && request.service_workers == ServiceWorkersMode::All
                && services.service_worker(&request)? == ServiceWorkerDecision::Intercept
            {
                log::warn!(
                    "service-worker interception is unsupported: {}",
                    crate::log_url::loggable(request.url())
                );
                return Err(RequestError::UnsupportedFeature(
                    "service-worker interception",
                ));
            }
        } else if request.context.environment.origin.is_some()
            || request.context.credentials == CredentialsMode::Include
            || request.destination != RequestDestination::Empty
            || request.initiator != InitiatorType::Other
        {
            log::warn!(
                "request needs browser services but none are installed: {}",
                crate::log_url::loggable(request.url())
            );
            return Err(RequestError::UnsupportedFeature("browser services"));
        }

        if validated_url.scheme.is_http()
            && request.cache_mode != CacheMode::NoStore
            && let Some(services) = &self.inner.services
        {
            request.cache_partition = Some(services.cache_partition(&request)?);
        }
        log::trace!(
            "request prepared: {} scheme={:?} cacheable={}",
            crate::log_url::loggable(validated_url.url.as_str()),
            validated_url.scheme,
            request.is_cacheable_method()
        );
        Ok((request, validated_url))
    }
}
