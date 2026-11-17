//! JavaScript console output and the `print` binding.

use std::sync::Arc;

use js::{HostFunction, JsError, JsResult, Object, Realm, Value};

/// Destination for messages emitted by browser Web APIs.
pub trait ConsoleSink: Send + Sync {
    /// Records a normal console message.
    fn log(&self, message: &str);

    /// Records a warning; sinks without levels may treat it as a log.
    fn warn(&self, message: &str) {
        self.log(message);
    }

    /// Records an error; sinks without levels may treat it as a log.
    fn error(&self, message: &str) {
        self.log(message);
    }

    /// Clears the displayed output when supported by the sink.
    fn clear(&self) {
        log::warn!("console sink does not support clearing output");
    }
}

/// Formats a value for the native console.
///
/// Objects and arrays use a compact JSON-like representation, with repeated
/// references marked as circular. Functions and unavailable properties are
/// represented as text. This is limited to types supported by the JS engine.
pub fn format_console_value(value: &Value) -> String {
    fn format(value: &Value, seen: &mut Vec<Object>, nested: bool, depth: usize) -> String {
        match value {
            Value::String(text) if nested => format!("{text:?}"),
            Value::Object(object) | Value::Array(object) => {
                if seen.contains(object) {
                    return "\"[Circular]\"".to_owned();
                }
                if depth >= 32 {
                    return "\"[Max depth]\"".to_owned();
                }
                seen.push(object.clone());
                if matches!(value, Value::Array(_)) {
                    let length = match object.get_property("length") {
                        Ok(Some(Value::Number(length))) if length.is_finite() && length >= 0.0 => {
                            length as usize
                        }
                        Ok(_) => 0,
                        Err(error) => return format!("[Unable to read array length: {error}]"),
                    };
                    let displayed = length.min(1000);
                    let mut items = Vec::with_capacity(displayed + usize::from(length > displayed));
                    for index in 0..displayed {
                        match object.get_property(&index.to_string()) {
                            Ok(Some(item)) => items.push(format(&item, seen, true, depth + 1)),
                            Ok(None) => items.push("null".to_owned()),
                            Err(error) => items.push(format!("[Unable to read property: {error}]")),
                        }
                        if length > displayed {
                            items.push(format!("... {} more items", length - displayed));
                        }
                    }
                    format!("[{}]", items.join(", "))
                } else {
                    let mut items = Vec::new();
                    for key in object.keys() {
                        match object.get_property(&key) {
                            Ok(Some(item)) => items
                                .push(format!("{key:?}: {}", format(&item, seen, true, depth + 1))),
                            Ok(None) => {}
                            Err(error) => {
                                items.push(format!("{key:?}: [Unable to read property: {error}]"))
                            }
                        }
                    }
                    format!("{{{}}}", items.join(", "))
                }
            }
            _ => value.to_string(),
        }
    }
    format(value, &mut Vec::new(), false, 0)
}

fn emit(
    sink: &dyn ConsoleSink,
    level: fn(&dyn ConsoleSink, &str),
    arguments: &[Value],
) -> JsResult<Value> {
    let message = arguments
        .iter()
        .map(format_console_value)
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

/// Installs the console methods used by page scripts.
///
/// Groups emit their labels without indentation; table, dir, and trace use the
/// same text formatting as log. Stack traces and asynchronous promise reporting
/// depend on engine support and are not provided here.
pub fn register_console(realm: &mut Realm, console: Arc<dyn ConsoleSink>) -> JsResult<()> {
    let object = Object::new();
    for (name, level) in [
        ("log", emit_log as fn(&dyn ConsoleSink, &str)),
        ("info", emit_log),
        ("debug", emit_log),
        ("dir", emit_log),
        ("table", emit_log),
        ("trace", emit_log),
        ("warn", emit_warn),
        ("error", emit_error),
    ] {
        let sink = Arc::clone(&console);
        object.set_method(
            name,
            Arc::new(move |arguments| emit(sink.as_ref(), level, arguments)),
        );
    }
    let sink = Arc::clone(&console);
    object.set_method(
        "assert",
        Arc::new(move |arguments| {
            let truthy = match arguments.first() {
                None | Some(Value::Undefined | Value::Null | Value::Boolean(false)) => false,
                Some(Value::Number(number)) => *number != 0.0 && !number.is_nan(),
                Some(Value::String(text)) => !text.is_empty(),
                Some(_) => true,
            };
            if !truthy {
                let detail = arguments
                    .get(1..)
                    .unwrap_or_default()
                    .iter()
                    .map(format_console_value)
                    .collect::<Vec<_>>()
                    .join(" ");
                let message = if detail.is_empty() {
                    "Assertion failed:".to_owned()
                } else {
                    format!("Assertion failed: {detail}")
                };
                sink.error(&message);
            }
            Ok(Value::Undefined)
        }),
    );
    let sink = Arc::clone(&console);
    object.set_method(
        "clear",
        Arc::new(move |_| {
            sink.clear();
            Ok(Value::Undefined)
        }),
    );
    for name in ["group", "groupCollapsed"] {
        let sink = Arc::clone(&console);
        object.set_method(
            name,
            Arc::new(move |arguments| {
                if !arguments.is_empty() {
                    emit(sink.as_ref(), emit_log, arguments)?;
                }
                Ok(Value::Undefined)
            }),
        );
    }
    object.set_method("groupEnd", Arc::new(|_| Ok(Value::Undefined)));
    if let Some(Value::Object(window)) = realm.registered_global_value("window") {
        window.set("console", Value::Object(object.clone()));
    }
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

    #[test]
    fn extended_methods_assertions_and_clear_are_routed_to_sink() {
        #[derive(Default)]
        struct Sink(Mutex<Vec<(String, String)>>);
        impl ConsoleSink for Sink {
            fn log(&self, message: &str) {
                self.0.lock().unwrap().push(("log".into(), message.into()));
            }
            fn warn(&self, message: &str) {
                self.0.lock().unwrap().push(("warn".into(), message.into()));
            }
            fn error(&self, message: &str) {
                self.0
                    .lock()
                    .unwrap()
                    .push(("error".into(), message.into()));
            }
            fn clear(&self) {
                self.0.lock().unwrap().clear();
            }
        }
        let sink = Arc::new(Sink::default());
        let mut realm = Realm::new();
        register_console(&mut realm, sink.clone()).unwrap();
        realm
            .evaluate_script(
                "console.log('old'); console.clear(); console.info('info'); \
             console.debug('debug'); console.dir('dir'); console.table('table'); \
             console.trace('trace'); console.warn('warn'); console.error('error'); \
             console.assert(true, 'ignored'); console.assert(0, 'bad', 7); \
             console.assert(''); console.group('group'); console.groupCollapsed('collapsed'); \
             console.group(); console.groupEnd();",
            )
            .unwrap();
        assert_eq!(
            *sink.0.lock().unwrap(),
            [
                ("log".into(), "info".into()),
                ("log".into(), "debug".into()),
                ("log".into(), "dir".into()),
                ("log".into(), "table".into()),
                ("log".into(), "trace".into()),
                ("warn".into(), "warn".into()),
                ("error".into(), "error".into()),
                ("error".into(), "Assertion failed: bad 7".into()),
                ("error".into(), "Assertion failed:".into()),
                ("log".into(), "group".into()),
                ("log".into(), "collapsed".into()),
            ]
        );
    }

    #[test]
    fn formatting_includes_objects_arrays_and_circular_references() {
        let mut realm = Realm::new();
        let sink = Arc::new(TestConsole::default());
        register_console(&mut realm, sink.clone()).unwrap();
        realm
            .evaluate_script(
                "let item = {name: 'hi', list: [1, 2]}; item.self = item; console.log(item);",
            )
            .unwrap();
        assert_eq!(
            sink.messages.lock().unwrap().as_slice(),
            ["{\"name\": \"hi\", \"list\": [1, 2], \"self\": \"[Circular]\"}"]
        );
    }

    #[test]
    fn console_is_shared_with_window_in_either_registration_order() {
        for console_first in [true, false] {
            let mut realm = Realm::new();
            let sink = Arc::new(TestConsole::default());
            if console_first {
                register_console(&mut realm, sink.clone()).unwrap();
                crate::register_window(&mut realm).unwrap();
            } else {
                crate::register_window(&mut realm).unwrap();
                register_console(&mut realm, sink.clone()).unwrap();
            }
            realm
                .evaluate_script("window.console.info('window')")
                .unwrap();
            assert_eq!(sink.messages.lock().unwrap().as_slice(), ["window"]);
        }
    }
}
