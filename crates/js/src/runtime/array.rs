//! Array behavior shared by the tree-walk evaluator and the bytecode VM.
//!
//! An array value is an [`Object`] that stores each element under its decimal
//! index (`"0"`, `"1"`, ...) and the element count under `length`. Reads need
//! no special handling, because a missing index reads as `undefined`. Writes
//! go through [`set_array_property`] so that `length` stays in step with the
//! elements.

use std::sync::Arc;

use super::conversions::to_number;
use super::{Function, Object};
use crate::{HostFunction, JsError, JsErrorCategory, JsResult, Value};

/// The largest length an array can have.
///
/// Elements are stored individually, so the limit keeps a script such as
/// `a.length = 1e9` from making later joins and loops enormous. Exceeding it
/// is a runtime error rather than a sparse array.
pub const MAX_ARRAY_LENGTH: usize = 1 << 20;

/// The first number that is not an array index; `2**32 - 1` and above are
/// ordinary property names, as in JavaScript.
const INDEX_LIMIT: f64 = 4_294_967_295.0;

/// Returns the element position `key` names, if it is a canonical array
/// index: decimal digits without a leading zero, below `2**32 - 1`.
fn array_index(key: &str) -> Option<usize> {
    let digits = key.as_bytes();
    if digits.is_empty()
        || digits.len() > 10
        || !digits.iter().all(u8::is_ascii_digit)
        || (digits.len() > 1 && digits[0] == b'0')
    {
        return None;
    }
    let index: f64 = key.parse().ok()?;
    (index < INDEX_LIMIT).then_some(index as usize)
}

/// Returns the array's `length`, or `0` if the property is missing.
pub(crate) fn array_len(array: &Object) -> usize {
    match array.get_own("length") {
        Some(Value::Number(length)) => length as usize,
        _ => 0,
    }
}

fn set_len(array: &Object, length: usize) {
    array.set("length", Value::Number(length as f64));
}

fn length_error() -> JsError {
    JsError::with_context(
        JsErrorCategory::Runtime,
        format!("Invalid array length: arrays are limited to {MAX_ARRAY_LENGTH} elements"),
        "array length",
    )
}

/// Writes `array[key] = value` the way a script does.
///
/// - A canonical index (`"3"`) stores the element and, when it is at or past
///   the end, grows `length` to one more than the index. Any gap before it
///   stays unset: a hole that reads as `undefined` and that
///   [`Object::contains_key`] reports as absent.
/// - `length` converts the value with `ToNumber`; it must be a whole number
///   from `0` to [`MAX_ARRAY_LENGTH`]. A smaller length deletes the elements
///   past it, and a larger one only adds holes.
/// - Any other key, such as `"01"`, `"1.5"`, `"-1"` or `"label"`, is an
///   ordinary named property and does not affect `length`.
///
/// Fails, changing nothing, for an index at or past [`MAX_ARRAY_LENGTH`] or an
/// invalid length.
pub(crate) fn set_array_property(array: &Object, key: &str, value: Value) -> JsResult<()> {
    if key == "length" {
        let number = to_number(&value);
        if number.fract() != 0.0 || !(0.0..=MAX_ARRAY_LENGTH as f64).contains(&number) {
            return Err(length_error());
        }
        let length = number as usize;
        if length < array_len(array) {
            for name in array.keys() {
                if array_index(&name).is_some_and(|index| index >= length) {
                    array.remove(&name);
                }
            }
        }
        set_len(array, length);
        return Ok(());
    }
    if let Some(index) = array_index(key) {
        if index >= MAX_ARRAY_LENGTH {
            return Err(length_error());
        }
        array.set(key, value);
        if index >= array_len(array) {
            set_len(array, index + 1);
        }
        return Ok(());
    }
    array.set(key, value);
    Ok(())
}

/// Reads `array[key]`: the stored property, or for `push` and `pop` a
/// function bound to this array, or `undefined`.
///
/// A bound method is created on every read, so `a.push === a.push` is false,
/// and an own property named `push` or `pop` takes precedence.
pub(crate) fn get_array_property(array: &Object, key: &str) -> Value {
    if let Some(value) = array.get(key) {
        return value;
    }
    match key {
        "push" => bound_method(array, "push", |array, arguments| {
            for argument in arguments {
                let length = array_len(array);
                set_array_property(array, &length.to_string(), argument.clone())?;
            }
            Ok(Value::Number(array_len(array) as f64))
        }),
        "pop" => bound_method(array, "pop", |array, _| {
            let length = array_len(array);
            if length == 0 {
                return Ok(Value::Undefined);
            }
            let last = (length - 1).to_string();
            let value = array.get_own(&last).unwrap_or(Value::Undefined);
            array.remove(&last);
            set_len(array, length - 1);
            Ok(value)
        }),
        _ => Value::Undefined,
    }
}

fn bound_method(
    array: &Object,
    name: &str,
    body: impl Fn(&Object, &[Value]) -> JsResult<Value> + Send + Sync + 'static,
) -> Value {
    let array = array.clone();
    let function: HostFunction = Arc::new(move |arguments| body(&array, arguments));
    Value::Function(Function::from_host(name, function))
}

/// The JavaScript string form of an array: its elements joined by commas,
/// with holes, `undefined`, and `null` as empty text. An array met again
/// while printing itself prints as empty text.
pub(crate) fn array_to_string(array: &Object) -> String {
    join(array, &mut Vec::new())
}

fn join(array: &Object, visiting: &mut Vec<Object>) -> String {
    if visiting.contains(array) {
        return String::new();
    }
    visiting.push(array.clone());
    let parts: Vec<String> = (0..array_len(array))
        .map(|index| match array.get_own(&index.to_string()) {
            None | Some(Value::Undefined | Value::Null) => String::new(),
            Some(Value::Array(inner)) => join(&inner, visiting),
            Some(other) => other.to_string(),
        })
        .collect();
    visiting.pop();
    parts.join(",")
}
