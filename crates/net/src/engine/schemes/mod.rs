//! Scheme-specific Fetch dispatch and handlers.

mod about;
mod data;
mod http;
mod provider;

use crate::{
    api::{error::RequestError, request::PreparedRequest, response::Response},
    engine::RequestController,
    policy::FetchScheme,
};

impl RequestController {
    /// Dispatches a prepared request to its scheme-specific implementation.
    pub(crate) async fn fetch_scheme(
        &self,
        request: PreparedRequest,
        scheme: FetchScheme,
    ) -> Result<Response, RequestError> {
        log::debug!(
            "scheme handler chosen: {scheme:?} for {}",
            crate::log_url::loggable(request.url.as_str())
        );
        match scheme {
            FetchScheme::Http | FetchScheme::Https => self.fetch_http(request).await,
            FetchScheme::Data => self.fetch_data(request).await,
            FetchScheme::Blob | FetchScheme::File | FetchScheme::Byui => {
                self.fetch_provider_scheme(
                    request,
                    provider::ProviderScheme::for_fetch_scheme(scheme),
                )
                .await
            }
            FetchScheme::About => self.fetch_about(request).await,
        }
    }
}
