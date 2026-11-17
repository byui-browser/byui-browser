//! Unary and binary operator semantics shared by the evaluator and the VM.

use super::conversions::{
    is_truthy, less_than, loose_equals, strict_equals, to_int32, to_number, to_primitive,
    to_property_key, to_uint32,
};
use super::expressions::{has_property, instance_of};
use crate::ast::{BinaryOperator, UnaryOperator};
use crate::{JsResult, Value};

/// Applies a unary operator to an operand. Conversions never fail, so the
/// result is always a value.
///
/// `typeof` of an undeclared identifier is handled by the caller, which is
/// the only place that can tell a missing binding from a failed read.
pub(crate) fn unary(operator: UnaryOperator, value: &Value) -> Value {
    match operator {
        UnaryOperator::Negate => Value::Number(-to_number(value)),
        UnaryOperator::Plus => Value::Number(to_number(value)),
        UnaryOperator::Not => Value::Boolean(!is_truthy(value)),
        UnaryOperator::BitwiseNot => Value::Number(f64::from(!to_int32(value))),
        UnaryOperator::Typeof => Value::String(type_of(value).to_owned()),
        UnaryOperator::Void => Value::Undefined,
    }
}

/// The result of `typeof value`. `null` is `"object"`, as in JavaScript;
/// arrays are objects.
pub(crate) fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null | Value::Object(_) | Value::Array(_) => "object",
        Value::Boolean(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Function(_) => "function",
    }
}

/// Applies a binary operator to already-evaluated operands.
///
/// Only `in` and `instanceof` can fail, when the right operand is not a
/// suitable object or constructor; every other operator converts its
/// operands without errors.
pub(crate) fn binary(operator: BinaryOperator, left: &Value, right: &Value) -> JsResult<Value> {
    match operator {
        BinaryOperator::In => has_property(right, &to_property_key(left)).map(Value::Boolean),
        BinaryOperator::InstanceOf => instance_of(left, right).map(Value::Boolean),
        _ => Ok(eager_binary(operator, left, right)),
    }
}

/// `Number::exponentiate`. Differs from `f64::powf` where JavaScript returns
/// `NaN`: `1 ** Infinity`, `(-1) ** Infinity`, and any `NaN` exponent.
fn power(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() || (exponent.is_infinite() && base.abs() == 1.0) {
        f64::NAN
    } else {
        base.powf(exponent)
    }
}

/// The operators that cannot fail.
fn eager_binary(operator: BinaryOperator, left: &Value, right: &Value) -> Value {
    let number = |operation: fn(f64, f64) -> f64| {
        Value::Number(operation(to_number(left), to_number(right)))
    };
    let int32 = |operation: fn(i32, i32) -> i32| {
        Value::Number(f64::from(operation(to_int32(left), to_int32(right))))
    };
    let shift = to_uint32(right) & 31;
    match operator {
        BinaryOperator::Add => {
            let (left, right) = (to_primitive(left), to_primitive(right));
            if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
                Value::String(format!("{left}{right}"))
            } else {
                Value::Number(to_number(&left) + to_number(&right))
            }
        }
        BinaryOperator::Subtract => number(|left, right| left - right),
        BinaryOperator::Multiply => number(|left, right| left * right),
        BinaryOperator::Divide => number(|left, right| left / right),
        BinaryOperator::Remainder => number(|left, right| left % right),
        BinaryOperator::Exponent => number(power),
        BinaryOperator::BitwiseAnd => int32(|left, right| left & right),
        BinaryOperator::BitwiseOr => int32(|left, right| left | right),
        BinaryOperator::BitwiseXor => int32(|left, right| left ^ right),
        BinaryOperator::LeftShift => Value::Number(f64::from(to_int32(left).wrapping_shl(shift))),
        BinaryOperator::SignedRightShift => Value::Number(f64::from(to_int32(left) >> shift)),
        BinaryOperator::UnsignedRightShift => Value::Number(f64::from(to_uint32(left) >> shift)),
        BinaryOperator::Less => Value::Boolean(less_than(left, right) == Some(true)),
        BinaryOperator::Greater => Value::Boolean(less_than(right, left) == Some(true)),
        BinaryOperator::LessEqual => Value::Boolean(less_than(right, left) == Some(false)),
        BinaryOperator::GreaterEqual => Value::Boolean(less_than(left, right) == Some(false)),
        BinaryOperator::Equal => Value::Boolean(loose_equals(left, right)),
        BinaryOperator::NotEqual => Value::Boolean(!loose_equals(left, right)),
        BinaryOperator::StrictEqual => Value::Boolean(strict_equals(left, right)),
        BinaryOperator::StrictNotEqual => Value::Boolean(!strict_equals(left, right)),
        BinaryOperator::In | BinaryOperator::InstanceOf => {
            unreachable!("`binary` handles the fallible operators")
        }
    }
}
