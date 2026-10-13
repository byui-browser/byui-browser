//! Fetch execution and response delivery.

use crate::{
    api::{error::RequestError, request::Request, response::Response},
    engine::RequestController,
    policy::FetchScheme,
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

        // Dispatcher seam for feature 4.4. HTTP(S) retains the existing
        // network implementation; local scheme handlers follow separately.
        match validated_url.scheme {
            FetchScheme::Http | FetchScheme::Https => {
                self.fetch_http(request, url, cacheable).await
            }
            FetchScheme::Data
            | FetchScheme::Blob
            | FetchScheme::File
            | FetchScheme::Byui
            | FetchScheme::About => Err(RequestError::UnsupportedFeature(
                "scheme-specific fetch dispatcher",
            )),
        }
    }
}
