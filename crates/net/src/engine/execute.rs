//! Fetch execution and response delivery.

use crate::{
    api::{error::RequestError, request::Request, response::Response},
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
        let url = validated_url.url;
        let method = request.method().clone();
        let result = self
            .fetch_scheme(request, url.clone(), validated_url.scheme)
            .await;
        match &result {
            Ok(response) => log::debug!(
                "fetch done: {method} {} -> {} ({:?})",
                crate::log_url::loggable(url.as_str()),
                response.status,
                response.response_type
            ),
            Err(RequestError::Aborted) => log::debug!(
                "fetch aborted: {method} {}",
                crate::log_url::loggable(url.as_str())
            ),
            // Debug, not error: the caller (page load, subresource, page script)
            // decides how serious a failed fetch is and reports it once.
            Err(error) => log::debug!(
                "fetch failed: {method} {}: {}",
                crate::log_url::loggable(url.as_str()),
                crate::log_url::loggable_error(error)
            ),
        }
        result
    }
}
