//! ECMAScript type conversions and comparisons for primitive values.

use crate::Value;
use crate::lexer::is_js_whitespace;

/// `ToPrimitive`: functions convert to their string form; primitives are kept.
pub(super) fn to_primitive(value: &Value) -> Value {
    match value {
        Value::Function(_) => Value::String(value.to_string()),
        other => other.clone(),
    }
}

/// `ToBoolean`.
pub(super) fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(value) => *value,
        Value::Number(value) => *value != 0.0 && !value.is_nan(),
        Value::String(value) => !value.is_empty(),
        Value::Function(_) => true,
    }
}

/// `ToNumber`. Never fails: unconvertible values become `NaN`.
pub(super) fn to_number(value: &Value) -> f64 {
    match value {
        Value::Number(number) => *number,
        Value::Boolean(true) => 1.0,
        Value::Boolean(false) | Value::Null => 0.0,
        Value::Undefined | Value::Function(_) => f64::NAN,
        Value::String(text) => string_to_number(text),
    }
}

/// `StringToNumber`: surrounding whitespace is ignored, empty text is `0`,
/// `0x`/`0o`/`0b` prefixes and `Infinity` are recognized, and anything else
/// that is not a decimal literal is `NaN`.
pub(super) fn string_to_number(text: &str) -> f64 {
    let text = text.trim_matches(is_js_whitespace);
    if text.is_empty() {
        return 0.0;
    }

    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        let lower_prefix = text.get(..2).map(str::to_ascii_lowercase);
        if lower_prefix.as_deref() == Some(prefix) {
            let digits = &text[2..];
            if digits.is_empty() {
                return f64::NAN;
            }
            return digits
                .chars()
                .try_fold(0.0, |total, ch| {
                    ch.to_digit(radix)
                        .map(|digit| total * f64::from(radix) + f64::from(digit))
                })
                .unwrap_or(f64::NAN);
        }
    }

    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if unsigned == "Infinity" {
        return if text.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if is_decimal_literal(unsigned) {
        text.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// Checks the unsigned `StrDecimalLiteral` grammar, rejecting spellings such
/// as `inf` or `nan` that Rust's float parser would otherwise accept.
fn is_decimal_literal(text: &str) -> bool {
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(index) => (&text[..index], Some(&text[index + 1..])),
        None => (text, None),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let mantissa_ok =
        all_digits(integer) && all_digits(fraction) && !(integer.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|exponent| {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !digits.is_empty() && all_digits(digits)
    });
    mantissa_ok && exponent_ok
}

/// `Number::toString` with radix 10, using the shortest round-trip digits.
pub(crate) fn number_to_string(number: f64) -> String {
    if number.is_nan() {
        return "NaN".into();
    }
    if number == 0.0 {
        return "0".into();
    }
    if number.is_infinite() {
        return if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .into();
    }
    if number < 0.0 {
        return format!("-{}", number_to_string(-number));
    }

    // `{:e}` yields the shortest round-trip digits as `d.ddde±x`.
    let scientific = format!("{number:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("scientific notation has an exponent");
    let digits = mantissa.replace('.', "");
    let exponent: i32 = exponent.parse().expect("exponent is an integer");
    let digit_count = i32::try_from(digits.len()).expect("f64 has few digits");
    // Position of the decimal point relative to the digit string.
    let point = exponent + 1;

    if digit_count <= point && point <= 21 {
        let zeros = "0".repeat((point - digit_count) as usize);
        format!("{digits}{zeros}")
    } else if 0 < point && point <= 21 {
        let (whole, fraction) = digits.split_at(point as usize);
        format!("{whole}.{fraction}")
    } else if -6 < point && point <= 0 {
        format!("0.{}{digits}", "0".repeat(point.unsigned_abs() as usize))
    } else {
        let sign = if point > 0 { '+' } else { '-' };
        let magnitude = (point - 1).unsigned_abs();
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{first}e{sign}{magnitude}")
        } else {
            format!("{first}.{rest}e{sign}{magnitude}")
        }
    }
}

/// `ToUint32`: wraps the truncated number modulo 2^32.
pub(super) fn to_uint32(value: &Value) -> u32 {
    let number = to_number(value);
    if !number.is_finite() {
        return 0;
    }
    number.trunc().rem_euclid(4_294_967_296.0) as u32
}

/// `ToInt32`: `ToUint32` reinterpreted as a signed integer.
pub(super) fn to_int32(value: &Value) -> i32 {
    to_uint32(value) as i32
}

/// `IsStrictlyEqual` (`===`).
pub(super) fn strict_equals(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Boolean(left), Value::Boolean(right)) => left == right,
        (Value::Number(left), Value::Number(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Function(left), Value::Function(right)) => left == right,
        _ => false,
    }
}

/// `IsLooselyEqual` (`==`).
pub(super) fn loose_equals(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
        (Value::Number(number), Value::String(text))
        | (Value::String(text), Value::Number(number)) => *number == string_to_number(text),
        (Value::Boolean(_), _) => loose_equals(&Value::Number(to_number(left)), right),
        (_, Value::Boolean(_)) => loose_equals(left, &Value::Number(to_number(right))),
        (Value::Function(_), Value::Number(_) | Value::String(_)) => {
            loose_equals(&to_primitive(left), right)
        }
        (Value::Number(_) | Value::String(_), Value::Function(_)) => {
            loose_equals(left, &to_primitive(right))
        }
        _ => strict_equals(left, right),
    }
}

/// `IsLessThan`: `None` when either side converts to `NaN`. Strings compare
/// by UTF-16 code units, matching JavaScript ordering.
pub(super) fn less_than(left: &Value, right: &Value) -> Option<bool> {
    let (left, right) = (to_primitive(left), to_primitive(right));
    if let (Value::String(left), Value::String(right)) = (&left, &right) {
        return Some(left.encode_utf16().lt(right.encode_utf16()));
    }
    let (left, right) = (to_number(&left), to_number(&right));
    if left.is_nan() || right.is_nan() {
        None
    } else {
        Some(left < right)
    }
}
