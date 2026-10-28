//! `PageProvider` implementation for browser-owned embedded resources.

use std::sync::Arc;

use net::{
    HeaderName, HeaderValue, Origin, PageProvider, ProviderErrorKind, ProviderFuture, Request,
    RequestError, SchemeBody, SchemeResource, Url,
};

use super::registry::find_resource;

/// Resolves browser-owned pages and their static subresources from `byui:` URLs.
///
/// The provider does not read from the runtime filesystem. The registry
/// supplies compile-time embedded resources, and `net` remains responsible
/// for response validation and public response filtering.
#[derive(Clone, Copy, Debug, Default)]
pub struct BrowserPageProvider;

impl BrowserPageProvider {
    /// Creates a provider for the browser's embedded pages.
    pub const fn new() -> Self {
        Self
    }
}

impl PageProvider for BrowserPageProvider {
    /// Resolves a supported `byui:` page or static subresource.
    fn resolve<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let resource = find_resource(url)
            .map(|definition| resource(request, url.clone(), definition))
            .ok_or(RequestError::Provider {
                provider: "byui page",
                kind: ProviderErrorKind::ResourceMissing,
            });

        Box::pin(async move { resource })
    }
}

/// Builds provider-neutral metadata for one embedded resource.
fn resource(
    request: &Request,
    url: Url,
    definition: &super::registry::ResourceDefinition,
) -> SchemeResource {
    let media_type = HeaderValue::from_static(definition.media_type);
    SchemeResource {
        url,
        status: 200,
        status_text: Some("OK".into()),
        headers: vec![(HeaderName::from_static("content-type"), media_type.clone())],
        media_type: Some(media_type),
        origin: Origin::opaque(),
        body: if request.method() == net::Method::HEAD {
            SchemeBody::NullWithLength(definition.body.len())
        } else {
            SchemeBody::StaticBytes(definition.body)
        },
    }
}

/// Creates the provider handle used by browser composition code.
pub fn provider() -> Arc<dyn PageProvider> {
    Arc::new(BrowserPageProvider::new())
}
