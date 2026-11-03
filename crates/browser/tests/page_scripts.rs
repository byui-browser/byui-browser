use std::sync::{Arc, Mutex};

use browser::BrowserRuntime;
use security::{Origin, RendererSecurityContext, StoragePermission};
use webapis::{AlertSink, ConsoleSink};

#[derive(Default)]
struct TestConsole {
    messages: Mutex<Vec<String>>,
}

impl ConsoleSink for TestConsole {
    fn log(&self, message: &str) {
        self.messages.lock().unwrap().push(message.to_owned());
    }
}

#[derive(Default)]
struct TestAlert {
    messages: Mutex<Vec<String>>,
}

impl AlertSink for TestAlert {
    fn alert(&self, message: &str) {
        self.messages.lock().unwrap().push(message.to_owned());
    }
}

fn security_context() -> RendererSecurityContext {
    RendererSecurityContext::new(
        Origin::new("https", "example.com", None),
        StoragePermission::Allowed,
    )
}

#[test]
fn html_script_reaches_registered_web_api() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();

    let results = runtime
        .evaluate_page_scripts(
            "<main><script>print()</script></main>",
            console.clone(),
            alert.clone(),
            controller,
            security_context(),
        )
        .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::Undefined]);
    assert_eq!(
        console.messages.lock().unwrap().as_slice(),
        ["JS API Team Rocks!"]
    );
    assert!(alert.messages.lock().unwrap().is_empty());
}

#[test]
fn html_script_reaches_registered_alert_api() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();

    let results = runtime
        .evaluate_page_scripts(
            "<main><script>alert('team handshake')</script></main>",
            console,
            alert.clone(),
            controller,
            security_context(),
        )
        .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::Undefined]);
    assert_eq!(
        alert.messages.lock().unwrap().as_slice(),
        ["team handshake"]
    );
}

#[test]
fn html_script_reads_dom_attributes_through_document() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();

    let results = runtime
        .evaluate_page_scripts(
        "<main id='app' data-state='ready'><script>document.getElementById('app').getAttribute('DATA-STATE')</script></main>",
        console,
        alert,
        controller,
        security_context(),
    )
    .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::String("ready".to_owned())]);
}

#[test]
fn document_click_listener_runs_after_the_page_loads() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();

    let session = runtime
        .open_page_session(
            "<script>\
                function callback() { alert('Clicked') }\
                document.addEventListener('click', callback)\
             </script>",
            console,
            alert.clone(),
            controller,
            security_context(),
        )
        .expect("page session should load");

    assert!(alert.messages.lock().unwrap().is_empty());
    session
        .dispatch_click()
        .expect("document click should dispatch");
    assert_eq!(alert.messages.lock().unwrap().as_slice(), ["Clicked"]);
}

#[test]
fn linked_script_finds_page_elements_and_handles_their_clicks() {
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let page = browser::Page {
        url: String::new(),
        html: "<main><span>Hi</span><button id=\"btn\">Click Me</button>\
               <script src=\"script.js\"></script></main>"
            .into(),
        stylesheets: Vec::new(),
        scripts: vec![
            "function callback() { alert('Clicked') }\n\
             const btn = document.getElementById('btn')\n\
             btn.addEventListener('click', callback);"
                .into(),
        ],
    };

    let session = BrowserRuntime::in_memory()
        .open_page(
            &page,
            Arc::new(TestConsole::default()),
            alert.clone(),
            controller,
            security_context(),
        )
        .expect("linked script should find #btn");

    let document = html::parse_raw_html(page.html.clone());
    let button = document.get_element_by_id("btn").expect("#btn");
    let main = document
        .node(button)
        .and_then(|node| node.parent)
        .expect("main");
    session
        .dispatch_click_at(main)
        .expect("click outside the button should dispatch");
    assert!(alert.messages.lock().unwrap().is_empty());

    session
        .dispatch_click_at(button)
        .expect("button click should dispatch");
    assert_eq!(alert.messages.lock().unwrap().as_slice(), ["Clicked"]);
}
