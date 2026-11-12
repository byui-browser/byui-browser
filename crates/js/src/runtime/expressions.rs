//! Semantics of the expression forms beyond plain operators, shared by the
//! tree-walk evaluator and the bytecode VM: `in`, `instanceof`, `delete`,
//! spread, and the nullish test behind `??` and `?.`.

use super::array::array_len;
use super::non_object;
use crate::{JsError, JsErrorCategory, JsResult, Object, Value};

/// Whether `value` is `null` or `undefined`, the values `??`, `?.`, and `??=`
/// treat as missing.
pub(crate) fn is_nullish(value: &Value) -> bool {
    matches!(value, Value::Undefined | Value::Null)
}

/// `key in target`: whether `target` has the property, own or inherited.
///
/// Objects and arrays follow their prototype chain; a function reports only
/// its own properties. Any other right operand is a runtime error. Array
/// holes created by elisions such as `[1, , 3]` count as present, because the
/// engine stores them as `undefined` elements.
pub(crate) fn has_property(target: &Value, key: &str) -> JsResult<bool> {
    match target {
        Value::Object(object) => Ok(object.get_property(key)?.is_some()),
        Value::Array(array) => {
            Ok(array.get_property(key)?.is_some() || matches!(key, "push" | "pop"))
        }
        Value::Function(function) => Ok(function.get_own(key).is_some()),
        other => Err(JsError::with_context(
            JsErrorCategory::Runtime,
            format!("Cannot use 'in' operator to search for `{key}` in `{other}`"),
            "in operator",
        )),
    }
}

/// `value instanceof constructor`: whether the constructor's `prototype`
/// object is on the prototype chain of `value`.
///
/// The right operand must be a function with an object-valued `prototype`,
/// which only class constructors and bytecode functions have; anything else
/// is a runtime error. A left operand that is not an object or array is
/// never an instance.
pub(crate) fn instance_of(value: &Value, constructor: &Value) -> JsResult<bool> {
    let Value::Function(constructor) = constructor else {
        return Err(JsError::with_context(
            JsErrorCategory::Runtime,
            "Right-hand side of 'instanceof' is not callable",
            "instanceof operator",
        ));
    };
    let Some(Value::Object(prototype) | Value::Array(prototype)) = constructor.get_own("prototype")
    else {
        return Err(JsError::with_context(
            JsErrorCategory::Runtime,
            "Function has a non-object prototype in 'instanceof' check",
            "instanceof operator",
        ));
    };
    let (Value::Object(object) | Value::Array(object)) = value else {
        return Ok(false);
    };
    let mut current = object.prototype();
    while let Some(candidate) = current {
        if candidate == prototype {
            return Ok(true);
        }
        current = candidate.prototype();
    }
    Ok(false)
}

/// `delete target[key]`, returning the boolean the expression evaluates to.
///
/// Own properties of objects and arrays are removed; deleting an array
/// element leaves a hole that reads as `undefined`. Properties that cannot be
/// deleted (`length`, read-only and host accessor properties) report `false`
/// and stay. Primitives other than `null` and `undefined` report `true`
/// without effect, except a string's `length` and indices. `null` and
/// `undefined` are runtime errors, and so is a function, because functions do
/// not support property removal.
pub(crate) fn delete_property(target: &Value, key: &str) -> JsResult<bool> {
    match target {
        Value::Object(object) => Ok(object.delete_own(key)),
        Value::Array(array) => Ok(key != "length" && array.delete_own(key)),
        Value::String(text) => {
            let is_index = key
                .parse::<usize>()
                .is_ok_and(|index| index < text.chars().count());
            Ok(key != "length" && !is_index)
        }
        Value::Boolean(_) | Value::Number(_) => Ok(true),
        Value::Function(_) => Err(JsError::with_context(
            JsErrorCategory::Runtime,
            format!("Cannot delete property `{key}` of a function"),
            "delete expression",
        )),
        Value::Undefined | Value::Null => {
            Err(non_object(target, "delete", key, "delete expression"))
        }
    }
}

/// The values `...value` contributes in an array literal or argument list.
///
/// Arrays contribute their elements in order, with holes as `undefined`, and
/// strings their characters. There is no iterator protocol, so every other
/// value is a runtime error.
pub(crate) fn spread_values(value: &Value) -> JsResult<Vec<Value>> {
    match value {
        Value::Array(array) => Ok((0..array_len(array))
            .map(|index| {
                array
                    .get_own(&index.to_string())
                    .unwrap_or(Value::Undefined)
            })
            .collect()),
        Value::String(text) => Ok(text
            .chars()
            .map(|character| Value::String(character.to_string()))
            .collect()),
        other => Err(JsError::with_context(
            JsErrorCategory::Runtime,
            format!("`{other}` is not iterable"),
            "spread",
        )),
    }
}

/// `{ ...source }`: copies the own enumerable properties of `source` onto
/// `target`, in order.
///
/// `null` and `undefined` contribute nothing, an array contributes its
/// elements under their indices (not `length`), a string its characters, and
/// functions and other primitives nothing. A failing host getter on an
/// object source is returned as the error.
pub(crate) fn copy_data_properties(target: &Object, source: &Value) -> JsResult<()> {
    match source {
        Value::Object(object) => {
            for key in object.keys() {
                if let Some(value) = object.get_property(&key)? {
                    target.set(key, value);
                }
            }
        }
        Value::Array(array) => {
            for key in array.keys() {
                if key != "length"
                    && let Some(value) = array.get_own(&key)
                {
                    target.set(key, value);
                }
            }
        }
        Value::String(text) => {
            for (index, character) in text.chars().enumerate() {
                target.set(index.to_string(), Value::String(character.to_string()));
            }
        }
        Value::Undefined
        | Value::Null
        | Value::Boolean(_)
        | Value::Number(_)
        | Value::Function(_) => {}
    }
    Ok(())
}
