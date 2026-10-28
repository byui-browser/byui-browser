//! Browser-owned `byui:` page handling.

use reqwest::Url;

use crate::{
    api::{error::RequestError, request::Request, response::Response},
    engine::RequestController,
    policy::FetchScheme,
};

impl RequestController {
    /// Resolves a registered `byui:` page through the installed page provider.
    ///
    /// Page lookup, path normalization, registration, and page lifetime belong
    /// to the browser-owned provider. The engine routes the request and sends
    /// the provider result through shared response validation and filtering;
    /// page requests never use HTTP transport, cookies, or the HTTP cache.
    pub(crate) async fn fetch_byui(
        &self,
        request: Request,
        url: Url,
    ) -> Result<Response, RequestError> {
        super::require_read_method(&request, "byui scheme method")?;
        let provider = self
            .inner
            .providers
            .page()
            .ok_or(RequestError::MissingProvider("byui page"))?;
        self.finish_provider_request(
            &request,
            FetchScheme::Byui,
            provider.resolve(&request, &url),
        )
        .await
    }
}
