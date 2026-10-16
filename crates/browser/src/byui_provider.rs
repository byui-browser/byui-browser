//! Browser-owned provider for compile-time `byui:` resources.
//!
//! This file is intentionally standalone until the browser composition root
//! wires it into `RequestController::with_services_and_providers`. The Fetch
//! engine owns the provider trait and response pipeline; this module owns the
//! browser page registry and its embedded resources.

use std::sync::Arc;

use net::{
    HeaderName, HeaderValue, Origin, PageProvider, ProviderErrorKind, ProviderFuture, Request,
    RequestError, SchemeBody, SchemeResource, Url,
};

const WELCOME_HTML: &str = include_str!("../../../browser-pages/welcome/index.html");
const WELCOME_CSS: &str = include_str!("../../../browser-pages/welcome/style.css");
const WELCOME_JS: &str = include_str!("../../../browser-pages/welcome/script.js");

/// Resolves browser-owned pages and their static subresources from `byui:` URLs.
///
/// The provider does not read from the runtime filesystem. Resources are
/// embedded from the repository's `browser-pages` directory at compile time,
/// so page lookup is deterministic and independent of the process working
/// directory.
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
    fn resolve<'a>(&'a self, _request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let result = match (url.host_str(), url.path()) {
            (Some("welcome"), "" | "/" | "/index.html" | "/page") => {
                Some(("text/html; charset=utf-8", WELCOME_HTML))
            }
            (Some("welcome"), "/style.css") => Some(("text/css; charset=utf-8", WELCOME_CSS)),
            (Some("welcome"), "/script.js") => Some(("text/javascript; charset=utf-8", WELCOME_JS)),
            _ => None,
        };

        let resource = result
            .map(|(media_type, body)| resource(url.clone(), media_type, body.as_bytes()))
            .ok_or(RequestError::Provider {
                provider: "byui page",
                kind: ProviderErrorKind::ResourceMissing,
            });

        Box::pin(async move { resource })
    }
}

/// Builds provider-neutral metadata for one embedded resource.
fn resource(url: Url, media_type: &'static str, body: &[u8]) -> SchemeResource {
    let media_type = HeaderValue::from_static(media_type);
    SchemeResource {
        url,
        status: 200,
        status_text: Some("OK".into()),
        headers: vec![(HeaderName::from_static("content-type"), media_type.clone())],
        media_type: Some(media_type),
        origin: Origin::opaque(),
        body: SchemeBody::Bytes(body.to_vec()),
    }
}

/// Creates the provider handle used by browser composition code.
pub fn provider() -> Arc<dyn PageProvider> {
    Arc::new(BrowserPageProvider::new())
}
