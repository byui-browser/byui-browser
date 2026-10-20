use std::sync::{Arc, Mutex};

use browser::evaluate_page_scripts;
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

#[test]
fn html_script_reaches_registered_web_api() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );

    let results = evaluate_page_scripts(
        "<main><script>print()</script></main>",
        console.clone(),
        alert.clone(),
        controller,
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

    let results = evaluate_page_scripts(
        "<main><script>alert('team handshake')</script></main>",
        console,
        alert.clone(),
        controller,
    )
    .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::Undefined]);
    assert_eq!(
        alert.messages.lock().unwrap().as_slice(),
        ["team handshake"]
    );
}
