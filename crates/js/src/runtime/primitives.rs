//! Built-in `String` / `Number` constructors and their prototypes.
//!
//! Primitive strings and numbers have no own properties (aside from a string's
//! `length` and character indices). Property reads fall through to
//! [`string_prototype`] or [`number_prototype`], which hold the common methods.
//! Method calls receive the primitive as `this` through [`super::with_host_this`].
//! `Number.isNaN` and `Number.isFinite` inspect numeric primitives without
//! coercing other values, unlike the global `isNaN` and `isFinite` functions.

use std::sync::{Arc, OnceLock};

use super::conversions::{number_to_string, to_number};
use super::{Function, Object, host_this};
use crate::{HostFunction, JsError, JsResult, Value};

/// Installs the `String` and `Number` globals.
pub(super) fn install(state: &super::GlobalState) {
    state.install_host("String", Value::Function(string_constructor()));
    state.install_host("Number", Value::Function(number_constructor()));
    super::global_functions::install(state);
}

/// The shared `String.prototype` object.
pub(crate) fn string_prototype() -> Object {
    static PROTOTYPE: OnceLock<Object> = OnceLock::new();
    PROTOTYPE
        .get_or_init(|| {
            let prototype = Object::new();
            install_string_methods(&prototype);
            prototype
        })
        .clone()
}

/// The shared `Number.prototype` object.
pub(crate) fn number_prototype() -> Object {
    static PROTOTYPE: OnceLock<Object> = OnceLock::new();
    PROTOTYPE
        .get_or_init(|| {
            let prototype = Object::new();
            install_number_methods(&prototype);
            prototype
        })
        .clone()
}

/// Fresh `String` constructor sharing the intrinsic prototype. Used when the
/// VM installs short-lived standard globals.
pub(crate) fn string_constructor_value() -> Function {
    string_constructor()
}

/// Fresh `Number` constructor sharing the intrinsic prototype.
pub(crate) fn number_constructor_value() -> Function {
    number_constructor()
}

fn string_constructor() -> Function {
    let function = Function::from_host_constructor(
        "String",
        Arc::new(|args| Ok(Value::String(to_string_arg(args.first())))),
        Arc::new(|args| {
            // No distinct string-wrapper objects yet: `new String(x)` yields
            // the same primitive conversion as `String(x)`.
            Ok(Value::String(to_string_arg(args.first())))
        }),
    );
    function.set_own("prototype", Value::Object(string_prototype()));
    string_prototype().set_read_only("constructor", Value::Function(function.clone()));
    function
}

fn number_constructor() -> Function {
    let function = Function::from_host_constructor(
        "Number",
        Arc::new(|args| {
            Ok(Value::Number(match args.first() {
                Some(value) => to_number(value),
                None => 0.0,
            }))
        }),
        Arc::new(|args| {
            Ok(Value::Number(match args.first() {
                Some(value) => to_number(value),
                None => 0.0,
            }))
        }),
    );
    function.set_own("prototype", Value::Object(number_prototype()));
    number_prototype().set_read_only("constructor", Value::Function(function.clone()));
    // These predicates deliberately do not apply ToNumber.
    for (name, predicate) in [
        ("isNaN", f64::is_nan as fn(f64) -> bool),
        ("isFinite", f64::is_finite as fn(f64) -> bool),
        ("isSafeInteger", is_safe_integer as fn(f64) -> bool),
    ] {
        let method = Function::from_host(
            name,
            Arc::new(move |args| {
                Ok(Value::Boolean(
                    matches!(args.first(), Some(Value::Number(value)) if predicate(*value)),
                ))
            }),
        );
        method.set_own("length", Value::Number(1.0));
        function.set_own(name, Value::Function(method));
    }

    function
}

// The largest integer exactly and uniquely representable by an IEEE-754
// double is 2^53 - 1. This predicate never coerces non-number arguments.
fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0
}

/// Reads a property of a string primitive: `length`, a character index, or a
/// method / value from [`string_prototype`].
pub(crate) fn get_string_property(text: &str, key: &str) -> JsResult<Value> {
    if key == "length" {
        return Ok(Value::Number(text.chars().count() as f64));
    }
    if let Ok(index) = key.parse::<usize>() {
        return Ok(text
            .chars()
            .nth(index)
            .map(|character| Value::String(character.to_string()))
            .unwrap_or(Value::Undefined));
    }
    Ok(string_prototype()
        .get_property(key)?
        .unwrap_or(Value::Undefined))
}

/// Reads a property of a number primitive from [`number_prototype`].
pub(crate) fn get_number_property(key: &str) -> JsResult<Value> {
    Ok(number_prototype()
        .get_property(key)?
        .unwrap_or(Value::Undefined))
}

fn install_string_methods(prototype: &Object) {
    set_method(
        prototype,
        "charAt",
        Arc::new(|args| {
            let text = this_string()?;
            let index = to_integer(args.first());
            Ok(text
                .chars()
                .nth(index.max(0) as usize)
                .map(|character| Value::String(character.to_string()))
                .unwrap_or_else(|| Value::String(String::new())))
        }),
    );
    set_method(
        prototype,
        "charCodeAt",
        Arc::new(|args| {
            let text = this_string()?;
            let index = to_integer(args.first());
            Ok(match text.chars().nth(index.max(0) as usize) {
                Some(character) => Value::Number(f64::from(character as u32)),
                None => Value::Number(f64::NAN),
            })
        }),
    );
    set_method(
        prototype,
        "slice",
        Arc::new(|args| {
            let text = this_string()?;
            let chars: Vec<char> = text.chars().collect();
            let length = chars.len() as i64;
            let start = slice_index(args.first(), length);
            let end = args
                .get(1)
                .map(|value| slice_index(Some(value), length))
                .unwrap_or(length);
            if end <= start {
                return Ok(Value::String(String::new()));
            }
            Ok(Value::String(
                chars[start as usize..end as usize].iter().collect(),
            ))
        }),
    );
    set_method(
        prototype,
        "substring",
        Arc::new(|args| {
            let text = this_string()?;
            let chars: Vec<char> = text.chars().collect();
            let length = chars.len() as i64;
            let mut start = to_integer(args.first()).clamp(0, length);
            let mut end = args
                .get(1)
                .map(|value| to_integer(Some(value)).clamp(0, length))
                .unwrap_or(length);
            if start > end {
                std::mem::swap(&mut start, &mut end);
            }
            Ok(Value::String(
                chars[start as usize..end as usize].iter().collect(),
            ))
        }),
    );
    set_method(
        prototype,
        "indexOf",
        Arc::new(|args| {
            let text = this_string()?;
            let search = to_string_arg(args.first());
            let from = to_integer(args.get(1)).max(0) as usize;
            let chars: Vec<char> = text.chars().collect();
            let needle: Vec<char> = search.chars().collect();
            if needle.is_empty() {
                return Ok(Value::Number(from.min(chars.len()) as f64));
            }
            let start = from.min(chars.len());
            let found = chars[start..]
                .windows(needle.len())
                .position(|window| window == needle.as_slice())
                .map(|relative| (start + relative) as f64)
                .unwrap_or(-1.0);
            Ok(Value::Number(found))
        }),
    );
    set_method(
        prototype,
        "lastIndexOf",
        Arc::new(|args| {
            let text = this_string()?;
            let search = to_string_arg(args.first());
            let chars: Vec<char> = text.chars().collect();
            let needle: Vec<char> = search.chars().collect();
            let from = args
                .get(1)
                .map(|value| to_integer(Some(value)))
                .unwrap_or(chars.len() as i64)
                .clamp(0, chars.len() as i64) as usize;
            if needle.is_empty() {
                return Ok(Value::Number(from.min(chars.len()) as f64));
            }
            if needle.len() > chars.len() {
                return Ok(Value::Number(-1.0));
            }
            let last_start = from.min(chars.len().saturating_sub(needle.len()));
            let found = (0..=last_start)
                .rev()
                .find(|&index| chars[index..index + needle.len()] == needle[..])
                .map(|index| index as f64)
                .unwrap_or(-1.0);
            Ok(Value::Number(found))
        }),
    );
    set_method(
        prototype,
        "includes",
        Arc::new(|args| {
            let text = this_string()?;
            let search = to_string_arg(args.first());
            let from = to_integer(args.get(1)).max(0) as usize;
            let haystack: String = text.chars().skip(from).collect();
            Ok(Value::Boolean(haystack.contains(&search)))
        }),
    );
    set_method(
        prototype,
        "startsWith",
        Arc::new(|args| {
            let text = this_string()?;
            let search = to_string_arg(args.first());
            let from = to_integer(args.get(1)).max(0) as usize;
            let haystack: String = text.chars().skip(from).collect();
            Ok(Value::Boolean(haystack.starts_with(&search)))
        }),
    );
    set_method(
        prototype,
        "endsWith",
        Arc::new(|args| {
            let text = this_string()?;
            let search = to_string_arg(args.first());
            let end = args
                .get(1)
                .map(|value| to_integer(Some(value)).max(0) as usize)
                .unwrap_or_else(|| text.chars().count())
                .min(text.chars().count());
            let haystack: String = text.chars().take(end).collect();
            Ok(Value::Boolean(haystack.ends_with(&search)))
        }),
    );
    set_method(
        prototype,
        "toLowerCase",
        Arc::new(|_| Ok(Value::String(this_string()?.to_lowercase()))),
    );
    set_method(
        prototype,
        "toUpperCase",
        Arc::new(|_| Ok(Value::String(this_string()?.to_uppercase()))),
    );
    set_method(
        prototype,
        "trim",
        Arc::new(|_| Ok(Value::String(this_string()?.trim().to_owned()))),
    );
    set_method(
        prototype,
        "trimStart",
        Arc::new(|_| Ok(Value::String(this_string()?.trim_start().to_owned()))),
    );
    set_method(
        prototype,
        "trimEnd",
        Arc::new(|_| Ok(Value::String(this_string()?.trim_end().to_owned()))),
    );
    set_method(
        prototype,
        "concat",
        Arc::new(|args| {
            let mut text = this_string()?;
            for argument in args {
                text.push_str(&argument.to_string());
            }
            Ok(Value::String(text))
        }),
    );
    set_method(
        prototype,
        "repeat",
        Arc::new(|args| {
            let text = this_string()?;
            let count = to_integer(args.first());
            if count < 0 {
                return Err(JsError::new("Invalid count value"));
            }
            Ok(Value::String(text.repeat(count as usize)))
        }),
    );
    set_method(
        prototype,
        "split",
        Arc::new(|args| {
            let text = this_string()?;
            let separator = args.first().map(|value| value.to_string());
            let limit = args
                .get(1)
                .and_then(|value| match value {
                    Value::Number(number) if *number >= 0.0 => Some(*number as usize),
                    _ => None,
                })
                .unwrap_or(usize::MAX);
            let parts: Vec<String> = match separator.as_deref() {
                None => vec![text],
                Some("") => text
                    .chars()
                    .map(|character| character.to_string())
                    .collect(),
                Some(separator) => text.split(separator).map(str::to_owned).collect(),
            };
            let array = Object::new();
            let mut length = 0usize;
            for part in parts.into_iter().take(limit) {
                array.set(length.to_string(), Value::String(part));
                length += 1;
            }
            array.set("length", Value::Number(length as f64));
            Ok(Value::Array(array))
        }),
    );
    set_method(
        prototype,
        "replace",
        Arc::new(|args| {
            let text = this_string()?;
            let search = to_string_arg(args.first());
            let replacement = to_string_arg(args.get(1));
            Ok(Value::String(text.replacen(&search, &replacement, 1)))
        }),
    );
    set_method(
        prototype,
        "toString",
        Arc::new(|_| Ok(Value::String(this_string()?))),
    );
    set_method(
        prototype,
        "valueOf",
        Arc::new(|_| Ok(Value::String(this_string()?))),
    );
}

fn install_number_methods(prototype: &Object) {
    set_method(
        prototype,
        "toFixed",
        Arc::new(|args| {
            let number = this_number()?;
            let digits = to_integer(args.first()).clamp(0, 100) as usize;
            if !number.is_finite() {
                return Ok(Value::String(number_to_string(number)));
            }
            Ok(Value::String(format!("{number:.digits$}")))
        }),
    );
    set_method(
        prototype,
        "toString",
        Arc::new(|_| Ok(Value::String(number_to_string(this_number()?)))),
    );
    set_method(
        prototype,
        "valueOf",
        Arc::new(|_| Ok(Value::Number(this_number()?))),
    );
}

fn set_method(object: &Object, name: &str, function: HostFunction) {
    object.set_method(name, function);
}

fn this_string() -> JsResult<String> {
    Ok(match host_this() {
        Value::String(text) => text,
        Value::Number(number) => number_to_string(number),
        Value::Boolean(value) => value.to_string(),
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        other => other.to_string(),
    })
}

fn this_number() -> JsResult<f64> {
    Ok(to_number(&host_this()))
}

fn to_string_arg(value: Option<&Value>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "undefined".to_owned(),
    }
}

fn to_integer(value: Option<&Value>) -> i64 {
    match value {
        Some(value) => {
            let number = to_number(value);
            if !number.is_finite() {
                return 0;
            }
            number.trunc() as i64
        }
        None => 0,
    }
}

fn slice_index(value: Option<&Value>, length: i64) -> i64 {
    let index = to_integer(value);
    if index < 0 {
        (length + index).max(0)
    } else {
        index.min(length)
    }
}
