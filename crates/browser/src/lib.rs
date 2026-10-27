//! Browser-side composition of HTML, JavaScript, and Web APIs.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use html::Query;
use js::{JsResult, Realm, Value};
use net::{Request, RequestController, SchemeProviders};
use security::RendererSecurityContext;
use storage::{
    OriginStorage, PersistentOriginStorage, SharedStorage, StorageError, shared_storage,
};
use webapis::local_storage::register_local_storage;
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

/// Returns the browser-owned application-data directory for the current OS.
///
/// The renderer and webpage must never provide this path. Tests should inject
/// a temporary directory instead of using this process-wide location.
pub fn application_data_directory() -> io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        home_directory().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("BYUI Browser")
        })
    }

    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .map(PathBuf::from)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "application data directory is unavailable",
                )
            })?;
        Ok(root.join("BYUI Browser"))
    }

    #[cfg(target_os = "linux")]
    {
        let root = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                home_directory()
                    .ok()
                    .map(|home| home.join(".local").join("share"))
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "application data directory is unavailable",
                )
            })?;
        Ok(root.join("byui-browser"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        home_directory().map(|home| home.join(".byui-browser"))
    }
}

fn home_directory() -> io::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory is unavailable"))
}

/// Browser-owned state shared by renderer realms during one browser session.
#[derive(Debug, Clone)]
pub struct BrowserRuntime {
    storage: SharedStorage,
}

impl BrowserRuntime {
    /// Opens persistent storage below the browser application-data directory.
    ///
    /// The backend creates an additional `storage` directory below `root`.
    /// Callers must provide a browser-owned path, never a renderer-supplied
    /// path.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StorageError> {
        let storage = PersistentOriginStorage::open(root.as_ref().join("storage"))?;
        Ok(Self {
            storage: shared_storage(storage),
        })
    }

    /// Creates a runtime with application-lifetime storage for tests.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            storage: shared_storage(OriginStorage::new()),
        }
    }

    /// Parses a page, installs its Web APIs, and evaluates its scripts.
    ///
    /// The caller supplies the trusted renderer context created by browser
    /// navigation. JavaScript cannot choose the origin or storage key. The
    /// JavaScript engine's standard `localStorage` object syntax remains a
    /// follow-up; the registered host-function path is available now.
    pub fn evaluate_page_scripts(
        &self,
        html_source: &str,
        console: Arc<dyn ConsoleSink>,
        alert_sink: Arc<dyn AlertSink>,
        controller: Arc<RequestController>,
        security_context: RendererSecurityContext,
    ) -> JsResult<Vec<Value>> {
        let document = html::parse_raw_html(html_source.to_owned());
        let mut realm = Realm::new();
        register_print(&mut realm, console)?;
        register_alert(&mut realm, alert_sink)?;
        register_fetch(&mut realm, controller)?;
        register_document(&mut realm, Document::from_html_document(&document))?;
        register_local_storage(&mut realm, Arc::clone(&self.storage), security_context)?;

        document
            .script_sources()
            .iter()
            .map(|source| realm.evaluate_script(source))
            .collect()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use security::{Origin, RendererSecurityContext, StoragePermission};

    fn storage_context() -> RendererSecurityContext {
        RendererSecurityContext::new(
            Origin::new("https", "example.com", None),
            StoragePermission::Allowed,
        )
    }

    #[test]
    fn welcome_page_resolves_resources_from_html_links() {
        let page = welcome_page();

        assert!(page.html.contains("href=\"style.css\""));
        assert!(page.html.contains("src=\"script.js\""));
        assert!(page.css.contains("background-color"));
        assert!(page.js.contains("const message"));
    }

    #[test]
    fn runtime_connects_persistent_storage_to_the_local_storage_binding() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("byui-browser-runtime-{suffix}"));

        let first_runtime = BrowserRuntime::open(&root).expect("storage should open");
        let mut first_realm = Realm::new();
        register_local_storage(
            &mut first_realm,
            Arc::clone(&first_runtime.storage),
            storage_context(),
        )
        .expect("localStorage should register");
        assert_eq!(
            first_realm.call_global(
                "localStorage.setItem",
                &[Value::String("name".into()), Value::String("Nestor".into()),],
            ),
            Ok(Value::Undefined)
        );
        drop(first_runtime);

        let second_runtime = BrowserRuntime::open(&root).expect("storage should reopen");
        let mut second_realm = Realm::new();
        register_local_storage(
            &mut second_realm,
            Arc::clone(&second_runtime.storage),
            storage_context(),
        )
        .expect("localStorage should register after restart");
        assert_eq!(
            second_realm.call_global("localStorage.getItem", &[Value::String("name".into())],),
            Ok(Value::String("Nestor".into()))
        );

        std::fs::remove_dir_all(root).expect("test storage directory should be removable");
    }
}
