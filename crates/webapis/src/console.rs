//! JavaScript console output and the `print` binding.

use std::sync::Arc;

use js::{HostFunction, JsError, JsResult, Object, Realm, Value};

/// Destination for messages emitted by browser Web APIs.
pub trait ConsoleSink: Send + Sync {
    fn log(&self, message: &str);

    fn warn(&self, message: &str) {
        self.log(message);
    }

    fn error(&self, message: &str) {
        self.log(message);
    }
}

fn emit(
    sink: &dyn ConsoleSink,
    level: fn(&dyn ConsoleSink, &str),
    arguments: &[Value],
) -> JsResult<Value> {
    let message = arguments
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    level(sink, &message);
    Ok(Value::Undefined)
}

fn emit_log(sink: &dyn ConsoleSink, message: &str) {
    sink.log(message);
}

fn emit_warn(sink: &dyn ConsoleSink, message: &str) {
    sink.warn(message);
}

fn emit_error(sink: &dyn ConsoleSink, message: &str) {
    sink.error(message);
}

/// JavaScript-facing `print` implementation.
///
/// The first slice intentionally has no arguments and emits a fixed message.
/// The output is injected so the renderer can route it to DevTools rather than
/// coupling Web APIs to process stdout.
pub fn print(console: &dyn ConsoleSink, arguments: &[Value]) -> JsResult<Value> {
    if !arguments.is_empty() {
        log::debug!(
            "print() called with {} arguments; rejected",
            arguments.len()
        );
        return Err(JsError::new("print() does not accept arguments"));
    }

    log::debug!("print bridged to console sink");
    console.log("JS API Team Rocks!");
    Ok(Value::Undefined)
}

/// Installs the Web API global `print` in a JavaScript realm.
pub fn register_print(realm: &mut Realm, console: Arc<dyn ConsoleSink>) -> JsResult<()> {
    let function: HostFunction = Arc::new(move |arguments| print(console.as_ref(), arguments));
    realm.register_global_function("print", function)
}

/// Installs the standard console methods used by page scripts.
pub fn register_console(realm: &mut Realm, console: Arc<dyn ConsoleSink>) -> JsResult<()> {
    let log_console = Arc::clone(&console);
    let warn_console = Arc::clone(&console);
    let error_console = console;
    let object = Object::new();
    object.set_method(
        "log",
        Arc::new(move |arguments| emit(log_console.as_ref(), emit_log, arguments)),
    );
    object.set_method(
        "warn",
        Arc::new(move |arguments| emit(warn_console.as_ref(), emit_warn, arguments)),
    );
    object.set_method(
        "error",
        Arc::new(move |arguments| emit(error_console.as_ref(), emit_error, arguments)),
    );
    realm.register_global_value("console", Value::Object(object))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use js::{Realm, Value};

    use super::*;

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
    fn print_sends_fixed_message_to_console() {
        let console = TestConsole::default();

        assert_eq!(print(&console, &[]).unwrap(), Value::Undefined);
        assert_eq!(
            console.messages.lock().unwrap().as_slice(),
            ["JS API Team Rocks!"]
        );
    }

    #[test]
    fn print_rejects_arguments() {
        let console = TestConsole::default();

        assert_eq!(
            print(&console, &[Value::String("hello".to_owned())])
                .unwrap_err()
                .to_string(),
            "print() does not accept arguments"
        );
    }

    #[test]
    fn registration_exposes_print_to_the_realm() {
        let console = Arc::new(TestConsole::default());
        let mut realm = Realm::new();
        register_print(&mut realm, console.clone()).unwrap();

        assert_eq!(realm.evaluate_script("print()"), Ok(Value::Undefined));
        assert_eq!(
            console.messages.lock().unwrap().as_slice(),
            ["JS API Team Rocks!"]
        );
    }

    #[test]
    fn registration_exposes_console_methods() {
        let console = Arc::new(TestConsole::default());
        let mut realm = Realm::new();
        register_console(&mut realm, console.clone()).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "console.log('hello', 7); console.warn('careful'); console.error('bad')"
            ),
            Ok(Value::Undefined)
        );
        assert_eq!(
            console.messages.lock().unwrap().as_slice(),
            ["hello 7", "careful", "bad"]
        );
    }
}
