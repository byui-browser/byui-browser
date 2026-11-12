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
fn page_script_runs_queued_microtasks_before_the_next_script() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let results = BrowserRuntime::in_memory()
        .evaluate_page_scripts(
            "<script>var order = ''; function callback() { order = order + 'a'; } queueMicrotask(callback)</script><script>order</script>",
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("page script should evaluate");
    assert_eq!(results[1], js::Value::String("a".into()));
}

#[test]
fn page_session_script_evaluates_through_a_microtask_checkpoint() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();
    let session = runtime
        .open_page(
            &browser::Page {
                html: "<main></main>".into(),
                url: "https://example.com/".into(),
                scripts: Vec::new(),
                stylesheets: Vec::new(),
                font_faces: Vec::new(),
            },
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("page session should open");
    session
        .evaluate_script(
            "var order = ''; function callback() { order = order + 'a'; } queueMicrotask(callback)",
        )
        .expect("script should evaluate");
    assert_eq!(
        session.evaluate_script("order"),
        Ok(js::Value::String("a".into()))
    );
}

#[test]
fn html_script_history_survives_document_registration() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let result = BrowserRuntime::in_memory()
        .evaluate_page_scripts(
            "<script>window.history().pushState('ready', ''); window.history().state</script>",
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("history should remain available after document registration");

    assert_eq!(result, vec![js::Value::String("ready".into())]);
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
fn linked_page_script_reads_its_navigation_url_from_window_location() {
    let console = Arc::new(TestConsole::default());
    let navigations = Arc::new(Mutex::new(Vec::new()));
    let captured_navigations = navigations.clone();
    let page = browser::Page {
        url: "https://example.com/path?q=term#section".into(),
        html: String::new(),
        stylesheets: Vec::new(),
        scripts: vec!["if (document.location === window.location && window.location.href === 'https://example.com/path?q=term#section') window.location.href = window.location.href".into()],
        font_faces: Vec::new(),
    };
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );

    BrowserRuntime::in_memory()
        .open_page_with_navigation(
            &page,
            console.clone(),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
            Arc::new(move |url, _behavior| {
                captured_navigations.lock().unwrap().push(url.to_string());
            }),
        )
        .expect("page script should evaluate");

    assert_eq!(navigations.lock().unwrap().as_slice(), [page.url]);
}

#[test]
fn window_location_shares_the_window_with_viewport_and_device_scale() {
    let navigations = Arc::new(Mutex::new(Vec::new()));
    let captured_navigations = navigations.clone();
    let page = browser::Page {
        url: "https://example.com/start".into(),
        html: String::new(),
        stylesheets: Vec::new(),
        scripts: vec![
            "if (window.devicePixelRatio === 2 && window.innerWidth === 800 \
             && window.innerHeight === 600 \
             && window.location.href === 'https://example.com/start') location.assign('/next')"
                .into(),
        ],
        font_faces: Vec::new(),
    };
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );

    BrowserRuntime::in_memory()
        .open_page_with_viewport_and_navigation(
            &page,
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
            2.0,
            Some(webapis::ViewportSize {
                width: 800,
                height: 600,
            }),
            Arc::new(move |url, _behavior| {
                captured_navigations.lock().unwrap().push(url.to_string());
            }),
        )
        .expect("page script should evaluate");

    assert_eq!(
        navigations.lock().unwrap().as_slice(),
        ["https://example.com/next"]
    );
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
        font_faces: Vec::new(),
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
        .and_then(|node| node.get_parent())
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
