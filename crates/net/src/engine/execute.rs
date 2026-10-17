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
        let (request, validated_url, cacheable) = self.prepare_request(request)?;
        let url = validated_url.url;
        self.fetch_scheme(request, url, validated_url.scheme, cacheable)
            .await
    }
}
