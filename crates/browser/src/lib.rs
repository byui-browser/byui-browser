//! Browser-side composition of HTML, JavaScript, and Web APIs.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

use html::Query;
use js::{JsResult, Realm, Value};
use net::{Request, RequestController, SchemeProviders};
use security::file_access::{FileAccessCapability, FileAccessError};
use webapis::{ConsoleSink, register_fetch, register_print};

pub mod byui_provider;
pub mod fetch_services;
pub mod file_provider;

/// Explicit configuration for the browser's local-file capability.
#[derive(Clone)]
pub struct FileAccessConfiguration {
    capability: Arc<FileAccessCapability>,
    grants: file_provider::FileAccessGrantSource,
    navigation_authorizations: Arc<file_provider::FileNavigationAuthorizations>,
}

impl FileAccessConfiguration {
    /// Creates a local-file configuration from roots approved by trusted
    /// browser or platform code.
    ///
    /// This is the browser's filesystem-capability boundary. Callers must use
    /// user-selected paths or an equivalent trusted embedder decision, never a
    /// path supplied by web content. The resulting policy and directory
    /// capabilities are created together and cannot be mixed with another
    /// root set.
    pub fn from_trusted_roots(
        allowed_roots: impl IntoIterator<Item = std::path::PathBuf>,
    ) -> Result<Self, FileAccessError> {
        let capability = Arc::new(FileAccessCapability::from_trusted_roots(allowed_roots)?);
        let navigation_authorizations =
            Arc::new(file_provider::FileNavigationAuthorizations::new());
        let grants = file_provider::FileAccessGrantSource::new(
            capability.issue_grant(),
            Arc::clone(&navigation_authorizations),
        );
        Ok(Self {
            capability,
            grants,
            navigation_authorizations,
        })
    }

    /// Returns the browser-owned grant source for this configuration.
    ///
    /// The browser or platform owner may revoke this source when its
    /// user-approved permission expires. Revocation denies future opens while
    /// an already-open body retains its file handle until it is finished,
    /// aborted, or dropped.
    pub fn grant_source(&self) -> file_provider::FileAccessGrantSource {
        self.grants.clone()
    }

    /// Creates a request authorized as one top-level local-file navigation.
    ///
    /// This browser-owned operation is required in addition to
    /// [`net::RequestMode::Navigate`]. The returned authorization is consumed
    /// when the file provider admits the request, cannot be replayed, and is
    /// cleared if this configuration's grant source is replaced or revoked.
    pub fn navigation_request(
        &self,
        url: impl Into<String>,
    ) -> Result<Request, file_provider::FileNavigationError> {
        let mut request = Request::get(url);
        request
            .set_mode(net::RequestMode::Navigate)
            .expect("GET requests may always use navigation mode");
        self.navigation_authorizations.authorize(request.id())?;
        Ok(request)
    }
}

/// Builds a browser request controller with the built-in page provider and an
/// optional explicitly configured local-file capability and browser services.
///
/// When `file_access` is `None`, no `file:` provider is installed. When it is
/// present, callers must supply a trusted [`FileAccessConfiguration`]; this
/// function does not infer roots from process state. Local-file navigations
/// must be created through [`FileAccessConfiguration::navigation_request`],
/// not by setting [`net::RequestMode::Navigate`] on an ordinary request.
/// Browser services use a conservative no-cookie, no-worker default and
/// permit requests carrying browser client context.
pub fn request_controller(file_access: Option<FileAccessConfiguration>) -> Arc<RequestController> {
    let mut providers = SchemeProviders::new().with_page(byui_provider::provider());
    if let Some(file_access) = file_access {
        providers = providers.with_file(file_provider::provider(
            file_access.capability,
            file_access.grants,
            file_access.navigation_authorizations,
        ));
    }
    Arc::new(
        RequestController::builder(net::Config::default())
            .with_fetch_services(Arc::new(fetch_services::BrowserFetchServices::default()))
            .with_scheme_providers(providers)
            .build()
            .expect("network controller should initialize"),
    )
}

/// A browser page and the resources referenced by its HTML source.
///
/// Resource text is loaded through the browser-owned `byui:` provider, so
/// native shells do not depend on the process working directory. The HTML
/// references are resolved and validated before the page is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// HTML document source.
    pub html: String,
    /// CSS source referenced by the document's stylesheet link.
    pub css: String,
    /// JavaScript source referenced by the document's script element.
    pub js: String,
}

/// Loads the welcome page and resolves its local stylesheet and script links.
pub fn welcome_page() -> Page {
    let controller = request_controller(None);
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let html = runtime
        .block_on(fetch_text(&controller, "byui://welcome/page"))
        .expect("welcome HTML should be available through byui:");
    let document = html::parse_raw_html(html.clone());
    let stylesheet_url = document
        .query("link")
        .into_iter()
        .find_map(|node| {
            node.kind.element_attributes().and_then(|attributes| {
                (attribute(attributes, "rel") == Some("stylesheet")
                    && attribute(attributes, "href") == Some("style.css"))
                .then_some("byui://welcome/style.css")
            })
        })
        .expect("welcome HTML must link style.css as its stylesheet");
    let script_url = document
        .query("script")
        .into_iter()
        .find_map(|node| {
            node.kind.element_attributes().and_then(|attributes| {
                (attribute(attributes, "src") == Some("script.js"))
                    .then_some("byui://welcome/script.js")
            })
        })
        .expect("welcome HTML must link script.js as its script");
    let css = runtime
        .block_on(fetch_text(&controller, stylesheet_url))
        .expect("welcome stylesheet should be available through byui:");
    let js = runtime
        .block_on(fetch_text(&controller, script_url))
        .expect("welcome script should be available through byui:");

    Page { html, css, js }
}

async fn fetch_text(
    controller: &RequestController,
    url: &str,
) -> Result<String, net::RequestError> {
    controller.fetch(Request::get(url)).await?.body.text().await
}

fn attribute<'a>(attributes: &'a [html::Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

trait ElementAttributes {
    fn element_attributes(&self) -> Option<&[html::Attribute]>;
}

impl ElementAttributes for html::NodeKind {
    fn element_attributes(&self) -> Option<&[html::Attribute]> {
        match self {
            html::NodeKind::Element(element) => Some(&element.attributes),
            _ => None,
        }
    }
}

/// Parses `html_source`, installs the browser's Web APIs, and evaluates each
/// inline page script in document order.
///
/// The scripts are evaluated by one [`Realm`], so every script uses the same
/// registered host-function path. DOM mutation and scheduling remain outside
/// this entry point until those APIs are supported by the engine.
pub fn evaluate_page_scripts(
    html_source: &str,
    console: Arc<dyn ConsoleSink>,
    controller: Arc<RequestController>,
) -> JsResult<Vec<Value>> {
    let document = html::parse_raw_html(html_source.to_owned());
    let mut realm = Realm::new();
    register_print(&mut realm, console)?;
    register_fetch(&mut realm, controller)?;

    document
        .script_sources()
        .iter()
        .map(|source| realm.evaluate_script(source))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_page_resolves_resources_from_html_links() {
        let page = welcome_page();

        assert!(page.html.contains("href=\"style.css\""));
        assert!(page.html.contains("src=\"script.js\""));
        assert!(page.css.contains("background-color"));
        assert!(page.js.contains("const message"));
    }
}
