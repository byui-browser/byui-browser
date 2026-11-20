//! Core constructors and namespaces exposed in every realm.

use std::sync::Arc;

use super::conversions::{number_to_string, to_number};
use super::{Function, Object};
use crate::{JsError, Value};

/// Installs core ECMAScript values supported by this runtime.
pub(super) fn install(state: &super::GlobalState) {
    for name in ["Object", "Boolean"] {
        if let Some(value) = value(name) {
            state.install_host(name, value);
        }
    }
    state.install_host("NaN", Value::Number(f64::NAN));
    state.install_host("Infinity", Value::Number(f64::INFINITY));
    state.install_host("Math", Value::Object(math_namespace()));
    state.install_host("JSON", Value::Object(json_namespace()));
}

/// Returns a fresh standard global for the standalone VM environment.
pub(crate) fn value(name: &str) -> Option<Value> {
    match name {
        "Object" => Some(Value::Function(object_constructor())),
        "Boolean" => Some(Value::Function(boolean_constructor())),
        "Math" => Some(Value::Object(math_namespace())),
        "JSON" => Some(Value::Object(json_namespace())),
        "NaN" => Some(Value::Number(f64::NAN)),
        "Infinity" => Some(Value::Number(f64::INFINITY)),
        _ => None,
    }
}

fn object_constructor() -> Function {
    Function::from_host_constructor(
        "Object",
        Arc::new(|args| Ok(object_arg(args.first()))),
        Arc::new(|args| Ok(object_arg(args.first()))),
    )
}

fn object_arg(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Object(_) | Value::Array(_) | Value::Function(_)) => value.unwrap().clone(),
        _ => Value::Object(Object::new()),
    }
}

fn boolean_constructor() -> Function {
    let convert = |args: &[Value]| Value::Boolean(args.first().is_some_and(truthy));
    Function::from_host_constructor(
        "Boolean",
        Arc::new(move |args| Ok(convert(args))),
        Arc::new(move |args| Ok(Value::Object(boolean_box(args.first())))),
    )
}

fn boolean_box(value: Option<&Value>) -> Object {
    let object = Object::new();
    object.set_read_only("[[BooleanData]]", Value::Boolean(value.is_some_and(truthy)));
    object
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(v) => *v,
        Value::Number(v) => *v != 0.0 && !v.is_nan(),
        Value::String(v) => !v.is_empty(),
        _ => true,
    }
}

fn math_namespace() -> Object {
    let math = Object::new();
    for (name, value) in [
        ("PI", std::f64::consts::PI),
        ("E", std::f64::consts::E),
        ("LN2", std::f64::consts::LN_2),
        ("LN10", std::f64::consts::LN_10),
        ("SQRT2", std::f64::consts::SQRT_2),
    ] {
        math.set_read_only(name, Value::Number(value));
    }
    for (name, operation) in [
        ("abs", f64::abs as fn(f64) -> f64),
        ("floor", f64::floor),
        ("ceil", f64::ceil),
        ("round", f64::round),
        ("trunc", f64::trunc),
        ("sqrt", f64::sqrt),
        ("sin", f64::sin),
        ("cos", f64::cos),
        ("tan", f64::tan),
        ("log", f64::ln),
        ("exp", f64::exp),
    ] {
        math.set_method(
            name,
            Arc::new(move |args| {
                Ok(Value::Number(operation(
                    args.first().map(to_number).unwrap_or(f64::NAN),
                )))
            }),
        );
    }
    math.set_method(
        "sign",
        Arc::new(|args| {
            let value = args.first().map(to_number).unwrap_or(f64::NAN);
            Ok(Value::Number(if value.is_nan() || value == 0.0 {
                value
            } else if value.is_sign_positive() {
                1.0
            } else {
                -1.0
            }))
        }),
    );
    for (name, operation) in [("max", f64::max as fn(f64, f64) -> f64), ("min", f64::min)] {
        math.set_method(
            name,
            Arc::new(move |args| {
                if args.is_empty() {
                    return Ok(Value::Number(if name == "max" {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    }));
                }
                let mut result = if name == "max" {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                };
                for arg in args {
                    let value = to_number(arg);
                    if value.is_nan() {
                        return Ok(Value::Number(f64::NAN));
                    }
                    result = operation(result, value);
                }
                Ok(Value::Number(result))
            }),
        );
    }
    math.set_method(
        "pow",
        Arc::new(|args| {
            Ok(Value::Number(
                to_number(args.first().unwrap_or(&Value::Undefined))
                    .powf(to_number(args.get(1).unwrap_or(&Value::Undefined))),
            ))
        }),
    );
    math
}

fn json_namespace() -> Object {
    let json = Object::new();
    json.set_method(
        "stringify",
        Arc::new(|args| {
            let Some(value) = args.first() else {
                return Ok(Value::Undefined);
            };
            match value {
                Value::Undefined | Value::Function(_) => Ok(Value::Undefined),
                Value::Null => Ok(Value::String("null".into())),
                Value::String(text) => Ok(Value::String(format!(
                    "\"{}\"",
                    text.replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', "\\n")
                        .replace('\r', "\\r")
                ))),
                Value::Number(number) if !number.is_finite() => Ok(Value::String("null".into())),
                Value::Number(number) => Ok(Value::String(number_to_string(*number))),
                Value::Boolean(value) => Ok(Value::String(value.to_string())),
                Value::Array(array) => {
                    let len = super::array::array_len(array);
                    let mut parts = Vec::with_capacity(len);
                    for i in 0..len {
                        let item = array.get_own(&i.to_string()).unwrap_or(Value::Null);
                        let Value::String(text) = json_quote_or_value(&item)? else {
                            return Ok(Value::Undefined);
                        };
                        parts.push(text);
                    }
                    Ok(Value::String(format!("[{}]", parts.join(","))))
                }
                Value::Object(_) => Err(JsError::runtime(
                    "JSON.stringify for ordinary objects is not supported yet",
                )),
            }
        }),
    );
    json.set_method(
        "parse",
        Arc::new(|_| Err(JsError::runtime("JSON.parse is not supported yet"))),
    );
    json
}

fn json_quote_or_value(value: &Value) -> crate::JsResult<Value> {
    if matches!(value, Value::Array(_) | Value::Object(_)) {
        return Err(JsError::runtime("nested JSON values are not supported yet"));
    }
    let namespace = json_namespace();
    match namespace.get("stringify") {
        Some(Value::Function(f)) => f.call(std::slice::from_ref(value)),
        _ => Ok(Value::Undefined),
    }
}
