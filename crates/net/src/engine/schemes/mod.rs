//! Scheme-specific Fetch dispatch and handlers.

mod about;
mod blob;
mod byui;
mod data;
mod file;
mod http;

use crate::{
    api::{
        error::RequestError,
        providers::{ProviderFuture, SchemeResource},
        request::Request,
        response::Response,
    },
    engine::RequestController,
    policy::FetchScheme,
};

/// Rejects methods that cannot retrieve an immutable provider-owned resource.
///
/// Provider-backed local schemes expose registered or browser-authorized
/// resources; they do not implement mutation semantics. Enforce this before
/// provider lookup so unsupported methods cannot trigger provider side effects.
pub(super) fn require_read_method(
    request: &Request,
    feature: &'static str,
) -> Result<(), RequestError> {
    if matches!(
        request.method(),
        &reqwest::Method::GET | &reqwest::Method::HEAD
    ) {
        Ok(())
    } else {
        Err(RequestError::UnsupportedFeature(feature))
    }
}

/// Waits for a provider without allowing cancellation to leave its operation
/// or any resources held by its future running in the background.
async fn await_provider(
    request: &Request,
    operation: ProviderFuture<'_>,
) -> Result<SchemeResource, RequestError> {
    if request.signal().is_aborted() {
        return Err(RequestError::Aborted);
    }
    tokio::select! {
        _ = request.signal().cancelled() => Err(RequestError::Aborted),
        result = operation => result,
    }
}

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
