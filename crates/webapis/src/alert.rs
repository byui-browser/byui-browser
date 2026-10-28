//! JavaScript `alert` binding.

use std::sync::Arc;

use js::{HostFunction, JsResult, Realm, Value};

/// Destination for synchronous page alert requests.
///
/// The browser owns the concrete implementation. Keeping this boundary in
/// Web APIs lets the JavaScript engine and browser composition layer agree on
/// alert delivery without coupling the engine to a native window toolkit.
pub trait AlertSink: Send + Sync {
    /// Shows an alert containing `message` and returns after it is handled.
    fn alert(&self, message: &str);
}

/// Implements the JavaScript `alert(message)` Web API.
///
/// The optional first argument is converted using the JavaScript value's
/// standard display conversion. Additional arguments are ignored, matching
/// the browser API's single-message shape. The returned value is always
/// `undefined`.
pub fn alert(sink: &dyn AlertSink, arguments: &[Value]) -> JsResult<Value> {
    let message = arguments.first().map_or_else(String::new, Value::to_string);
    sink.alert(&message);
    Ok(Value::Undefined)
}

/// Installs the Web API global `alert` in a JavaScript realm.
pub fn register_alert(realm: &mut Realm, sink: Arc<dyn AlertSink>) -> JsResult<()> {
    let function: HostFunction = Arc::new(move |arguments| alert(sink.as_ref(), arguments));
    realm.register_global_function("alert", function)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use js::{Realm, Value};

    use super::*;

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
    fn alert_converts_the_first_argument_and_returns_undefined() {
        let sink = TestAlert::default();

        assert_eq!(
            alert(
                &sink,
                &[Value::Number(42.0), Value::String("ignored".into())]
            )
            .unwrap(),
            Value::Undefined
        );
        assert_eq!(sink.messages.lock().unwrap().as_slice(), ["42"]);
    }

    #[test]
    fn alert_without_an_argument_uses_an_empty_message() {
        let sink = TestAlert::default();

        alert(&sink, &[]).unwrap();

        assert_eq!(sink.messages.lock().unwrap().as_slice(), [""]);
    }

    #[test]
    fn registration_exposes_alert_to_the_realm() {
        let sink = Arc::new(TestAlert::default());
        let mut realm = Realm::new();
        register_alert(&mut realm, sink.clone()).unwrap();

        assert_eq!(
            realm.evaluate_script("alert('hello')"),
            Ok(Value::Undefined)
        );
        assert_eq!(sink.messages.lock().unwrap().as_slice(), ["hello"]);
    }
}
