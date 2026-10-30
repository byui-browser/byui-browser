//! Loading a page from an address: fetch the document, then the stylesheets
//! and scripts it references.

use std::{fmt, future::Future, io, sync::Arc};

use futures_util::future::join_all;
use net::{Request, RequestController, Url};
use security::{Origin, RendererSecurityContext, StoragePermission};

use crate::{Page, attribute, request_controller};

/// Why a page, or the main document of a page, could not be loaded.
///
/// Subresource failures never surface as a `LoadError`; the loader skips the
/// stylesheet or script and prints a diagnostic instead.
#[derive(Debug)]
pub enum LoadError {
    /// The typed address is empty or is not a valid URL, even after the
    /// default `https://` scheme was added.
    InvalidUrl {
        /// The address as the user typed it, trimmed.
        address: String,
        /// Why the URL parser rejected it.
        reason: String,
    },
    /// The request failed before a response was delivered, or its body could
    /// not be read (DNS, connection, TLS, unsupported scheme, ...).
    Request(net::RequestError),
    /// The server answered, but not with a 2xx status.
    Status {
        /// The HTTP status code of the response.
        status: u16,
        /// The canonical reason phrase, possibly empty.
        status_text: String,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl { address, reason } => {
                write!(f, "\"{address}\" is not a valid address: {reason}")
            }
            Self::Request(error) => write!(f, "the request failed: {error}"),
            Self::Status {
                status,
                status_text,
            } => write!(f, "the server responded with status {status} {status_text}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<net::RequestError> for LoadError {
    fn from(error: net::RequestError) -> Self {
        Self::Request(error)
    }
}

/// Turns text typed in the address bar into a URL.
///
/// Surrounding whitespace is trimmed. Input containing `://` is parsed as
/// written; anything else gets `https://` prepended. There is no search
/// fallback: text that is not a URL is an error.
pub fn normalize_address(input: &str) -> Result<Url, LoadError> {
    let address = input.trim();
    let candidate = if address.contains("://") {
        address.to_owned()
    } else {
        format!("https://{address}")
    };
    if address.is_empty() {
        return Err(LoadError::InvalidUrl {
            address: String::new(),
            reason: "the address is empty".into(),
        });
    }
    Url::parse(&candidate).map_err(|error| LoadError::InvalidUrl {
        address: address.to_owned(),
        reason: error.to_string(),
    })
}

/// A fetched resource body and the URL it was finally served from.
struct Fetched {
    /// Final URL after redirects; the base for resolving relative links.
    url: Url,
    body: String,
}

/// Fetches `url` as text, failing on a non-2xx status.
async fn fetch_document(controller: &RequestController, url: Url) -> Result<Fetched, LoadError> {
    let response = controller.fetch(Request::get(url.as_str())).await?;
    if !(200..300).contains(&response.status) {
        return Err(LoadError::Status {
            status: response.status,
            status_text: response.status_text,
        });
    }
    // The response URL is empty for filtered responses; keep the request URL.
    let final_url = Url::parse(&response.url).unwrap_or(url);
    let body = response.body.text().await?;
    Ok(Fetched {
        url: final_url,
        body,
    })
}

/// Loads the page at `address` through `controller`.
///
/// `address` is normalized with [`normalize_address`]. The main document must
/// load with a 2xx status or this returns a [`LoadError`]; callers show
/// [`error_page`] in that case.
///
/// The returned [`Page`] holds the document plus, in document order, every
/// `<link rel="stylesheet">` and `<style>` as [`Page::stylesheets`] and every
/// `<script>` as [`Page::scripts`]. Relative `href`/`src` values are resolved
/// against the document's final URL. A stylesheet or script that fails to
/// load is skipped with a diagnostic on stderr; it does not fail the page.
/// A page served over a non-`byui:` scheme may not pull `byui:` or `file:`
/// subresources.
///
/// Limitation: CSS `@import` inside stylesheets is not followed, and fonts
/// and images are not fetched.
pub async fn load_page(controller: &RequestController, address: &str) -> Result<Page, LoadError> {
    load_page_with(address, |url| fetch_document(controller, url)).await
}

/// [`load_page`] with the fetch step injected so tests need no network.
async fn load_page_with<F, Fut>(address: &str, fetch: F) -> Result<Page, LoadError>
where
    F: Fn(Url) -> Fut,
    Fut: Future<Output = Result<Fetched, LoadError>>,
{
    let main = fetch(normalize_address(address)?).await?;
    let document = html::parse_raw_html(main.body.clone());

    let mut sheet_slots = Vec::new();
    let mut script_slots = Vec::new();
    let nodes = (0..)
        .map(html::NodeID::new)
        .map_while(|id| document.node(id).map(|node| (id, node)));
    for (id, node) in nodes {
        let html::NodeKind::Element(element) = &node.kind else {
            continue;
        };
        let attributes = element.get_attributes();
        let inline = || document.text_content(id).unwrap_or_default();
        match element.get_name().as_str() {
            "link" => {
                let is_stylesheet = attribute(attributes, "rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                });
                if let (true, Some(href)) = (is_stylesheet, attribute(attributes, "href")) {
                    sheet_slots.push(Slot::Remote(subresource_url(&main.url, href)));
                }
            }
            "style" => sheet_slots.push(Slot::Inline(inline())),
            "script" => script_slots.push(match attribute(attributes, "src") {
                Some(src) => Slot::Remote(subresource_url(&main.url, src)),
                None => Slot::Inline(inline()),
            }),
            _ => {}
        }
    }

    let (stylesheets, scripts) = futures_util::future::join(
        resolve_slots(&fetch, sheet_slots, "stylesheet", false),
        resolve_slots(&fetch, script_slots, "script", true),
    )
    .await;
    Ok(Page {
        url: main.url.to_string(),
        html: main.body,
        stylesheets,
        scripts,
    })
}

/// A stylesheet or script as found in the document, before fetching.
enum Slot {
    Inline(String),
    /// `Err` holds a diagnostic for a link that could not be resolved.
    Remote(Result<Url, String>),
}

/// Resolves `reference` against `base`, rejecting local schemes a remote page
/// must not reach.
fn subresource_url(base: &Url, reference: &str) -> Result<Url, String> {
    let url = base.join(reference).map_err(|error| error.to_string())?;
    if base.scheme() != "byui" && matches!(url.scheme(), "byui" | "file") {
        return Err(format!(
            "{} pages may not load {} URLs",
            base.scheme(),
            url.scheme()
        ));
    }
    Ok(url)
}

/// Fetches the remote slots concurrently and returns every source in slot
/// order. A failed slot is dropped, or kept as an empty string when
/// `keep_gap` is set so scripts stay aligned with their `<script>` elements.
async fn resolve_slots<F, Fut>(
    fetch: &F,
    slots: Vec<Slot>,
    kind: &str,
    keep_gap: bool,
) -> Vec<String>
where
    F: Fn(Url) -> Fut,
    Fut: Future<Output = Result<Fetched, LoadError>>,
{
    let fetched = join_all(slots.into_iter().map(|slot| async move {
        match slot {
            Slot::Inline(text) => Some(text),
            Slot::Remote(Err(reason)) => {
                eprintln!("skipping {kind}: {reason}");
                None
            }
            Slot::Remote(Ok(url)) => match fetch(url.clone()).await {
                Ok(fetched) => Some(fetched.body),
                Err(error) => {
                    eprintln!("skipping {kind} {url}: {error}");
                    None
                }
            },
        }
    }))
    .await;
    fetched
        .into_iter()
        .filter_map(|source| source.or_else(|| keep_gap.then(String::new)))
        .collect()
}

/// Builds the built-in page shown when loading `address` fails.
///
/// Both the address and the error text are HTML-escaped, so a hostile URL
/// cannot inject markup into the error page.
pub fn error_page(address: &str, error: &LoadError) -> Page {
    Page {
        url: String::new(),
        html: format!(
            "<html><body><main><h1>This page could not be loaded</h1>\
             <p>{}</p><p>{}</p></main></body></html>",
            escape_html(address),
            escape_html(&error.to_string()),
        ),
        stylesheets: vec!["main { padding: 16px; } h1 { color: #8a2d2d; }".into()],
        scripts: Vec::new(),
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Derives the renderer security context for a document served from `url`.
///
/// `http`, `https` and `byui` URLs with a host get their own origin (scheme,
/// host, effective port) with storage allowed, so `https://a.example` and
/// `https://b.example` never share `localStorage`, and `byui://welcome` keeps
/// the welcome page's origin. Anything else (empty, unparsable, `file:`,
/// `data:`, hostless) gets an opaque origin with storage denied, which
/// matches no real origin.
pub fn security_context_for_url(url: &str) -> RendererSecurityContext {
    if let Ok(url) = Url::parse(url)
        && matches!(url.scheme(), "http" | "https" | "byui")
        && let Some(host) = url.host_str()
    {
        return RendererSecurityContext::new(
            Origin::new(url.scheme(), host, url.port()),
            StoragePermission::Allowed,
        );
    }
    RendererSecurityContext::new(Origin::new("opaque", "", None), StoragePermission::Denied)
}

/// The outcome of one [`PageLoader::load`] call.
#[derive(Debug)]
pub struct Navigation {
    /// The final post-redirect URL on success; otherwise the normalized URL
    /// that was requested, or the trimmed typed text when it was not a valid
    /// URL.
    pub address: String,
    /// The loaded page, or why loading failed.
    pub result: Result<Page, LoadError>,
}

/// Loads pages on a background Tokio runtime so a UI thread never blocks.
///
/// Dropping the loader shuts the runtime down and cancels loads in flight.
pub struct PageLoader {
    runtime: tokio::runtime::Runtime,
    controller: Arc<RequestController>,
}

impl PageLoader {
    /// Starts the background runtime and builds a request controller.
    ///
    /// Fails only if the operating system refuses to start the runtime.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            runtime: tokio::runtime::Runtime::new()?,
            controller: request_controller(None),
        })
    }

    /// Starts loading `address` and returns immediately.
    ///
    /// `on_done` runs once, on a runtime thread, with the [`Navigation`]. It
    /// must not block; typically it forwards the result to the UI thread.
    pub fn load(&self, address: &str, on_done: impl FnOnce(Navigation) + Send + 'static) {
        let controller = Arc::clone(&self.controller);
        let typed = address.trim().to_owned();
        let address = address.to_owned();
        self.runtime.spawn(async move {
            let shown = normalize_address(&address).map_or(typed, |url| url.to_string());
            let result = load_page(&controller, &address).await;
            let shown = match &result {
                Ok(page) => page.url.clone(),
                Err(_) => shown,
            };
            on_done(Navigation {
                address: shown,
                result,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use super::*;

    /// Serves `pages` by exact URL and records which URLs were requested.
    async fn serve(
        pages: &HashMap<&str, &str>,
        requested: &Mutex<Vec<String>>,
        url: Url,
    ) -> Result<Fetched, LoadError> {
        requested.lock().unwrap().push(url.to_string());
        match pages.get(url.as_str()) {
            Some(body) => Ok(Fetched {
                url,
                body: (*body).to_owned(),
            }),
            None => Err(LoadError::Status {
                status: 404,
                status_text: "Not Found".into(),
            }),
        }
    }

    fn load(pages: &[(&str, &str)], address: &str) -> (Result<Page, LoadError>, Vec<String>) {
        let pages: HashMap<_, _> = pages.iter().copied().collect();
        let requested = Mutex::new(Vec::new());
        let result = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(load_page_with(address, |url| {
                serve(&pages, &requested, url)
            }));
        (result, requested.into_inner().unwrap())
    }

    #[test]
    fn addresses_without_a_scheme_get_https() {
        assert_eq!(
            normalize_address("  example.com/a ").unwrap().as_str(),
            "https://example.com/a"
        );
        assert_eq!(
            normalize_address("http://example.com").unwrap().scheme(),
            "http"
        );
        assert_eq!(
            normalize_address("byui://welcome/page").unwrap().scheme(),
            "byui"
        );
        assert!(matches!(
            normalize_address("   "),
            Err(LoadError::InvalidUrl { .. })
        ));
        assert!(matches!(
            normalize_address("http://exa mple.com"),
            Err(LoadError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn relative_links_resolve_against_the_document_url() {
        let (page, requested) = load(
            &[
                (
                    "https://example.com/ladder/l3/",
                    "<link rel=\"stylesheet\" href=\"style.css\">\
                     <link rel=stylesheet href=\"/abs.css\">\
                     <script src=\"../app.js\"></script>",
                ),
                ("https://example.com/ladder/l3/style.css", "a{}"),
                ("https://example.com/abs.css", "b{}"),
                ("https://example.com/ladder/app.js", "1+1"),
            ],
            "example.com/ladder/l3/",
        );
        let page = page.unwrap();

        assert_eq!(page.stylesheets, ["a{}", "b{}"]);
        assert_eq!(page.scripts, ["1+1"]);
        assert!(requested.contains(&"https://example.com/ladder/app.js".to_owned()));
    }

    #[test]
    fn stylesheets_keep_document_order_across_links_and_style_elements() {
        let (page, _) = load(
            &[
                (
                    "https://example.com/",
                    "<head><link rel=\"stylesheet\" href=\"1.css\">\
                     <style>two{}</style>\
                     <link rel=\"icon\" href=\"icon.png\">\
                     <link rel=\"stylesheet\" href=\"3.css\"></head>\
                     <body><script>inline()</script><script src=\"s.js\"></script></body>",
                ),
                ("https://example.com/1.css", "one{}"),
                ("https://example.com/3.css", "three{}"),
                ("https://example.com/s.js", "external()"),
            ],
            "https://example.com/",
        );
        let page = page.unwrap();

        assert_eq!(page.stylesheets, ["one{}", "two{}", "three{}"]);
        assert_eq!(page.scripts, ["inline()", "external()"]);
    }

    #[test]
    fn a_failing_subresource_is_skipped_without_failing_the_page() {
        let (page, requested) = load(
            &[
                (
                    "https://example.com/",
                    "<link rel=\"stylesheet\" href=\"missing.css\">\
                     <link rel=\"stylesheet\" href=\"ok.css\">\
                     <script src=\"missing.js\"></script><script>kept()</script>",
                ),
                ("https://example.com/ok.css", "ok{}"),
            ],
            "https://example.com/",
        );
        let page = page.unwrap();

        assert_eq!(page.stylesheets, ["ok{}"]);
        // The failed script leaves an empty slot so indices match elements.
        assert_eq!(page.scripts, ["", "kept()"]);
        assert!(requested.contains(&"https://example.com/missing.css".to_owned()));
    }

    #[test]
    fn remote_pages_may_not_load_local_scheme_subresources() {
        let (page, requested) = load(
            &[(
                "https://example.com/",
                "<link rel=\"stylesheet\" href=\"byui://welcome/style.css\">\
                 <script src=\"file:///etc/passwd\"></script>",
            )],
            "https://example.com/",
        );
        let page = page.unwrap();

        assert!(page.stylesheets.is_empty());
        assert_eq!(page.scripts, [""]);
        assert_eq!(requested, ["https://example.com/"]);
    }

    #[test]
    fn main_document_failure_is_a_load_error_and_renders_an_error_page() {
        let (result, _) = load(&[], "https://example.com/gone");
        let error = result.unwrap_err();
        assert!(matches!(error, LoadError::Status { status: 404, .. }));

        let page = error_page("https://example.com/<b>", &error);
        assert!(page.html.contains("https://example.com/&lt;b&gt;"));
        assert!(page.html.contains("404"));
        assert!(!page.html.contains("<b>"));
    }

    #[test]
    fn page_url_is_the_final_document_url() {
        let (page, _) = load(&[("https://example.com/a/", "<p>hi</p>")], "example.com/a/");
        assert_eq!(page.unwrap().url, "https://example.com/a/");
    }

    #[test]
    fn different_https_origins_get_different_contexts() {
        let a = security_context_for_url("https://a.example/x");
        let b = security_context_for_url("https://b.example/x");
        assert_ne!(a.storage_key(), b.storage_key());
        assert_eq!(a, security_context_for_url("https://a.example/y?q#f"));
        assert_ne!(a, security_context_for_url("http://a.example/x"));
        assert_ne!(a, security_context_for_url("https://a.example:8443/x"));
    }

    #[test]
    fn welcome_keeps_its_context_and_remote_pages_do_not_get_it() {
        let welcome = RendererSecurityContext::new(
            Origin::new("byui", "welcome", None),
            StoragePermission::Allowed,
        );
        assert_eq!(security_context_for_url("byui://welcome/page"), welcome);
        assert_ne!(security_context_for_url("https://welcome/"), welcome);
        assert_ne!(security_context_for_url("https://example.com/"), welcome);
    }

    #[test]
    fn unsupported_origins_are_isolated_without_storage() {
        for url in ["", "not a url", "file:///etc/passwd", "data:text/html,hi"] {
            let context = security_context_for_url(url);
            assert!(!context.can_read_storage() && !context.can_write_storage());
            assert_eq!(context.origin().scheme(), "opaque");
        }
    }

    #[test]
    fn load_page_reads_the_embedded_welcome_page_through_the_controller() {
        let controller = request_controller(None);
        let page = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(load_page(&controller, "byui://welcome/page"))
            .expect("welcome page should load");
        assert!(page.stylesheets[0].contains("background-color"));
        assert!(page.scripts[0].contains("btn.addEventListener"));
    }

    #[test]
    fn unknown_byui_routes_report_a_request_error() {
        let controller = request_controller(None);
        let result = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(load_page(&controller, "byui://missing/page"));
        assert!(matches!(result, Err(LoadError::Request(_))));
    }
}
