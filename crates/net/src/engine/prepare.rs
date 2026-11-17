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

        if let Some(services) = &self.inner.services {
            services.check_request(request)?;
            if validated_url.scheme != crate::policy::FetchScheme::File
                && request.service_workers_mode() == ServiceWorkersMode::All
                && services.service_worker(request)? == ServiceWorkerDecision::Intercept
            {
                log::warn!(
                    "service-worker interception is unsupported: {}",
                    crate::log_url::loggable(request.url())
                );
                return Err(RequestError::UnsupportedFeature(
                    "service-worker interception",
                ));
            }
        } else if request.environment().origin.is_some()
            || request.credentials_mode() == CredentialsMode::Include
            || request.destination() != RequestDestination::Empty
            || request.initiator() != InitiatorType::Other
        {
            log::warn!(
                "request needs browser services but none are installed: {}",
                crate::log_url::loggable(request.url())
            );
            return Err(RequestError::UnsupportedFeature("browser services"));
        }

        if validated_url.scheme.is_http()
            && let Some(services) = &self.inner.services
        {
            request.set_cache_partition(services.cache_partition(request)?);
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
