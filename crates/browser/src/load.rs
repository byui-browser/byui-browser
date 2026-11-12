//! Loading a page from an address: fetch the document, then the stylesheets
//! and scripts it references.

use std::{
    fmt,
    future::Future,
    io,
    sync::{Arc, Mutex},
    time::Instant,
};

use devtools::performance::{EventCategory, TimelineEvent};

use futures_util::future::join_all;
use net::{Request, RequestController, Url};
use security::{Origin, RendererSecurityContext, StoragePermission};

use crate::{FontResource, ImageResource, Page, PageResources, attribute, request_controller};

/// Why a page, or the main document of a page, could not be loaded.
///
/// Subresource failures never surface as a `LoadError`; the loader skips the
/// stylesheet or script and logs a warning instead.
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

/// A fetched resource body (text by default) and the URL it was finally
/// served from.
struct Fetched<B = String> {
    /// Final URL after redirects; the base for resolving relative links.
    url: Url,
    body: B,
}

/// Fetches `url` as text, failing on a non-2xx status.
async fn fetch_document(controller: &RequestController, url: Url) -> Result<Fetched, LoadError> {
    log::trace!("fetching {url}");
    let response = controller.fetch(Request::get(url.as_str())).await?;
    log::debug!("GET {url} -> {}", response.status);
    if !(200..300).contains(&response.status) {
        return Err(LoadError::Status {
            status: response.status,
            status_text: response.status_text,
        });
    }
    // The response URL is empty for filtered responses; keep the request URL.
    let final_url = Url::parse(&response.url).unwrap_or(url);
    let body = response.body.text().await?;
    log::debug!("fetched {final_url}: {} bytes", body.len());
    Ok(Fetched {
        url: final_url,
        body,
    })
}

/// Fetches `url` as raw bytes, failing on a non-2xx status.
async fn fetch_bytes(
    controller: &RequestController,
    url: Url,
) -> Result<Fetched<Vec<u8>>, LoadError> {
    log::trace!("fetching {url} as bytes");
    let response = controller.fetch(Request::get(url.as_str())).await?;
    log::debug!("GET {url} -> {}", response.status);
    if !(200..300).contains(&response.status) {
        return Err(LoadError::Status {
            status: response.status,
            status_text: response.status_text,
        });
    }
    let final_url = Url::parse(&response.url).unwrap_or(url);
    let body = response.body.bytes().await?;
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
/// load is skipped with a logged warning; it does not fail the page.
/// Every distinct `<img src>` is downloaded as bytes into
/// [`PageResources::images`]; images that fail to load are left out, which
/// layout shows as broken images. Once the stylesheets are in, each
/// top-level `@font-face` rule's first `src` URL that loads (relative to its
/// stylesheet's URL, skipping `local()` and unsupported `format()` hints) is
/// downloaded into [`PageResources::fonts`]. A page served over a non-`byui:` scheme may
/// not pull `byui:` or `file:` subresources.
///
/// Limitation: CSS `@import` inside stylesheets is not followed, CSS
/// background images are not fetched, `srcset`/`<picture>` sources are
/// ignored, and fonts are fetched without the CORS check browsers apply to
/// cross-origin fonts.
pub async fn load_page(controller: &RequestController, address: &str) -> Result<Page, LoadError> {
    load_page_timed(controller, address).await.0
}

/// [`load_page`], also returning a DevTools Performance timeline of the load.
///
/// The timeline holds one [`EventCategory::Network`] event per download (the
/// document, then each remote stylesheet and script, including failed ones)
/// and one [`EventCategory::Parse`] event for parsing the document's HTML to
/// discover subresources. It is returned even when the load fails, covering
/// the work done before the failure.
pub async fn load_page_timed(
    controller: &RequestController,
    address: &str,
) -> (Result<Page, LoadError>, Vec<TimelineEvent>) {
    let timeline = Mutex::new(Vec::new());
    let result = load_page_with(
        address,
        |url| fetch_document(controller, url),
        |url| fetch_bytes(controller, url),
        &timeline,
    )
    .await;
    (result, timeline.into_inner().unwrap_or_default())
}

/// Appends `event` to `timeline`.
fn push_event(timeline: &Mutex<Vec<TimelineEvent>>, event: TimelineEvent) {
    if let Ok(mut timeline) = timeline.lock() {
        timeline.push(event);
    }
}

/// `"Download <name>"`, naming `url` by its last path segment (such as
/// `style.css`), or the whole URL when its path has no file name.
fn download_label(url: &Url) -> String {
    let name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())
        .map_or_else(|| url.to_string(), str::to_owned);
    format!("Download {name}")
}

/// [`load_page`] with the fetch steps injected so tests need no network:
/// `fetch` downloads text (the document, stylesheets, scripts) and
/// `fetch_bytes` binary files (images). Each stage's timing is appended to
/// `timeline`.
async fn load_page_with<F, Fut, G, GFut>(
    address: &str,
    fetch: F,
    fetch_bytes: G,
    timeline: &Mutex<Vec<TimelineEvent>>,
) -> Result<Page, LoadError>
where
    F: Fn(Url) -> Fut,
    Fut: Future<Output = Result<Fetched, LoadError>>,
    G: Fn(Url) -> GFut,
    GFut: Future<Output = Result<Fetched<Vec<u8>>, LoadError>>,
{
    let started = Instant::now();
    let url = normalize_address(address).inspect_err(|error| {
        log::error!("page load failed for {:?}: {error}", address.trim());
    })?;
    log::info!("navigation started: {url}");
    let download_started = Instant::now();
    let main = fetch(url.clone()).await;
    push_event(
        timeline,
        TimelineEvent::since(
            EventCategory::Network,
            download_label(&url),
            download_started,
        ),
    );
    let main = main.inspect_err(|error| {
        log::error!("page load failed for {url}: {error}");
    })?;
    log::trace!("parsing document {}: {} bytes", main.url, main.body.len());
    let (document, parse) = TimelineEvent::measure(EventCategory::Parse, "Parse HTML", || {
        html::parse_raw_html(main.body.clone())
    });
    push_event(timeline, parse);

    let mut sheet_slots = Vec::new();
    let mut script_slots = Vec::new();
    let mut image_sources: Vec<String> = Vec::new();
    let nodes = (0..)
        .map(html::NodeID::new)
        .map_while(|id| document.node(id).map(|node| (id, node)));
    for (id, node) in nodes {
        let html::NodeKind::Element(element) = node.get_kind() else {
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
            "img" if *element.get_namespace() == html::Namespace::Html => {
                // Layout looks images up by the trimmed attribute text.
                let src = attribute(attributes, "src").unwrap_or_default().trim();
                if !src.is_empty() && !image_sources.iter().any(|known| known == src) {
                    image_sources.push(src.to_owned());
                }
            }
            "script" => script_slots.push(match attribute(attributes, "src") {
                Some(src) => Slot::Remote(subresource_url(&main.url, src)),
                None => Slot::Inline(inline()),
            }),
            _ => {}
        }
    }

    log::debug!(
        "subresources discovered in {}: {} stylesheets, {} scripts, {} images",
        main.url,
        sheet_slots.len(),
        script_slots.len(),
        image_sources.len()
    );
    let (stylesheets, scripts, images) = futures_util::future::join3(
        resolve_slots(
            &fetch,
            sheet_slots,
            "stylesheet",
            false,
            &main.url,
            timeline,
        ),
        resolve_slots(&fetch, script_slots, "script", true, &main.url, timeline),
        fetch_images(&fetch_bytes, &main.url, image_sources, timeline),
    )
    .await;
    let fonts = fetch_fonts(&fetch_bytes, &stylesheets, timeline).await;
    log::debug!(
        "subresources loaded: {} stylesheets, {} scripts, {} images, {} fonts",
        stylesheets.len(),
        scripts.len(),
        images.len(),
        fonts.len()
    );
    let stylesheets = stylesheets.into_iter().map(|(_, source)| source).collect();
    let scripts = scripts.into_iter().map(|(_, source)| source).collect();
    log::info!(
        "page loaded: {} ({} ms)",
        main.url,
        started.elapsed().as_millis()
    );
    Ok(Page {
        url: main.url.to_string(),
        html: main.body,
        stylesheets,
        scripts,
        resources: PageResources {
            images: images.into(),
            fonts: fonts.into(),
        },
    })
}

/// Downloads each image `src` (resolved against `base`) concurrently,
/// returning the ones that loaded in `sources` order. Failures are logged
/// and left out. Each download is appended to `timeline`.
async fn fetch_images<G, GFut>(
    fetch_bytes: &G,
    base: &Url,
    sources: Vec<String>,
    timeline: &Mutex<Vec<TimelineEvent>>,
) -> Vec<ImageResource>
where
    G: Fn(Url) -> GFut,
    GFut: Future<Output = Result<Fetched<Vec<u8>>, LoadError>>,
{
    let fetched = join_all(sources.into_iter().map(|src| async move {
        let url = match subresource_url(base, &src) {
            Ok(url) => url,
            Err(reason) => {
                log::warn!("skipping image {src:?}: {reason}");
                return None;
            }
        };
        let started = Instant::now();
        let result = fetch_bytes(url.clone()).await;
        push_event(
            timeline,
            TimelineEvent::since(EventCategory::Network, download_label(&url), started),
        );
        match result {
            Ok(fetched) => {
                log::debug!("fetched image {url}: {} bytes", fetched.body.len());
                Some(ImageResource {
                    src,
                    url: fetched.url.to_string(),
                    bytes: fetched.body.into(),
                })
            }
            Err(error) => {
                log::warn!("skipping image {url}: {error}");
                None
            }
        }
    }))
    .await;
    fetched.into_iter().flatten().collect()
}

/// Downloads the web fonts of every `@font-face` rule in `stylesheets`
/// (each paired with the URL its relative URLs resolve against), trying
/// each rule's loadable `src` URLs in order until one downloads. Rules are
/// fetched concurrently; results keep stylesheet and rule order. Each
/// download is appended to `timeline`.
async fn fetch_fonts<G, GFut>(
    fetch_bytes: &G,
    stylesheets: &[(Url, String)],
    timeline: &Mutex<Vec<TimelineEvent>>,
) -> Vec<FontResource>
where
    G: Fn(Url) -> GFut,
    GFut: Future<Output = Result<Fetched<Vec<u8>>, LoadError>>,
{
    let rules = stylesheets
        .iter()
        // Skip parsing sheets that cannot contain a rule.
        .filter(|(_, source)| source.to_ascii_lowercase().contains("@font-face"))
        .flat_map(|(base, source)| {
            css::font_face_rules(&css::parse_stylesheet(source))
                .into_iter()
                .map(move |rule| (base, rule))
        });
    let fetched = join_all(rules.map(|(base, rule)| async move {
        for source in rule
            .sources
            .iter()
            .filter(|source| source.is_loadable_url())
        {
            let css::FontSource::Url { url, .. } = source else {
                continue;
            };
            let url = match subresource_url(base, url) {
                Ok(url) => url,
                Err(reason) => {
                    log::warn!("skipping font {url:?} for {:?}: {reason}", rule.family);
                    continue;
                }
            };
            let started = Instant::now();
            let result = fetch_bytes(url.clone()).await;
            push_event(
                timeline,
                TimelineEvent::since(EventCategory::Network, download_label(&url), started),
            );
            match result {
                Ok(fetched) => {
                    log::debug!(
                        "fetched font {url} for {:?}: {} bytes",
                        rule.family,
                        fetched.body.len()
                    );
                    return Some(FontResource {
                        family: rule.family.clone(),
                        weight: rule.weight,
                        italic: rule.italic,
                        url: fetched.url.to_string(),
                        bytes: fetched.body.into(),
                    });
                }
                Err(error) => log::warn!("font {url} for {:?} failed: {error}", rule.family),
            }
        }
        log::warn!("no source of @font-face {:?} loaded", rule.family);
        None
    }))
    .await;
    fetched.into_iter().flatten().collect()
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
/// order, paired with its URL: the final response URL for a fetched slot,
/// `document` for inline ones. A failed slot is dropped, or kept as an empty
/// string when `keep_gap` is set so scripts stay aligned with their
/// `<script>` elements. Each download, successful or not, is appended to
/// `timeline`.
async fn resolve_slots<F, Fut>(
    fetch: &F,
    slots: Vec<Slot>,
    kind: &str,
    keep_gap: bool,
    document: &Url,
    timeline: &Mutex<Vec<TimelineEvent>>,
) -> Vec<(Url, String)>
where
    F: Fn(Url) -> Fut,
    Fut: Future<Output = Result<Fetched, LoadError>>,
{
    let fetched = join_all(slots.into_iter().map(|slot| async move {
        match slot {
            Slot::Inline(text) => Some((document.clone(), text)),
            Slot::Remote(Err(reason)) => {
                log::warn!("skipping {kind}: {reason}");
                None
            }
            Slot::Remote(Ok(url)) => {
                let started = Instant::now();
                let fetched = fetch(url.clone()).await;
                push_event(
                    timeline,
                    TimelineEvent::since(EventCategory::Network, download_label(&url), started),
                );
                match fetched {
                    Ok(fetched) => {
                        log::debug!("fetched {kind} {url}: {} bytes", fetched.body.len());
                        Some((fetched.url, fetched.body))
                    }
                    Err(error) => {
                        log::warn!("skipping {kind} {url}: {error}");
                        None
                    }
                }
            }
        }
    }))
    .await;
    fetched
        .into_iter()
        .filter_map(|source| source.or_else(|| keep_gap.then(|| (document.clone(), String::new()))))
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
        resources: PageResources::default(),
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
        let origin = Origin::new(url.scheme(), host, url.port());
        log::debug!("security context derived: origin {origin:?}, storage allowed");
        return RendererSecurityContext::new(origin, StoragePermission::Allowed);
    }
    log::debug!("security context derived: opaque origin, storage denied");
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
    /// Downloads and parsing done during the load, for the DevTools
    /// Performance panel; see [`load_page_timed`].
    pub timeline: Vec<TimelineEvent>,
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
        log::debug!("page load scheduled: {typed}");
        self.runtime.spawn(async move {
            let shown = normalize_address(&address).map_or(typed, |url| url.to_string());
            let (result, timeline) = load_page_timed(&controller, &address).await;
            let shown = match &result {
                Ok(page) => page.url.clone(),
                Err(_) => shown,
            };
            on_done(Navigation {
                address: shown,
                result,
                timeline,
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
        let (result, requested, _) = load_timed(pages, address);
        (result, requested)
    }

    /// [`load`], also returning the Performance timeline.
    fn load_timed(
        pages: &[(&str, &str)],
        address: &str,
    ) -> (Result<Page, LoadError>, Vec<String>, Vec<TimelineEvent>) {
        let pages: HashMap<_, _> = pages.iter().copied().collect();
        let requested = Mutex::new(Vec::new());
        let timeline = Mutex::new(Vec::new());
        let result = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(load_page_with(
                address,
                |url| serve(&pages, &requested, url),
                |url| async {
                    let fetched = serve(&pages, &requested, url).await?;
                    Ok(Fetched {
                        url: fetched.url,
                        body: fetched.body.into_bytes(),
                    })
                },
                &timeline,
            ));
        (
            result,
            requested.into_inner().unwrap(),
            timeline.into_inner().unwrap(),
        )
    }

    #[test]
    fn timeline_records_each_download_and_the_html_parse() {
        let (result, _, timeline) = load_timed(
            &[
                (
                    "https://example.test/",
                    r#"<link rel="stylesheet" href="style.css"><style>p{}</style><script src="app.js"></script><script src="gone.js"></script>"#,
                ),
                ("https://example.test/style.css", "p { color: red }"),
                ("https://example.test/app.js", "1"),
            ],
            "https://example.test/",
        );
        assert!(result.is_ok());
        let mut labels: Vec<_> = timeline
            .iter()
            .map(|event| (event.category, event.label.as_str()))
            .collect();
        labels.sort_by_key(|(_, label)| *label);
        assert_eq!(
            labels,
            [
                (EventCategory::Network, "Download app.js"),
                (EventCategory::Network, "Download gone.js"),
                (EventCategory::Network, "Download https://example.test/"),
                (EventCategory::Network, "Download style.css"),
                (EventCategory::Parse, "Parse HTML"),
            ],
            "inline <style> is not a download; failed downloads still appear"
        );
    }

    #[test]
    fn images_are_fetched_once_per_src_and_failures_are_skipped() {
        let (result, requested) = load(
            &[
                (
                    "https://example.test/dir/page.html",
                    r#"<img src=" a.png "><img src="a.png"><img src="gone.png"><img><img src="">
                       <svg><img src="not-html.png"></svg><img src="/top.svg">"#,
                ),
                ("https://example.test/dir/a.png", "PNG"),
                ("https://example.test/top.svg", "<svg/>"),
            ],
            "https://example.test/dir/page.html",
        );
        let page = result.expect("page loads");
        let images: Vec<_> = page
            .resources
            .images
            .iter()
            .map(|image| (image.src.as_str(), image.url.as_str(), &image.bytes[..]))
            .collect();
        assert_eq!(
            images,
            [
                ("a.png", "https://example.test/dir/a.png", &b"PNG"[..]),
                ("/top.svg", "https://example.test/top.svg", &b"<svg/>"[..]),
            ]
        );
        let image_requests = requested
            .iter()
            .filter(|url| url.ends_with(".png") || url.ends_with(".svg"))
            .count();
        assert_eq!(image_requests, 3, "{requested:?}");
    }

    #[test]
    fn font_face_sources_resolve_against_their_stylesheet_and_fall_back() {
        let (result, requested) = load(
            &[
                (
                    "https://example.test/page.html",
                    r#"<link rel="stylesheet" href="css/site.css">
                       <style>@font-face { font-family: "B"; font-weight: bold;
                         src: local(B), url(missing.woff), url(x.eot) format("embedded-opentype"),
                              url(b.ttf) format("truetype") }</style>"#,
                ),
                (
                    "https://example.test/css/site.css",
                    "@font-face { font-family: A; font-style: italic;                      src: url(../f/a.woff2) format('woff2'), url(a.ttf) }",
                ),
                ("https://example.test/f/a.woff2", "wOF2"),
                ("https://example.test/b.ttf", "TTF"),
            ],
            "https://example.test/page.html",
        );
        let page = result.expect("page loads");
        let fonts: Vec<_> = page
            .resources
            .fonts
            .iter()
            .map(|font| {
                (
                    font.family.as_str(),
                    font.weight,
                    font.italic,
                    font.url.as_str(),
                )
            })
            .collect();
        assert_eq!(
            fonts,
            [
                ("A", (400, 400), true, "https://example.test/f/a.woff2"),
                ("B", (700, 700), false, "https://example.test/b.ttf"),
            ]
        );
        assert_eq!(&page.resources.fonts[1].bytes[..], b"TTF");
        assert!(requested.contains(&"https://example.test/missing.woff".to_owned()));
        assert!(
            !requested
                .iter()
                .any(|url| url.ends_with(".eot") || url.ends_with("a.ttf"))
        );
    }

    #[test]
    fn remote_pages_may_not_load_local_images() {
        let (result, requested) = load(
            &[("https://example.test/", r#"<img src="file:///etc/x.png">"#)],
            "https://example.test/",
        );
        assert!(result.unwrap().resources.images.is_empty());
        assert_eq!(requested, ["https://example.test/"]);
    }

    #[test]
    fn timeline_covers_a_failed_document_download() {
        let (result, _, timeline) = load_timed(&[], "https://example.test/missing.html");
        assert!(result.is_err());
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].label, "Download missing.html");
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
