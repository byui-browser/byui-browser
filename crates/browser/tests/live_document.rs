//! Script DOM changes reach the page session's live document, and the
//! built-in welcome page's controls work through it.

use std::sync::{Arc, Mutex};

use browser::{BrowserRuntime, Page, PageSession};
use webapis::{AlertSink, ConsoleSink};

struct Quiet;

impl ConsoleSink for Quiet {
    fn log(&self, _message: &str) {}
}

impl AlertSink for Quiet {
    fn alert(&self, _message: &str) {}
}

/// Opens `page` and returns its session plus every URL it navigates to.
fn open(page: &Page) -> (PageSession, Arc<Mutex<Vec<String>>>) {
    let navigations = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&navigations);
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let session = BrowserRuntime::in_memory()
        .open_page_with_navigation(
            page,
            Arc::new(Quiet),
            Arc::new(Quiet),
            controller,
            browser::security_context_for_url(&page.url),
            Arc::new(move |url, _behavior| captured.lock().unwrap().push(url.to_string())),
        )
        .expect("page scripts should run");
    (session, navigations)
}

fn attribute(document: &html::HTMLDocument, id: &str, name: &str) -> Option<String> {
    let node = document.get_element_by_id(id)?;
    match document.node(node)?.get_kind() {
        html::NodeKind::Element(element) => element
            .get_attributes()
            .iter()
            .find(|attribute| attribute.get_name() == name)
            .map(|attribute| attribute.get_value().clone()),
        _ => None,
    }
}

/// A page whose one script, `script`, runs after `html` is parsed.
fn inline_page(html: &str, script: &str) -> Page {
    Page {
        url: "https://example.com/".into(),
        html: html.into(),
        stylesheets: Vec::new(),
        scripts: vec![script.into()],
        resources: Default::default(),
    }
}

#[test]
fn script_attribute_changes_reach_the_live_document() {
    let (session, _) = open(&inline_page(
        "<div id=\"box\" class=\"off\"></div>",
        "document.getElementById('box').setAttribute('class', 'on')",
    ));

    let live = session.live_document();
    assert_eq!(attribute(&live, "box", "class").as_deref(), Some("on"));
    assert!(session.document_html().contains("class=\"on\""));
}

#[test]
fn click_listener_changes_reach_the_live_document_with_stable_node_ids() {
    let (session, _) = open(&inline_page(
        "<div id=\"box\" class=\"off\"><span id=\"label\">Hi</span></div>",
        "const box = document.getElementById('box');\n\
         box.addEventListener('click', (event) => {\n\
           event.target.setAttribute('data-hit', 'yes');\n\
           box.setAttribute('class', 'on');\n\
         });",
    ));
    let before = session.live_document();
    let label = before.get_element_by_id("label").expect("#label");

    session
        .dispatch_click_at(label)
        .expect("click should dispatch");

    let after = session.live_document();
    assert_eq!(attribute(&after, "box", "class").as_deref(), Some("on"));
    // `event.target` writes to the live tree too, not to a copy.
    assert_eq!(
        attribute(&after, "label", "data-hit").as_deref(),
        Some("yes")
    );
    // Node IDs survive the change, so layout hit tests stay valid.
    assert_eq!(after.get_element_by_id("label"), Some(label));
}

#[test]
fn welcome_page_oak_toggle_switches_the_tree_class() {
    let page = browser::welcome_page();
    let (session, navigations) = open(&page);
    let class = |session: &PageSession| attribute(&session.live_document(), "page", "class");
    assert_eq!(class(&session).as_deref(), Some("page tree-green"));

    let document = session.live_document();
    let blue = document.get_element_by_id("pick-blue").expect("#pick-blue");
    let green = document
        .get_element_by_id("pick-green")
        .expect("#pick-green");
    session.dispatch_click_at(blue).expect("blue click");
    assert_eq!(class(&session).as_deref(), Some("page tree-blue"));
    session.dispatch_click_at(green).expect("green click");
    assert_eq!(class(&session).as_deref(), Some("page tree-green"));
    assert!(navigations.lock().unwrap().is_empty());
}

#[test]
fn welcome_page_tiles_navigate_to_their_links() {
    let page = browser::welcome_page();
    let (session, navigations) = open(&page);
    let document = session.live_document();
    let tiles = [
        ("tile-byui", "https://www.byui.edu/"),
        ("tile-canvas", "https://byui.instructure.com/"),
        ("tile-library", "https://library.byui.edu/"),
        ("tile-email", "https://outlook.office.com/"),
        ("tile-code", "https://github.com/byui-browser/byui-browser"),
        (
            "tile-ladder",
            "https://willpatbarr.github.io/byui-browser-ladder/",
        ),
    ];
    for (id, _) in tiles {
        let tile = document.get_element_by_id(id).expect("tile");
        // Clicks usually land on the tile's text, so dispatch at a child.
        let child = document
            .node(tile)
            .and_then(|node| node.get_children().first().copied())
            .unwrap_or(tile);
        session.dispatch_click_at(child).expect("tile click");
    }
    let expected: Vec<String> = tiles.iter().map(|(_, url)| (*url).to_owned()).collect();
    assert_eq!(*navigations.lock().unwrap(), expected);
}
