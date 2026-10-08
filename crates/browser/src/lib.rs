//! Shared browser startup content and composition helpers.
//!
//! **Primary owners**: Browser UX Team (startup composition is cross-team).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use html::Query;

const WELCOME_HTML: &str = include_str!("../../../browser-pages/welcome/index.html");
const WELCOME_CSS: &str = include_str!("../../../browser-pages/welcome/style.css");
const WELCOME_JS: &str = include_str!("../../../browser-pages/welcome/script.js");

/// A browser page and the resources referenced by its HTML source.
///
/// Resource text is embedded at compile time so native shells do not depend on
/// the process working directory. The HTML references are still resolved and
/// validated before the page is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// HTML document source.
    pub html: &'static str,
    /// CSS source referenced by the document's stylesheet link.
    pub css: &'static str,
    /// JavaScript source referenced by the document's script element.
    pub js: &'static str,
}

/// Loads the welcome page and resolves its local stylesheet and script links.
pub fn welcome_page() -> Page {
    let document = html::parse_raw_html(WELCOME_HTML.to_owned());
    let stylesheet = document
        .query("link")
        .into_iter()
        .find_map(|node| {
            node.kind.element_attributes().and_then(|attributes| {
                (attribute(attributes, "rel") == Some("stylesheet")
                    && attribute(attributes, "href") == Some("style.css"))
                .then_some(WELCOME_CSS)
            })
        })
        .expect("welcome HTML must link style.css as its stylesheet");
    let script = document
        .query("script")
        .into_iter()
        .find_map(|node| {
            node.kind.element_attributes().and_then(|attributes| {
                (attribute(attributes, "src") == Some("script.js")).then_some(WELCOME_JS)
            })
        })
        .expect("welcome HTML must link script.js as its script");

    Page {
        html: WELCOME_HTML,
        css: stylesheet,
        js: script,
    }
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
