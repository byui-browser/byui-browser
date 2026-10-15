//! Browser-owned `file:` scheme handling.

use reqwest::Url;

use crate::{
    api::{error::RequestError, request::Request, response::Response},
    engine::RequestController,
    policy::FetchScheme,
};

impl RequestController {
    /// Resolves a browser-authorized `file:` URL through the installed provider.
    ///
    /// Filesystem access, path normalization, sandbox policy, and resource
    /// lifetime belong to the browser-owned provider. The engine only routes
    /// the request and sends the provider result through shared response
    /// validation and filtering. File responses never use HTTP transport,
    /// cookies, or the HTTP cache.
    pub(crate) async fn fetch_file(
        &self,
        request: Request,
        url: Url,
    ) -> Result<Response, RequestError> {
        let provider = self
            .inner
            .providers
            .file()
            .ok_or(RequestError::MissingProvider("file"))?;
        let resource = provider.open(&request, &url).await?;
        self.finalize_scheme_resource(&request, resource, FetchScheme::File)
    }
}
