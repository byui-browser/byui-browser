//! Browser-service response processing.

use crate::api::Headers;
use crate::{
    api::{
        error::RequestError, request::Request, response::InternalResponse, services::ResponseInfo,
    },
    engine::RequestController,
};

/// Exposure class selected while finalizing one internal response.
///
/// Normal responses are classified by CORS policy. A manual redirect is
/// deliberately opaque, but otherwise follows the same validation, browser
/// authorization, diagnostics, and exposure boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponseFinalization {
    /// Classify the response using the ordinary CORS policy.
    Standard,
    /// Finalize a manual redirect as an opaque redirect response.
    ManualRedirect,
}

impl RequestController {
    /// Applies shared response policy before public response exposure.
    ///
    /// This deliberately does not process cookies or cache entries: local
    /// schemes do not participate in either HTTP facility.
    pub(crate) fn finalize_response(
        &self,
        request: &Request,
        response: &mut InternalResponse,
        finalization: ResponseFinalization,
    ) -> Result<(), RequestError> {
        response.response_type = match finalization {
            ResponseFinalization::Standard => self.inner.cors.response_type(request, response)?,
            // Do not perform CORS classification first: Fetch must hide a
            // manual cross-origin redirect instead of rejecting it.
            ResponseFinalization::ManualRedirect => {
                crate::api::response::ResponseType::OpaqueRedirect
            }
        };
        log::trace!(
            "response type classified: {:?} for {}",
            response.response_type,
            crate::log_url::loggable(request.url())
        );
        crate::policy::validate_response(request, response)?;
        self.approve_response(request, response)
    }

    /// Finalizes one HTTP response before any HTTP-only post-processing.
    ///
    /// Cookie storage belongs immediately after the shared approval boundary:
    /// a denied response must not mutate browser storage. Cache insertion is
    /// deliberately left to the HTTP handler because a streaming body is only
    /// cacheable after its caller has consumed it successfully.
    pub(crate) fn finalize_http_response(
        &self,
        request: &Request,
        url: &reqwest::Url,
        response: &mut InternalResponse,
        finalization: ResponseFinalization,
    ) -> Result<(), RequestError> {
        self.finalize_response(request, response, finalization)?;
        self.process_response_cookies(request, url, response)
    }

    /// Converts and finalizes a scheme-owned resource through the common path.
    pub(crate) fn finalize_scheme_resource(
        &self,
        request: &Request,
        resource: crate::api::providers::SchemeResource,
        scheme: crate::policy::FetchScheme,
    ) -> Result<crate::api::response::Response, RequestError> {
        let mut response = InternalResponse::from_scheme_resource(resource, request, scheme)?;
        self.finalize_response(request, &mut response, ResponseFinalization::Standard)?;
        Ok(response.expose())
    }

    /// Gives approved HTTP response cookies to browser-owned storage policy.
    fn process_response_cookies(
        &self,
        request: &Request,
        url: &reqwest::Url,
        response: &mut InternalResponse,
    ) -> Result<(), RequestError> {
        if response.cookie_headers_processed || !request.credentials_allowed(url) {
            return Ok(());
        }
        if let Some(cookies) = &self.inner.cookies {
            let values: Vec<http::HeaderValue> = response
                .headers
                .get_all(http::header::SET_COOKIE)
                .iter()
                .cloned()
                .collect();
            cookies.store_set_cookie(request, &values)?;
            response.cookie_headers_processed = true;
        }
        Ok(())
    }

    /// Gives browser-owned services the final opportunity to approve a response.
    pub(crate) fn approve_response(
        &self,
        request: &Request,
        response: &InternalResponse,
    ) -> Result<(), RequestError> {
        if let Some(policy) = &self.inner.response_policy {
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
            policy.check_response(request, &info).inspect_err(|error| {
                log::warn!(
                    "response rejected by browser services: {} ({})",
                    crate::log_url::loggable(request.url()),
                    crate::log_url::loggable_error(error)
                );
            })?;
            policy.response_headers(request, &info);
        }
        Ok(())
    }
}
