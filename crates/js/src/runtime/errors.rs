//! Native error constructors (`Error`, `RangeError`, `ReferenceError`,
//! `URIError`) and the URI decoding global `decodeURIComponent`.
//!
//! Every native error prototype inherits from the shared `Error.prototype`,
//! so `new RangeError() instanceof Error` holds. Instances own a string
//! `message` (when one was given) and an optional `cause`; `name` and the
//! `toString` method come from the prototype chain.
//!
//! Limitations: instances have no `stack` property, and errors the engine
//! raises itself (for example reading an undeclared variable) are still
//! caught as plain `Error`-named objects rather than `ReferenceError`
//! instances.

use std::sync::{Arc, OnceLock};

use super::{Function, Object, host_this};
use crate::{JsError, Value};

/// The native error kinds this module provides, in installation order.
const KINDS: &[&str] = &["Error", "RangeError", "ReferenceError", "URIError"];

/// Installs the native error constructors and `decodeURIComponent`.
pub(super) fn install(state: &super::GlobalState) {
    for kind in KINDS {
        state.install_host(kind, Value::Function(error_constructor(kind)));
    }
    state.install_host(
        "decodeURIComponent",
        Value::Function(decode_uri_component_function()),
    );
}

/// Returns a fresh standalone value for one of this module's globals, for the
/// VM's short-lived global environment.
pub(super) fn value(name: &str) -> Option<Value> {
    if KINDS.contains(&name) {
        return Some(Value::Function(error_constructor(name)));
    }
    (name == "decodeURIComponent").then(|| Value::Function(decode_uri_component_function()))
}

/// The shared `Error.prototype`, holding the default `name`, an empty
/// `message`, and `toString`.
fn error_prototype() -> Object {
    static PROTOTYPE: OnceLock<Object> = OnceLock::new();
    PROTOTYPE
        .get_or_init(|| {
            let prototype = Object::new();
            prototype.set("name", Value::String("Error".into()));
            prototype.set("message", Value::String(String::new()));
            prototype.set_method(
                "toString",
                Arc::new(|_| Ok(Value::String(error_to_string(&host_this())))),
            );
            prototype
        })
        .clone()
}

/// The intrinsic prototype for `kind`. Subclass prototypes are created once
/// and inherit from [`error_prototype`].
fn kind_prototype(kind: &str) -> Object {
    static RANGE: OnceLock<Object> = OnceLock::new();
    static REFERENCE: OnceLock<Object> = OnceLock::new();
    static URI: OnceLock<Object> = OnceLock::new();
    let slot = match kind {
        "RangeError" => &RANGE,
        "ReferenceError" => &REFERENCE,
        "URIError" => &URI,
        _ => return error_prototype(),
    };
    slot.get_or_init(|| {
        let prototype = Object::with_prototype(Some(error_prototype()));
        prototype.set("name", Value::String(kind.into()));
        prototype.set("message", Value::String(String::new()));
        prototype
    })
    .clone()
}

/// The intrinsic constructor for `kind`. It is created once and shared, like
/// its prototype, so `RangeError.prototype.constructor === RangeError` holds
/// even in the VM, which looks standard globals up afresh on every use.
fn error_constructor(kind: &str) -> Function {
    static ERROR: OnceLock<Function> = OnceLock::new();
    static RANGE: OnceLock<Function> = OnceLock::new();
    static REFERENCE: OnceLock<Function> = OnceLock::new();
    static URI: OnceLock<Function> = OnceLock::new();
    let slot = match kind {
        "RangeError" => &RANGE,
        "ReferenceError" => &REFERENCE,
        "URIError" => &URI,
        _ => &ERROR,
    };
    slot.get_or_init(|| build_error_constructor(kind)).clone()
}

/// Builds the constructor for `kind`. Calling it with or without `new`
/// creates the same new instance, as in JavaScript.
fn build_error_constructor(kind: &str) -> Function {
    let prototype = kind_prototype(kind);
    let call_prototype = prototype.clone();
    let construct_prototype = prototype.clone();
    let function = Function::from_host_constructor(
        kind,
        Arc::new(move |args| Ok(Value::Object(create_error(&call_prototype, args)))),
        Arc::new(move |args| Ok(Value::Object(create_error(&construct_prototype, args)))),
    );
    function.set_own("prototype", Value::Object(prototype.clone()));
    prototype.set_read_only("constructor", Value::Function(function.clone()));
    function
}

/// Creates an instance whose prototype is `prototype`, following the
/// `Error(message, options)` constructor steps: a defined `message` is
/// converted to a string, and an `options` object's `cause` is copied.
fn create_error(prototype: &Object, args: &[Value]) -> Object {
    let error = Object::with_prototype(Some(prototype.clone()));
    if let Some(message) = args.first()
        && *message != Value::Undefined
    {
        error.set("message", Value::String(message.to_string()));
    }
    if let Some(Value::Object(options)) = args.get(1)
        && options.contains_key("cause")
    {
        error.set("cause", options.get("cause").unwrap_or(Value::Undefined));
    }
    error
}

/// Creates a native error object of `kind` carrying `message`, for host code
/// that needs to throw one, such as `decodeURIComponent` throwing `URIError`.
pub(crate) fn new_error(kind: &str, message: &str) -> Value {
    Value::Object(create_error(
        &kind_prototype(kind),
        &[Value::String(message.into())],
    ))
}

/// `Error.prototype.toString`: `name: message`, omitting whichever part is
/// empty.
fn error_to_string(this: &Value) -> String {
    let (Value::Object(object) | Value::Array(object)) = this else {
        return "Error".into();
    };
    let read = |key: &str, default: &str| match object.get_property(key) {
        Ok(Some(Value::Undefined)) | Ok(None) | Err(_) => default.to_owned(),
        Ok(Some(value)) => value.to_string(),
    };
    let name = read("name", "Error");
    let message = read("message", "");
    match (name.is_empty(), message.is_empty()) {
        (true, _) => message,
        (_, true) => name,
        _ => format!("{name}: {message}"),
    }
}

fn decode_uri_component_function() -> Function {
    Function::from_host(
        "decodeURIComponent",
        Arc::new(|args| {
            let input = match args.first() {
                Some(value) => value.to_string(),
                None => "undefined".to_owned(),
            };
            decode_uri_component(&input)
                .map(Value::String)
                .map_err(|message| JsError::thrown(new_error("URIError", message)))
        }),
    )
}

/// Decodes every `%XX` escape in `input`, as `decodeURIComponent` does.
///
/// Escapes are hexadecimal UTF-8 bytes; a multi-byte sequence must be fully
/// escaped and form valid UTF-8 (no overlong forms or surrogates). Any
/// malformed escape yields the `URIError` message.
pub(crate) fn decode_uri_component(input: &str) -> Result<String, &'static str> {
    const MALFORMED: &str = "URI malformed";
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    let escaped_byte = |at: usize| -> Option<u8> {
        if bytes.get(at) != Some(&b'%') {
            return None;
        }
        let digits = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        u8::from_str_radix(digits, 16).ok()
    };
    while index < bytes.len() {
        if bytes[index] != b'%' {
            // Copy the run of unescaped text up to the next `%` unchanged;
            // `%` is ASCII, so the run ends on a character boundary.
            let end = input[index..]
                .find('%')
                .map_or(bytes.len(), |offset| index + offset);
            output.push_str(&input[index..end]);
            index = end;
            continue;
        }
        let lead = escaped_byte(index).ok_or(MALFORMED)?;
        index += 3;
        let length = match lead.leading_ones() {
            0 => 1,
            count @ 2..=4 => count as usize,
            _ => return Err(MALFORMED),
        };
        let mut sequence = vec![lead];
        for _ in 1..length {
            let continuation = escaped_byte(index).ok_or(MALFORMED)?;
            if continuation & 0xC0 != 0x80 {
                return Err(MALFORMED);
            }
            sequence.push(continuation);
            index += 3;
        }
        output.push_str(std::str::from_utf8(&sequence).map_err(|_| MALFORMED)?);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::decode_uri_component;

    #[test]
    fn decodes_ascii_and_multi_byte_escapes() {
        assert_eq!(decode_uri_component("a%20b%2Fc").as_deref(), Ok("a b/c"));
        assert_eq!(decode_uri_component("%E2%82%AC").as_deref(), Ok("€"));
        assert_eq!(decode_uri_component("%F0%9F%98%80").as_deref(), Ok("😀"));
        assert_eq!(decode_uri_component("caf%c3%a9").as_deref(), Ok("café"));
        assert_eq!(
            decode_uri_component("ünchanged").as_deref(),
            Ok("ünchanged")
        );
    }

    #[test]
    fn rejects_malformed_escapes() {
        for input in [
            "%",
            "%2",
            "%zz",
            "%C3",
            "%C3%28",
            "%80",
            "%C0%AF",
            "%ED%A0%80",
            "%F8%80",
        ] {
            assert!(decode_uri_component(input).is_err(), "{input}");
        }
    }
}
