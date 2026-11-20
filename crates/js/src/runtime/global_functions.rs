//! Numeric global functions: `parseInt`, `parseFloat`, `isNaN`, and
//! `isFinite`, URI helpers, and legacy escape helpers (ECMAScript §19.2).
//!
//! Arguments are converted the way the specification requires: the parsing
//! functions apply `ToString` (after `ToPrimitive`) and the predicates apply
//! `ToNumber`, so `isNaN("abc")` is `true` and `parseInt(" 42px")` is `42`.
//!
//! Limitation: `parseInt` with a radix other than 10 accumulates digits in
//! floating point, so results above 2^53 may differ in the last bits from
//! engines that round the full digit string exactly. Radix-10 results are
//! correctly rounded. Object arguments are converted with the engine's
//! shared `ToPrimitive`, which tries `valueOf` before `toString` even where
//! the specification's string hint would try `toString` first.

use std::sync::Arc;

use super::Function;
use super::conversions::{to_int32, to_number, to_primitive};
use crate::lexer::is_js_whitespace;
use crate::{JsError, Value};

/// The global function names this module provides.
const NAMES: &[&str] = &[
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "encodeURI",
    "decodeURI",
    "escape",
    "unescape",
];

/// Installs numeric globals and URI helpers.
pub(super) fn install(state: &super::GlobalState) {
    for name in NAMES {
        if let Some(function) = value(name) {
            state.install_host(name, function);
        }
    }
}

/// Returns a fresh standalone value for one of this module's globals, for the
/// VM's short-lived global environment.
pub(super) fn value(name: &str) -> Option<Value> {
    let function = match name {
        "parseInt" => Function::from_host(
            "parseInt",
            Arc::new(|args| {
                let radix = args.get(1).unwrap_or(&Value::Undefined);
                Ok(Value::Number(parse_int(&argument_string(args), radix)))
            }),
        ),
        "parseFloat" => Function::from_host(
            "parseFloat",
            Arc::new(|args| Ok(Value::Number(parse_float(&argument_string(args))))),
        ),
        "isNaN" => Function::from_host(
            "isNaN",
            Arc::new(|args| Ok(Value::Boolean(argument_number(args).is_nan()))),
        ),
        "isFinite" => Function::from_host(
            "isFinite",
            Arc::new(|args| Ok(Value::Boolean(argument_number(args).is_finite()))),
        ),
        "encodeURIComponent" => Function::from_host(
            "encodeURIComponent",
            Arc::new(|args| Ok(Value::String(encode_uri_component(&argument_string(args))))),
        ),
        "encodeURI" => Function::from_host(
            "encodeURI",
            Arc::new(|args| Ok(Value::String(encode_uri(&argument_string(args))))),
        ),
        "decodeURI" => Function::from_host(
            "decodeURI",
            Arc::new(|args| {
                let input = argument_string(args);
                super::errors::decode_uri(&input)
                    .map(Value::String)
                    .map_err(|_| {
                        JsError::thrown(super::errors::new_error("URIError", "URI malformed"))
                    })
            }),
        ),
        "escape" => Function::from_host(
            "escape",
            Arc::new(|args| Ok(Value::String(legacy_escape(&argument_string(args))))),
        ),
        "unescape" => Function::from_host(
            "unescape",
            Arc::new(|args| Ok(Value::String(legacy_unescape(&argument_string(args))))),
        ),
        _ => return None,
    };
    Some(Value::Function(function))
}

fn encode_uri(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || b";/?:@&=+$,#-_.!~*'()".contains(&byte) {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    output
}

fn legacy_escape(input: &str) -> String {
    let mut output = String::new();
    for unit in input.encode_utf16() {
        if unit <= 0x7f
            && (char::from(unit as u8).is_ascii_alphanumeric()
                || b"@*_+-./".contains(&(unit as u8)))
        {
            output.push(char::from(unit as u8));
        } else if unit < 0x100 {
            output.push_str(&format!("%{unit:02X}"));
        } else {
            output.push_str(&format!("%u{unit:04X}"));
        }
    }
    output
}

fn legacy_unescape(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::<u16>::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes
            .get(index..index + 2)
            .is_some_and(|pair| pair == b"%u" || pair == b"%U")
            && index + 6 <= bytes.len()
            && let Ok(unit) = u16::from_str_radix(&input[index + 2..index + 6], 16)
        {
            output.push(unit);
            index += 6;
            continue;
        }
        if bytes[index] == b'%'
            && index + 3 <= bytes.len()
            && let Ok(byte) = u8::from_str_radix(&input[index + 1..index + 3], 16)
        {
            output.push(u16::from(byte));
            index += 3;
            continue;
        }
        let ch = input[index..].chars().next().unwrap();
        output.extend(ch.encode_utf16(&mut [0; 2]).iter().copied());
        index += ch.len_utf8();
    }
    String::from_utf16_lossy(&output)
}

/// Encodes a URI component as uppercase hexadecimal escapes of UTF-8 bytes.
///
/// Only ASCII letters, digits, and `-_.!~*'()` pass through. Rust strings
/// cannot contain lone UTF-16 surrogates, so the ECMAScript URIError case
/// cannot reach this function; the lexer rejects those inputs earlier.
fn encode_uri_component(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    output
}

/// `ToString(args[0])`, where a missing argument is `undefined`.
fn argument_string(args: &[Value]) -> String {
    match args.first() {
        Some(value) => to_primitive(value).to_string(),
        None => "undefined".to_owned(),
    }
}

/// `ToNumber(args[0])`, where a missing argument is `undefined` (`NaN`).
fn argument_number(args: &[Value]) -> f64 {
    args.first()
        .map_or(f64::NAN, |value| to_number(&to_primitive(value)))
}

/// `parseInt(string, radix)` (ECMAScript §19.2.5) on an already-converted
/// string.
fn parse_int(input: &str, radix: &Value) -> f64 {
    let text = input.trim_start_matches(is_js_whitespace);
    let (sign, text) = match text.as_bytes().first() {
        Some(b'-') => (-1.0, &text[1..]),
        Some(b'+') => (1.0, &text[1..]),
        _ => (1.0, text),
    };
    let mut radix = to_int32(radix);
    let mut strip_prefix = true;
    if radix != 0 {
        if !(2..=36).contains(&radix) {
            return f64::NAN;
        }
        strip_prefix = radix == 16;
    } else {
        radix = 10;
    }
    let mut text = text;
    if strip_prefix && (text.starts_with("0x") || text.starts_with("0X")) {
        text = &text[2..];
        radix = 16;
    }
    let radix = radix as u32;
    let digits_end = text
        .find(|character: char| !character.is_digit(radix))
        .unwrap_or(text.len());
    let digits = &text[..digits_end];
    if digits.is_empty() {
        return f64::NAN;
    }
    let magnitude = if radix == 10 {
        // Rust's float parser rounds the full decimal string correctly.
        digits.parse().unwrap_or(f64::NAN)
    } else {
        digits.chars().fold(0.0, |total, character| {
            total * f64::from(radix) + f64::from(character.to_digit(radix).unwrap_or(0))
        })
    };
    sign * magnitude
}

/// `parseFloat(string)` (ECMAScript §19.2.4) on an already-converted string:
/// the longest prefix that is a `StrDecimalLiteral` (or `Infinity`), after
/// leading whitespace. Hex, octal, and binary prefixes are not recognized.
fn parse_float(input: &str) -> f64 {
    let text = input.trim_start_matches(is_js_whitespace);
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    if text[end..].starts_with("Infinity") {
        return if text.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let count_digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };
    let integer_digits = count_digits(end);
    end += integer_digits;
    let mut fraction_digits = 0;
    if bytes.get(end) == Some(&b'.') {
        fraction_digits = count_digits(end + 1);
        if integer_digits > 0 || fraction_digits > 0 {
            end += 1 + fraction_digits;
        }
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return f64::NAN;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent_end = end + 1;
        if matches!(bytes.get(exponent_end), Some(b'+' | b'-')) {
            exponent_end += 1;
        }
        let exponent_digits = count_digits(exponent_end);
        if exponent_digits > 0 {
            end = exponent_end + exponent_digits;
        }
    }
    text[..end].parse().unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_handles_signs_prefixes_and_radix_bounds() {
        let parse = |text: &str, radix: Value| parse_int(text, &radix);
        assert_eq!(parse("  42px", Value::Undefined), 42.0);
        assert_eq!(parse("-0x1F", Value::Undefined), -31.0);
        assert_eq!(parse("0x1F", Value::Number(16.0)), 31.0);
        assert!(parse("0x1F", Value::Number(10.0)) == 0.0);
        assert_eq!(parse("z", Value::Number(36.0)), 35.0);
        assert_eq!(parse("101", Value::Number(2.0)), 5.0);
        assert!(parse("12", Value::Number(1.0)).is_nan());
        assert!(parse("12", Value::Number(37.0)).is_nan());
        assert!(parse("", Value::Undefined).is_nan());
        assert!(parse("0x", Value::Undefined).is_nan());
        assert!(parse("-0", Value::Undefined).is_sign_negative());
        assert_eq!(parse("3.99", Value::Undefined), 3.0);
        assert_eq!(parse("1e3", Value::Undefined), 1.0);
    }

    #[test]
    fn parse_float_takes_the_longest_decimal_prefix() {
        assert_eq!(parse_float("  2.75abc"), 2.75);
        assert_eq!(parse_float(".5"), 0.5);
        assert_eq!(parse_float("5."), 5.0);
        assert_eq!(parse_float("-1.5e3x"), -1500.0);
        assert_eq!(parse_float("2e"), 2.0);
        assert_eq!(parse_float("2e+"), 2.0);
        assert_eq!(parse_float("0x10"), 0.0);
        assert_eq!(parse_float("-Infinityx"), f64::NEG_INFINITY);
        assert!(parse_float("-0").is_sign_negative());
        assert!(parse_float(".").is_nan());
        assert!(parse_float("abc").is_nan());
        assert!(parse_float("infinity").is_nan());
    }
}
