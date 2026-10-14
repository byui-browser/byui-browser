//! End-to-end coverage for the `print` Web API: JavaScript source evaluated by
//! a [`Realm`] must reach `webapis::print` through the registered global.

use std::sync::{Arc, Mutex};

use js::{JsErrorCategory, Realm, Value};
use webapis::{ConsoleSink, register_print};

const MESSAGE: &str = "JS API Team Rocks!";

#[derive(Default)]
struct CapturingConsole {
    messages: Mutex<Vec<String>>,
}

impl ConsoleSink for CapturingConsole {
    fn log(&self, message: &str) {
        self.messages.lock().unwrap().push(message.to_owned());
    }
}

impl CapturingConsole {
    fn count(&self) -> usize {
        self.messages.lock().unwrap().len()
    }
}

fn realm_with_print() -> (Realm, Arc<CapturingConsole>) {
    let console = Arc::new(CapturingConsole::default());
    let mut realm = Realm::new();
    register_print(&mut realm, console.clone()).unwrap();
    (realm, console)
}

#[test]
fn javascript_print_calls_the_web_api_and_captures_console_output() {
    let (realm, console) = realm_with_print();

    let result = realm.evaluate_script("  print()  ");

    assert_eq!(result, Ok(Value::Undefined));
    assert_eq!(console.messages.lock().unwrap().as_slice(), [MESSAGE]);
}

#[test]
fn print_runs_once_per_javascript_call() {
    let (realm, console) = realm_with_print();

    let result = realm.evaluate_script(
        "function greet() { print(); return 'greeted'; }
         let i = 0;
         while (i < 3) { greet(); i = i + 1; }
         if (i == 3) { print(); }
         greet()",
    );

    assert_eq!(result, Ok(Value::String("greeted".into())));
    assert_eq!(console.messages.lock().unwrap().as_slice(), [MESSAGE; 5]);
}

#[test]
fn print_is_not_called_on_untaken_paths() {
    let (realm, console) = realm_with_print();

    realm
        .evaluate_script(
            "if (false) { print(); }
             false && print();
             true || print();
             function unused() { print(); }",
        )
        .unwrap();

    assert_eq!(console.count(), 0);
}

#[test]
fn print_returns_undefined_to_javascript() {
    let (realm, console) = realm_with_print();

    assert_eq!(
        realm.evaluate_script("print() === undefined"),
        Ok(Value::Boolean(true))
    );
    assert_eq!(console.count(), 1);
}

#[test]
fn print_rejects_arguments_and_stops_the_script() {
    let (realm, console) = realm_with_print();

    let error = realm
        .evaluate_script("print('hello'); print();")
        .unwrap_err();

    assert_eq!(error.message, "print() does not accept arguments");
    assert_eq!(error.category, JsErrorCategory::Runtime);
    assert_eq!(console.count(), 0);
}

#[test]
fn print_is_undefined_until_registered() {
    let error = Realm::new().evaluate_script("print()").unwrap_err();

    assert_eq!(error.message, "`print` is not defined");
    assert_eq!(error.category, JsErrorCategory::Runtime);
}

#[test]
fn scripts_can_shadow_print_without_reaching_the_web_api() {
    let (realm, console) = realm_with_print();

    assert_eq!(
        realm.evaluate_script("let print = 1; print"),
        Ok(Value::Number(1.0))
    );
    assert_eq!(console.count(), 0);
    realm.evaluate_script("print()").unwrap();
    assert_eq!(console.count(), 1);
}
