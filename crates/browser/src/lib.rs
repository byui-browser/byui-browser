//! Browser-side composition of HTML, JavaScript, and Web APIs.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use html::Query;
use js::{JsResult, Realm, Value};
use net::{Request, RequestController, SchemeProviders};
use security::{
    RendererSecurityContext,
    file_access::{FileAccessCapability, FileAccessError, TrustedDirectorySelection},
};
use storage::{
    OriginStorage, PersistentOriginStorage, SharedStorage, StorageError, shared_storage,
};
use webapis::local_storage::register_local_storage;
use webapis::{
    AlertSink, ConsoleSink, Document, register_alert, register_document, register_fetch,
    register_print,
};

pub mod byui_provider;
pub mod fetch_services;
pub mod file_provider;
pub mod refresh;

/// Browser-owned resource limits for one local-file session.
///
/// The limits bound the length of each individual file, the total response
/// bytes served by a session, the duration of each read, and aggregate
/// buffering between blocking readers and response consumers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileAccessLimits {
    /// Maximum number of bytes in one local file admitted for `GET` or `HEAD`.
    pub max_file_size: u64,
    /// Maximum cumulative response-body bytes delivered by this session.
    /// `HEAD` responses consume no bytes from this budget.
    pub max_session_bytes: u64,
    /// Maximum elapsed time for setup and streaming of one admitted `GET`.
    pub max_read_duration: Duration,
    /// Maximum aggregate bytes queued between blocking local-file readers and
    /// response-body consumers for this session.
    pub max_buffered_bytes: u64,
    /// Optional lifetime for each issued local-file grant. Expiry cancels
    /// pending and live work through the normal revocation path.
    pub grant_lifetime: Option<Duration>,
}

/// Browser-owned interpretation policy for responses from a local-file session.
///
/// The currently supported static mode permits only extension-classified HTML,
/// CSS, and passive raster images. It does not authorize JavaScript, SVG,
/// unknown binary data, or local-file subresources; a future script-capable
/// mode requires document/container identity enforcement end to end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileAccessContentPolicy {
    /// Render only static, top-level documents and passive response types.
    StaticDocument,
}

impl Default for FileAccessLimits {
    /// Uses a conservative 64 MiB maximum individual file size.
    fn default() -> Self {
        Self {
            max_file_size: 64 * 1024 * 1024,
            max_session_bytes: 256 * 1024 * 1024,
            max_read_duration: Duration::from_secs(30),
            max_buffered_bytes: 512 * 1024,
            grant_lifetime: None,
        }
    }
}

/// Explicit configuration for the browser's local-file capability.
#[derive(Clone)]
pub struct FileAccessConfiguration {
    capability: Arc<FileAccessCapability>,
    grants: file_provider::FileAccessGrantSource,
    navigation_authorizations: Arc<file_provider::FileNavigationAuthorizations>,
    limits: FileAccessLimits,
    content_policy: FileAccessContentPolicy,
}

impl FileAccessConfiguration {
    /// Creates a local-file configuration from trusted platform selections.
    ///
    /// This is the browser's filesystem-capability boundary. The platform team
    /// creates each [`TrustedDirectorySelection`] from one user-approved
    /// selection operation. Selection construction verifies that its canonical
    /// path and retained directory handle identify the same directory; this
    /// API then consumes the selection without reopening its pathname.
    /// `limits` is browser-owned and is never derived from web content or
    /// caller-controlled request metadata.
    pub fn from_trusted_directory_selections(
        selected_directories: impl IntoIterator<Item = TrustedDirectorySelection>,
        limits: FileAccessLimits,
    ) -> Result<Self, FileAccessError> {
        if limits.max_file_size == 0
            || limits.max_session_bytes == 0
            || limits.max_read_duration.is_zero()
            || limits.max_buffered_bytes == 0
            || limits
                .grant_lifetime
                .is_some_and(|lifetime| lifetime.is_zero())
        {
            return Err(FileAccessError::InvalidPath);
        }
        let capability = Arc::new(FileAccessCapability::from_trusted_directory_selections(
            selected_directories,
        )?);
        let navigation_authorizations =
            Arc::new(file_provider::FileNavigationAuthorizations::new());
        let grants = file_provider::FileAccessGrantSource::new(
            capability.issue_grant(),
            Arc::clone(&navigation_authorizations),
            limits.grant_lifetime,
        );
        Ok(Self {
            capability,
            grants,
            navigation_authorizations,
            limits,
            content_policy: FileAccessContentPolicy::StaticDocument,
        })
    }

    /// Returns the browser-owned grant source for this configuration.
    ///
    /// The browser or platform owner may revoke this source when its
    /// user-approved permission expires. Revocation denies future opens and
    /// cancels pending setup plus live response bodies from this session.
    pub fn grant_source(&self) -> file_provider::FileAccessGrantSource {
        self.grants.clone()
    }

    /// Returns the interpretation policy enforced for this local-file session.
    pub fn content_policy(&self) -> FileAccessContentPolicy {
        self.content_policy
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
        if !self.grants.is_active() {
            return Err(file_provider::FileNavigationError::SessionRevoked);
        }
        let mut request = Request::get(url);
        request
            .set_mode(net::RequestMode::Navigate)
            .expect("GET requests may always use navigation mode");
        self.navigation_authorizations.authorize(request.id())?;
        Ok(request)
    }

    /// Ends this local-file session after a navigation-away or tab close.
    ///
    /// The operation revokes future file opens, clears pending navigations,
    /// and cancels setup plus live response bodies owned by this session.
    pub fn close(&self) {
        self.grants.revoke();
    }

    /// Revokes this local-file session when its document navigates away.
    ///
    /// This is equivalent to [`Self::close`]. It is a separate browser-facing
    /// operation so navigation code can express its lifecycle transition.
    pub fn navigation_away(&self) {
        self.close();
    }

    /// Ends the local-file session during explicit browser shutdown.
    ///
    /// This cancels pending and live work, clears unused navigation
    /// authorizations, and records a path-free shutdown audit event.
    pub fn shutdown(&self) {
        self.grants.shutdown();
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
            file_access.limits,
            file_access.content_policy,
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

#[cfg(not(target_os = "windows"))]
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
