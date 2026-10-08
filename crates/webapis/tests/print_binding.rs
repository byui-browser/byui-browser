use std::sync::{Arc, Mutex};

use js::{Realm, Value};
use webapis::{ConsoleSink, register_print};

#[derive(Default)]
struct CapturingConsole {
    messages: Mutex<Vec<String>>,
}

impl ConsoleSink for CapturingConsole {
    fn log(&self, message: &str) {
        self.messages.lock().unwrap().push(message.to_owned());
    }
}

#[test]
fn javascript_print_calls_the_web_api_and_captures_console_output() {
    let console = Arc::new(CapturingConsole::default());
    let mut realm = Realm::new();
    register_print(&mut realm, console.clone()).unwrap();

    let result = realm.evaluate_script("  print()  ");

    assert_eq!(result, Ok(Value::Undefined));
    assert_eq!(
        console.messages.lock().unwrap().as_slice(),
        ["JS API Team Rocks!"]
    );
}
