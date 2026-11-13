//! Per-tab desktop state. Window resources and reusable services stay in App.

use chrome::{BrowserState, TabId};
use net::Url;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tiny_skia::Pixmap;

use crate::{
    content::{PageEvents, PageLayout},
    elements_panel::ElementsPanel,
    find_in_page::FindInPage,
    page_source::PageSource,
    toolbar::ChromeState,
};

/// Owns one document/session and its UI state. Moving it never reloads scripts.
pub(crate) struct TabState {
    pub id: TabId,
    pub document_id: u64,
    /// Cached display label from the loaded document, independent of address editing.
    pub title: String,
    pub chrome: ChromeState,
    pub browser_state: BrowserState,
    pub address: String,
    pub address_before_edit: String,
    pub loaded_page: Option<browser::Page>,
    pub page: Option<((u32, u32), Pixmap)>,
    pub page_layout: Option<PageLayout>,
    /// CSS pointer and focus state (`:hover`, `:active`, `:focus`) for this
    /// tab's document; reset when another document is shown.
    pub selector_state: css::SelectorState,
    /// Wall-clock origin of this document's CSS animation timeline, after its first layout.
    pub animation_clock: Option<Instant>,
    /// Next visible animation sample. Hidden tabs keep their clock without frame wakeups.
    pub next_animation_frame: Option<Instant>,
    pub scroll_y: f32,
    pub page_zoom: f64,
    pub page_events: PageEvents,
    pub scripts_evaluated: bool,
    pub find: Option<FindInPage>,
    pub find_reveal: bool,
    pub page_source: Option<PageSource>,
    pub elements: Option<ElementsPanel>,
    pub devtools_panel: Option<ElementsPanel>,
    pub network_log: devtools::network::NetworkLog,
    pub console: Arc<Mutex<devtools::Console>>,
    pub performance: Arc<Mutex<devtools::performance::PerformanceLog>>,
    pub history_rollback: Option<BrowserState>,
    pub audit_id: u64,
    pub navigation_id: u64,
    pub loading_id: u64,
    pub navigation_loading: Option<(u64, u64)>,
    pub reload_completion: Option<(u64, Box<dyn Fn() -> bool>)>,
}

impl TabState {
    pub fn new(id: TabId, inset: f32) -> Self {
        Self {
            id,
            document_id: 0,
            title: "Welcome".into(),
            chrome: ChromeState {
                window_controls_inset: inset,
                ..ChromeState::default()
            },
            browser_state: BrowserState::new(),
            address: "byui://welcome".into(),
            address_before_edit: "byui://welcome".into(),
            loaded_page: None,
            page: None,
            page_layout: None,
            selector_state: css::SelectorState::default(),
            animation_clock: None,
            next_animation_frame: None,
            scroll_y: 0.0,
            page_zoom: 1.0,
            page_events: PageEvents::new(),
            scripts_evaluated: false,
            find: None,
            find_reveal: false,
            page_source: None,
            elements: None,
            devtools_panel: None,
            network_log: devtools::network::NetworkLog::new(),
            console: Arc::new(Mutex::new(devtools::Console::new())),
            performance: Arc::new(Mutex::new(devtools::performance::PerformanceLog::new())),
            history_rollback: None,
            audit_id: 0,
            navigation_id: 0,
            loading_id: 0,
            navigation_loading: None,
            reload_completion: None,
        }
    }
}

/// Resolves a title once per document load; parsing never runs scripts or fetches.
pub(crate) fn page_title(page: &browser::Page, address: &str) -> String {
    let address = if page.url.is_empty() {
        address
    } else {
        &page.url
    };
    let parsed = Url::parse(address).ok();
    if parsed.as_ref().is_some_and(|url| {
        url.scheme() == "byui"
            && url.host_str() == Some("welcome")
            && matches!(url.path(), "" | "/" | "/page" | "/index.html")
    }) {
        return "Welcome".into();
    }
    let document = html::parse_raw_html(page.html.clone());
    if let Some((index,_))=document.nodes.iter().enumerate().find(|(_,node)| {
        matches!(node.get_kind(),html::NodeKind::Element(element) if element.get_name().eq_ignore_ascii_case("title"))
    }) {
        let text=document.text_content(html::NodeID::new(index)).unwrap_or_default();
        let title=text.split_whitespace().collect::<Vec<_>>().join(" ");
        let title:String=title.chars().filter(|c|!c.is_control()).collect();
        if !title.is_empty() {return title;}
    }
    if let Some(mut url) = parsed {
        if let Some(host) = url.host_str() {
            let host = host.strip_prefix("www.").unwrap_or(host);
            return match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_owned(),
            };
        }
        if url.scheme() == "data" {
            return "data: page".into();
        }
        if url.scheme() == "file"
            && let Some(name) = url
                .path_segments()
                .and_then(|mut parts| parts.rfind(|s| !s.is_empty()))
        {
            return name.to_owned();
        }
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }
    let fallback = address.split(['?', '#']).next().unwrap_or_default().trim();
    if fallback.is_empty() {
        chrome::DEFAULT_TAB_TITLE.into()
    } else {
        fallback.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(url: &str, html: &str) -> browser::Page {
        browser::Page {
            url: url.into(),
            html: html.into(),
            stylesheets: vec![],
            scripts: vec![],
            resources: Default::default(),
        }
    }
    #[test]
    fn welcome_aliases_always_use_welcome_without_changing_the_url() {
        for url in [
            "byui://welcome",
            "byui://welcome/",
            "byui://welcome/page",
            "byui://welcome/index.html?lang=en#top",
        ] {
            let page = page(url, "<title>Welcome to BYUI Browser</title>");
            assert_eq!(page_title(&page, url), "Welcome");
            assert_eq!(page.url, url);
        }
        let empty = page("", "<title>Welcome</title>");
        assert_eq!(page_title(&empty, "byui://welcome/page"), "Welcome");
        assert_eq!(page_title(&page("https://welcome/page", ""), ""), "welcome");
    }
    #[test]
    fn document_titles_decode_entities_collapse_whitespace_and_preserve_unicode() {
        let url = "https://www.example.com/long/path?q=value#fragment";
        let page = page(
            url,
            "<html><head><title>  Café &amp; Rust\n  Browser 😀 </title></head><body>body</body></html>",
        );
        assert_eq!(page_title(&page, url), "Café & Rust Browser 😀");
        assert_eq!(page.url, url);
    }
    #[test]
    fn missing_and_empty_titles_fall_back_to_short_addresses() {
        for html in ["<body>No title</body>", "<title> \n\t </title>"] {
            let url = "https://user:secret@www.example.com:8443/a/long/path?q=value#section";
            assert_eq!(page_title(&page(url, html), url), "example.com:8443");
        }
        assert_eq!(
            page_title(&page("file:///tmp/example.html?x=1", ""), ""),
            "example.html"
        );
        assert_eq!(page_title(&page("about:blank", ""), ""), "about:blank");
        assert_eq!(
            page_title(&page("data:text/html,body", ""), ""),
            "data: page"
        );
        assert_eq!(page_title(&page("", ""), ""), "New Tab");
    }
}
