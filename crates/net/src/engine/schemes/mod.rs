//! Scheme-specific Fetch dispatch and handlers.

mod data;

use crate::{
    api::{error::RequestError, request::Request, response::Response},
    engine::RequestController,
    policy::FetchScheme,
};

impl RequestController {
    /// Dispatches a prepared request to its scheme-specific implementation.
    pub(crate) async fn fetch_scheme(
        &self,
        request: Request,
        url: reqwest::Url,
        scheme: FetchScheme,
        cacheable: bool,
    ) -> Result<Response, RequestError> {
        match scheme {
            FetchScheme::Http | FetchScheme::Https => {
                self.fetch_http(request, url, cacheable).await
            }
            FetchScheme::Data => self.fetch_data(request, url).await,
            FetchScheme::Blob => {
                if self.inner.providers.blob().is_none() {
                    Err(RequestError::MissingProvider("blob"))
                } else {
                    Err(RequestError::UnsupportedFeature("blob scheme handler"))
                }
            }
            FetchScheme::File => {
                if self.inner.providers.file().is_none() {
                    Err(RequestError::MissingProvider("file"))
                } else {
                    Err(RequestError::UnsupportedFeature("file scheme handler"))
                }
            }
            FetchScheme::Byui => {
                if self.inner.providers.page().is_none() {
                    Err(RequestError::MissingProvider("byui page"))
                } else {
                    Err(RequestError::UnsupportedFeature("byui scheme handler"))
                }
            }
            FetchScheme::About => Err(RequestError::UnsupportedFeature("about scheme handler")),
        }
    }
}
