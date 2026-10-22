//! Browser-side composition of HTML, JavaScript, and Web APIs.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

use html::Query;
use js::{JsResult, Realm, Value};
use net::{Request, RequestController, SchemeProviders};
use webapis::{
    AlertSink, ConsoleSink, Document, register_alert, register_document, register_fetch,
    register_print,
};

pub mod byui_provider;

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
    let controller = RequestController::builder(net::Config::default())
        .with_scheme_providers(SchemeProviders::new().with_page(byui_provider::provider()))
        .build()
        .expect("network controller should initialize");
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
    alert_sink: Arc<dyn AlertSink>,
    controller: Arc<RequestController>,
) -> JsResult<Vec<Value>> {
    let document = html::parse_raw_html(html_source.to_owned());
    let mut realm = Realm::new();
    register_print(&mut realm, console)?;
    register_alert(&mut realm, alert_sink)?;
    register_fetch(&mut realm, controller)?;
    register_document(&mut realm, Document::from_html_document(&document))?;

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
