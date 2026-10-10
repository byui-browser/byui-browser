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
};

impl RequestController {
    pub(crate) fn prepare_request(
        &self,
        mut request: Request,
    ) -> Result<(Request, reqwest::Url, bool), RequestError> {
        if request.signal.is_aborted() {
            return Err(RequestError::Aborted);
        }
        let url = self
            .inner
            .policy
            .validate_request(&request, self.inner.max_keepalive_body_size)?;
        request.prepare_url(url.clone());

        if let Some(services) = &self.inner.services {
            services.check_request(&request)?;
            if request.service_workers == ServiceWorkersMode::All
                && services.service_worker(&request)? == ServiceWorkerDecision::Intercept
            {
                return Err(RequestError::UnsupportedFeature(
                    "service-worker interception",
                ));
            }
        } else if request.context.environment.origin.is_some()
            || request.context.credentials == CredentialsMode::Include
            || request.destination != RequestDestination::Empty
            || request.initiator != InitiatorType::Other
        {
            return Err(RequestError::UnsupportedFeature("browser services"));
        }

        let cacheable = request.is_cacheable_method();
        if cacheable && request.cache_mode != CacheMode::NoStore {
            if let Some(services) = &self.inner.services {
                request.cache_partition = Some(services.cache_partition(&request)?);
            }
        }
        Ok((request, url, cacheable))
    }
}
