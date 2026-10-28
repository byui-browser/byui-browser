//! Browser-owned `blob:` scheme handling.

use reqwest::Url;

use crate::{
    api::{error::RequestError, request::Request, response::Response},
    engine::RequestController,
    policy::FetchScheme,
};

impl RequestController {
    /// Resolves a browser-owned `blob:` URL through the installed provider.
    ///
    /// The provider owns object-URL lookup, revocation, creator-origin
    /// association, and resource lifetime. The Fetch engine only validates
    /// and exposes the provider-neutral result through the shared response
    /// pipeline. Blob responses never use the HTTP cache or cookie services.
    pub(crate) async fn fetch_blob(
        &self,
        request: Request,
        url: Url,
    ) -> Result<Response, RequestError> {
        super::require_read_method(&request, "blob scheme method")?;
        let provider = self
            .inner
            .providers
            .blob()
            .ok_or(RequestError::MissingProvider("blob"))?;
        self.finish_provider_request(
            &request,
            FetchScheme::Blob,
            provider.resolve(&request, &url),
        )
        .await
    }
}
