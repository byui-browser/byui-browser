use std::sync::{Arc, Mutex};

use browser::evaluate_page_scripts;
use webapis::ConsoleSink;

#[derive(Default)]
struct TestConsole {
    messages: Mutex<Vec<String>>,
}

impl ConsoleSink for TestConsole {
    fn log(&self, message: &str) {
        self.messages.lock().unwrap().push(message.to_owned());
    }
}

#[test]
fn html_script_reaches_registered_web_api() {
    let console = Arc::new(TestConsole::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );

    let results = evaluate_page_scripts(
        "<main><script>print()</script></main>",
        console.clone(),
        controller,
    )
    .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::Undefined]);
    assert_eq!(
        console.messages.lock().unwrap().as_slice(),
        ["JS API Team Rocks!"]
    );
}

#[test]
fn html_script_reads_dom_attributes_through_document() {
    let console = Arc::new(TestConsole::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );

    let results = evaluate_page_scripts(
        "<main id='app' data-state='ready'><script>document.getElementById('app').getAttribute('DATA-STATE')</script></main>",
        console,
        controller,
    )
    .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::String("ready".to_owned())]);
}
