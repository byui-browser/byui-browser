//! Scheme-specific Fetch dispatch and handlers.

mod about;
mod blob;
mod byui;
mod data;
mod file;
mod http;

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
            FetchScheme::Blob => self.fetch_blob(request, url).await,
            FetchScheme::File => self.fetch_file(request, url).await,
            FetchScheme::Byui => self.fetch_byui(request, url).await,
            FetchScheme::About => self.fetch_about(request, url).await,
        }
    }
}
