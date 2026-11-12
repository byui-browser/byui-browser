//! Browser-service response processing.

use crate::api::Headers;
use crate::{
    api::{
        error::RequestError, request::Request, response::InternalResponse, services::ResponseInfo,
    },
    engine::RequestController,
};

impl RequestController {
    /// Applies shared response policy before public response exposure.
    ///
    /// This deliberately does not process cookies or cache entries: local
    /// schemes do not participate in either HTTP facility.
    pub(crate) fn finalize_response(
        &self,
        request: &Request,
        response: &mut InternalResponse,
    ) -> Result<(), RequestError> {
        self.finalize_response_with_type(request, response, None)
    }

    /// Applies shared policy to a manual redirect before exposing its opaque view.
    pub(crate) fn finalize_manual_redirect_response(
        &self,
        request: &Request,
        response: &mut InternalResponse,
    ) -> Result<(), RequestError> {
        // A manual redirect is intentionally opaque. It must still reach
        // browser-owned response policy, but running the CORS exposure check
        // first would turn a hidden cross-origin redirect into CorsDenied.
        response.response_type = crate::api::response::ResponseType::OpaqueRedirect;
        crate::policy::validate_response(request, response)?;
        self.approve_response(request, response)
    }

    /// Runs CORS validation and optionally replaces the public response class.
    fn finalize_response_with_type(
        &self,
        request: &Request,
        response: &mut InternalResponse,
        exposed_type: Option<crate::api::response::ResponseType>,
    ) -> Result<(), RequestError> {
        let classified_type = self.inner.cors.response_type(request, response)?;
        response.response_type = exposed_type.unwrap_or(classified_type);
        log::trace!(
            "response type classified: {:?} for {}",
            response.response_type,
            crate::log_url::loggable(request.url())
        );
        crate::policy::validate_response(request, response)?;
        self.approve_response(request, response)
    }

    /// Converts and finalizes a scheme-owned resource through the common path.
    pub(crate) fn finalize_scheme_resource(
        &self,
        request: &Request,
        resource: crate::api::providers::SchemeResource,
        scheme: crate::policy::FetchScheme,
    ) -> Result<crate::api::response::Response, RequestError> {
        let mut response = InternalResponse::from_scheme_resource(resource, request, scheme)?;
        self.finalize_response(request, &mut response)?;
        Ok(response.expose())
    }

    /// Gives browser-owned services the final opportunity to approve a response.
    pub(crate) fn approve_response(
        &self,
        request: &Request,
        response: &InternalResponse,
    ) -> Result<(), RequestError> {
        if let Some(services) = &self.inner.services {
            let info = ResponseInfo {
                status: response.status.as_u16(),
                status_text: response.status_text.clone(),
                url: response.url_list.last().cloned().unwrap_or_default(),
                headers: Headers::internal_response(&response.headers),
                response_origin: response.origin.clone(),
                request_origin: response.request_origin.clone(),
                request_mode: response.request_mode,
                response_type: response.response_type,
                redirect_count: response.redirect_count,
                from_cache: response.from_cache,
            };
            services
                .check_response(request, &info)
                .inspect_err(|error| {
                    log::warn!(
                        "response rejected by browser services: {} ({})",
                        crate::log_url::loggable(request.url()),
                        crate::log_url::loggable_error(error)
                    );
                })?;
            services.response_headers(request, &info);
        }
        Ok(())
    }
}
