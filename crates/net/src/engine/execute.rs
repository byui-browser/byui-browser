//! Fetch execution and response delivery.

use crate::{
    api::{diagnostics::NetworkPhase, error::RequestError, request::Request, response::Response},
    engine::RequestController,
};

impl RequestController {
    /// Validates and executes one browser request.
    ///
    /// The returned headers are available before the body is consumed. Body
    /// chunks are delivered as the transport receives them. A network response
    /// with bytes is inserted into the cache only after the caller consumes the
    /// stream to completion; dropped or failed streams are not cached. Null
    /// bodies may be cached as soon as headers arrive.
    pub async fn fetch(&self, request: Request) -> Result<Response, RequestError> {
        let original_url = request.url().to_owned();
        let original_url = reqwest::Url::parse(&original_url).ok();
        if let Some(url) = &original_url {
            self.observe(&request, NetworkPhase::Started, url, None, false);
        }
        log::debug!(
            "fetch start: {} {}",
            request.method(),
            crate::log_url::loggable(request.url())
        );
        let (request, validated_url) = self.prepare_request(request).inspect_err(|error| {
            log::debug!(
                "fetch rejected during preparation: {}",
                crate::log_url::loggable_error(error)
            );
        })?;
        self.observe(
            &request,
            NetworkPhase::Prepared,
            &validated_url.url,
            None,
            false,
        );
        let url = validated_url.url;
        let method = request.method().clone();
        let request_id = request.id();
        let redirect_count = request.redirect_count;
        let result = self
            .fetch_scheme(request, url.clone(), validated_url.scheme)
            .await;
        match &result {
            Ok(response) => {
                self.observe_parts(
                    request_id,
                    NetworkPhase::Completed,
                    url.as_str(),
                    Some(response.status),
                    response.from_cache,
                    redirect_count,
                );
                log::debug!(
                    "fetch done: {method} {} -> {} ({:?})",
                    crate::log_url::loggable(url.as_str()),
                    response.status,
                    response.response_type
                )
            }
            Err(RequestError::Aborted) => {
                self.observe_parts(
                    request_id,
                    NetworkPhase::Aborted,
                    url.as_str(),
                    None,
                    false,
                    redirect_count,
                );
                log::debug!(
                    "fetch aborted: {method} {}",
                    crate::log_url::loggable(url.as_str())
                )
            }
            // Debug, not error: the caller (page load, subresource, page script)
            // decides how serious a failed fetch is and reports it once.
            Err(error) => {
                self.observe_parts(
                    request_id,
                    if matches!(error, RequestError::Aborted) {
                        NetworkPhase::Aborted
                    } else {
                        NetworkPhase::Failed
                    },
                    url.as_str(),
                    None,
                    false,
                    redirect_count,
                );
                log::debug!(
                    "fetch failed: {method} {}: {}",
                    crate::log_url::loggable(url.as_str()),
                    crate::log_url::loggable_error(error)
                )
            }
        }
        result
    }

    /// Executes a request with a response-shaped Fetch network-error boundary.
    ///
    /// Pre-exposure failures become [`Response::network_error`]. Abort remains
    /// a typed error because cancellation rejects the Rust future, while body
    /// stream failures remain attached to an already delivered response body.
    pub async fn fetch_response(&self, request: Request) -> Result<Response, RequestError> {
        match self.fetch(request).await {
            Ok(response) => Ok(response),
            Err(RequestError::Aborted) => Err(RequestError::Aborted),
            Err(error) => {
                log::debug!(
                    "normalizing fetch failure to network error: {}",
                    crate::log_url::loggable_error(&error)
                );
                Ok(Response::network_error())
            }
        }
    }
}
