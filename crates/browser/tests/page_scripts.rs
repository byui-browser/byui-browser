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
