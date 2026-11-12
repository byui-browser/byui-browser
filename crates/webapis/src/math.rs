//! The script-visible ECMAScript `Math` namespace object.

use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use getrandom::fill;
use js::{HostFunction, JsError, JsResult, Object, Realm, Value};

/// `2 ** 1023`, half of the first power of two an `f64` cannot hold.
const TWO_POW_1023: f64 = f64::from_bits(0x7FE0_0000_0000_0000);
/// `2 ** 970`, half the gap between `f64::MAX` and the float below it.
const HALF_MAX_ULP: f64 = f64::from_bits(0x7C90_0000_0000_0000);
/// `2 ** 32`, the modulus of `ToUint32`.
const TWO_POW_32: f64 = 4_294_967_296.0;

fn type_error(message: impl Into<String>) -> JsError {
    let exception = Object::new();
    exception.set("name", Value::String("TypeError".into()));
    exception.set("message", Value::String(message.into()));
    JsError::thrown(Value::Object(exception))
}

/// Whether `ch` is ECMAScript `WhiteSpace` or a `LineTerminator`.
fn is_js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
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

/// `StringToNumber`: surrounding whitespace is ignored, empty text is `0`,
/// `0x`/`0o`/`0b` prefixes and `Infinity` are recognized, and anything else
/// that is not a decimal literal is `NaN`.
fn string_to_number(text: &str) -> f64 {
    let text = text.trim_matches(is_js_whitespace);
    if text.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if text
            .get(..2)
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        {
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

/// `ToNumber`, matching the engine's own arithmetic operators.
///
/// Objects are converted through their `valueOf` and then `toString` methods
/// when those return a primitive; an error thrown by either propagates to the
/// calling script. Objects without such a method, arrays, and functions are
/// converted through their string form, so `[]` is `0`, `[5]` is `5`, and a
/// plain object is `NaN`.
fn to_number(value: &Value) -> JsResult<f64> {
    Ok(match value {
        Value::Number(number) => *number,
        Value::Boolean(true) => 1.0,
        Value::Boolean(false) | Value::Null => 0.0,
        Value::Undefined => f64::NAN,
        Value::String(text) => string_to_number(text),
        Value::Object(object) => {
            for name in ["valueOf", "toString"] {
                let Some(Value::Function(method)) = object.get_property(name)? else {
                    continue;
                };
                let result = method.call_with_this(value.clone(), &[])?;
                if !matches!(
                    result,
                    Value::Object(_) | Value::Function(_) | Value::Array(_)
                ) {
                    return to_number(&result);
                }
            }
            f64::NAN
        }
        Value::Function(_) | Value::Array(_) => string_to_number(&value.to_string()),
    })
}

/// Converts the argument at `index`, treating a missing one as `undefined`.
fn number_argument(arguments: &[Value], index: usize) -> JsResult<f64> {
    arguments.get(index).map_or(Ok(f64::NAN), to_number)
}

/// Converts every argument before any is used, as the variadic `Math`
/// functions require.
fn number_arguments(arguments: &[Value]) -> JsResult<Vec<f64>> {
    arguments.iter().map(to_number).collect()
}

/// `ToUint32`: truncates toward zero and wraps modulo `2 ** 32`.
fn to_uint32(number: f64) -> u32 {
    if !number.is_finite() {
        return 0;
    }
    number.trunc().rem_euclid(TWO_POW_32) as u32
}

fn unary(math: &Object, name: &str, operation: fn(f64) -> f64) {
    let function: HostFunction =
        Arc::new(move |arguments| Ok(Value::Number(operation(number_argument(arguments, 0)?))));
    math.set_method(name, function);
}

fn binary(math: &Object, name: &str, operation: fn(f64, f64) -> f64) {
    let function: HostFunction = Arc::new(move |arguments| {
        let first = number_argument(arguments, 0)?;
        let second = number_argument(arguments, 1)?;
        Ok(Value::Number(operation(first, second)))
    });
    math.set_method(name, function);
}

fn variadic(math: &Object, name: &str, operation: fn(&[f64]) -> f64) {
    let function: HostFunction =
        Arc::new(move |arguments| Ok(Value::Number(operation(&number_arguments(arguments)?))));
    math.set_method(name, function);
}

/// `Math.round`: nearest integer, with ties going toward positive infinity
/// and the sign of zero preserved (`round(-0.5)` is `-0`).
fn round(number: f64) -> f64 {
    if !number.is_finite() || number == number.trunc() {
        return number;
    }
    let floor = number.floor();
    // `number - floor` is exact for every non-integral double, unlike
    // `number + 0.5`, which rounds 0.49999999999999994 up to 1.
    let rounded = if number - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    };
    rounded.copysign(number)
}

/// `Math.sign`: `1`, `-1`, or the argument itself for zeros and `NaN`.
fn sign(number: f64) -> f64 {
    if number.is_nan() || number == 0.0 {
        number
    } else {
        1.0_f64.copysign(number)
    }
}

/// `Math.pow`, which unlike IEEE 754 `pow` yields `NaN` for a `NaN` exponent
/// and for a base of `1` or `-1` raised to an infinite exponent.
fn pow(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() || (base.abs() == 1.0 && exponent.is_infinite()) {
        f64::NAN
    } else {
        base.powf(exponent)
    }
}

/// `Math.max`: `-Infinity` for no arguments, `NaN` if any argument is `NaN`,
/// and `+0` ranks above `-0`.
fn max(numbers: &[f64]) -> f64 {
    numbers.iter().fold(f64::NEG_INFINITY, |highest, &number| {
        if highest.is_nan() || number.is_nan() {
            f64::NAN
        } else if number > highest || (number == highest && highest.is_sign_negative()) {
            number
        } else {
            highest
        }
    })
}

/// `Math.min`: `Infinity` for no arguments, `NaN` if any argument is `NaN`,
/// and `-0` ranks below `+0`.
fn min(numbers: &[f64]) -> f64 {
    numbers.iter().fold(f64::INFINITY, |lowest, &number| {
        if lowest.is_nan() || number.is_nan() {
            f64::NAN
        } else if number < lowest || (number == lowest && number.is_sign_negative()) {
            number
        } else {
            lowest
        }
    })
}

/// `Math.hypot`: an infinite argument wins over `NaN`, and the operands are
/// scaled by the largest magnitude so the squares neither overflow nor
/// underflow.
fn hypot(numbers: &[f64]) -> f64 {
    if numbers.iter().any(|number| number.is_infinite()) {
        return f64::INFINITY;
    }
    if numbers.iter().any(|number| number.is_nan()) {
        return f64::NAN;
    }
    let largest = numbers
        .iter()
        .fold(0.0_f64, |largest, number| largest.max(number.abs()));
    if largest == 0.0 {
        return 0.0;
    }
    let sum: f64 = numbers
        .iter()
        .map(|number| (number / largest).powi(2))
        .sum();
    sum.sqrt() * largest
}

/// `Math.fround`: the nearest single-precision value, widened back.
fn fround(number: f64) -> f64 {
    f64::from(number as f32)
}

/// `Math.f16round`: the nearest IEEE 754 half-precision value (ties to even),
/// widened back. Magnitudes of 65520 and above become infinite.
fn f16round(number: f64) -> f64 {
    const MIN_EXPONENT: i32 = -14;
    const MAX_FINITE: f64 = 65_504.0;
    let magnitude = number.abs();
    if !number.is_finite() || magnitude == 0.0 {
        return number;
    }
    // Half of the smallest subnormal half-float (2 ** -24) ties to even, zero.
    if magnitude <= 2.0_f64.powi(-25) {
        return 0.0_f64.copysign(number);
    }
    let exponent = (((magnitude.to_bits() >> 52) & 0x7ff) as i32 - 1023).max(MIN_EXPONENT);
    // Half-floats carry 10 fraction bits, so values in this binade are
    // multiples of `quantum`. Scaling by a power of two is exact.
    let quantum = 2.0_f64.powi(exponent - 10);
    let rounded = (magnitude / quantum).round_ties_even() * quantum;
    if rounded > MAX_FINITE {
        f64::INFINITY.copysign(number)
    } else {
        rounded.copysign(number)
    }
}

/// Adds two doubles exactly as `(sum, error)`. Requires `|x| >= |y|`.
fn fast_two_sum(x: f64, y: f64) -> (f64, f64) {
    let hi = x + y;
    (hi, y - (hi - x))
}

fn by_magnitude(x: f64, y: f64) -> (f64, f64) {
    if x.abs() < y.abs() { (y, x) } else { (x, y) }
}

/// The correctly rounded sum of `values`, as `Math.sumPrecise` defines it.
///
/// This is Shewchuk's exact-partials summation extended, as in the TC39
/// proposal's reference implementation, with a counter of `2 ** 1024`
/// overflows so intermediate sums beyond `f64::MAX` still produce the right
/// finite result. The sum of no values, or of only `-0`, is `-0`.
fn sum_precise(values: &[f64]) -> f64 {
    if values.iter().any(|value| value.is_nan()) {
        return f64::NAN;
    }
    let has_positive_infinity = values.contains(&f64::INFINITY);
    let has_negative_infinity = values.contains(&f64::NEG_INFINITY);
    match (has_positive_infinity, has_negative_infinity) {
        (true, true) => return f64::NAN,
        (true, false) => return f64::INFINITY,
        (false, true) => return f64::NEG_INFINITY,
        (false, false) => {}
    }
    if values
        .iter()
        .all(|value| *value == 0.0 && value.is_sign_negative())
    {
        return -0.0;
    }

    // Non-overlapping partial sums in increasing magnitude; their exact sum,
    // plus `overflow * 2 ** 1024`, is the exact sum of the values seen.
    let mut partials: Vec<f64> = Vec::new();
    let mut overflow = 0.0_f64;
    for &value in values {
        let mut x = value;
        let mut used = 0;
        for index in 0..partials.len() {
            let (larger, smaller) = by_magnitude(x, partials[index]);
            let (mut hi, mut lo) = fast_two_sum(larger, smaller);
            if hi.is_infinite() {
                let sign = hi.signum();
                overflow += sign;
                let reduced = (larger - sign * TWO_POW_1023) - sign * TWO_POW_1023;
                let (larger, smaller) = by_magnitude(reduced, smaller);
                (hi, lo) = fast_two_sum(larger, smaller);
            }
            if lo != 0.0 {
                partials[used] = lo;
                used += 1;
            }
            x = hi;
        }
        partials.truncate(used);
        if x != 0.0 {
            partials.push(x);
        }
    }

    let mut hi = 0.0_f64;
    let mut lo = 0.0_f64;
    if overflow != 0.0 {
        let next = partials.pop().unwrap_or(0.0);
        if overflow.abs() > 1.0 || (overflow > 0.0 && next > 0.0) || (overflow < 0.0 && next < 0.0)
        {
            return f64::INFINITY.copysign(overflow);
        }
        // Work at half scale so the single overflow can be added exactly.
        (hi, lo) = fast_two_sum(overflow * TWO_POW_1023, next / 2.0);
        lo *= 2.0;
        if (2.0 * hi).is_infinite() {
            // Exactly half an ulp short of 2 ** 1024 rounds to infinity under
            // ties-to-even unless a lower partial pulls it back toward MAX.
            let below = partials.last().copied().unwrap_or(0.0);
            let pulled_back =
                hi.abs() == TWO_POW_1023 && lo == -HALF_MAX_ULP.copysign(hi) && below * hi < 0.0;
            return if pulled_back { f64::MAX } else { f64::INFINITY }.copysign(hi);
        }
        if lo != 0.0 {
            partials.push(lo);
            lo = 0.0;
        }
        hi *= 2.0;
    }
    while let Some(partial) = partials.pop() {
        (hi, lo) = fast_two_sum(hi, partial);
        if lo != 0.0 {
            break;
        }
    }
    // When the error is exactly half an ulp, the next partial's sign decides
    // which way the result rounds.
    if let Some(&below) = partials.last()
        && ((lo < 0.0 && below < 0.0) || (lo > 0.0 && below > 0.0))
    {
        let doubled = lo * 2.0;
        let candidate = hi + doubled;
        if doubled == candidate - hi {
            hi = candidate;
        }
    }
    hi
}

fn sum_precise_binding(arguments: &[Value]) -> JsResult<Value> {
    let Some(Value::Array(array)) = arguments.first() else {
        return Err(type_error("Math.sumPrecise() requires an array of numbers"));
    };
    let length = match array.get("length") {
        Some(Value::Number(length)) if length.is_finite() && length >= 0.0 => length as usize,
        _ => 0,
    };
    let mut values = Vec::with_capacity(length);
    for index in 0..length {
        let Some(Value::Number(number)) = array.get(&index.to_string()) else {
            return Err(type_error(
                "Math.sumPrecise() requires every element to be a number",
            ));
        };
        values.push(number);
    }
    Ok(Value::Number(sum_precise(&values)))
}

/// Seeds the `Math.random` generator from the operating system, falling back
/// to the clock and a process-wide counter if no entropy source is available.
fn random_seed() -> u64 {
    static FALLBACK_COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut bytes = [0; 8];
    if fill(&mut bytes).is_ok() {
        return u64::from_le_bytes(bytes);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    nanos
        ^ FALLBACK_COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Advances a SplitMix64 state and returns a uniform double in `[0, 1)` built
/// from the top 53 bits of its output.
fn next_random(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^= mixed >> 31;
    (mixed >> 11) as f64 / (1_u64 << 53) as f64
}

/// Builds the `Math` namespace object with its constants and functions.
fn math_object() -> Object {
    let math = Object::new();
    for (name, value) in [
        ("E", std::f64::consts::E),
        ("LN10", std::f64::consts::LN_10),
        ("LN2", std::f64::consts::LN_2),
        ("LOG10E", std::f64::consts::LOG10_E),
        ("LOG2E", std::f64::consts::LOG2_E),
        ("PI", std::f64::consts::PI),
        ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
        ("SQRT2", std::f64::consts::SQRT_2),
    ] {
        math.set_read_only(name, Value::Number(value));
    }

    unary(&math, "abs", f64::abs);
    unary(&math, "acos", f64::acos);
    unary(&math, "acosh", f64::acosh);
    unary(&math, "asin", f64::asin);
    unary(&math, "asinh", f64::asinh);
    unary(&math, "atan", f64::atan);
    unary(&math, "atanh", f64::atanh);
    unary(&math, "cbrt", f64::cbrt);
    unary(&math, "ceil", f64::ceil);
    unary(&math, "clz32", |number| {
        f64::from(to_uint32(number).leading_zeros())
    });
    unary(&math, "cos", f64::cos);
    unary(&math, "cosh", f64::cosh);
    unary(&math, "exp", f64::exp);
    unary(&math, "expm1", f64::exp_m1);
    unary(&math, "f16round", f16round);
    unary(&math, "floor", f64::floor);
    unary(&math, "fround", fround);
    unary(&math, "log", f64::ln);
    unary(&math, "log10", f64::log10);
    unary(&math, "log1p", f64::ln_1p);
    unary(&math, "log2", f64::log2);
    unary(&math, "round", round);
    unary(&math, "sign", sign);
    unary(&math, "sin", f64::sin);
    unary(&math, "sinh", f64::sinh);
    unary(&math, "sqrt", f64::sqrt);
    unary(&math, "tan", f64::tan);
    unary(&math, "tanh", f64::tanh);
    unary(&math, "trunc", f64::trunc);

    binary(&math, "atan2", f64::atan2);
    binary(&math, "imul", |left, right| {
        f64::from(to_uint32(left).wrapping_mul(to_uint32(right)) as i32)
    });
    binary(&math, "pow", pow);

    variadic(&math, "hypot", hypot);
    variadic(&math, "max", max);
    variadic(&math, "min", min);

    let state = Mutex::new(random_seed());
    let random: HostFunction = Arc::new(move |_| {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(Value::Number(next_random(&mut state)))
    });
    math.set_method("random", random);
    let sum_precise: HostFunction = Arc::new(sum_precise_binding);
    math.set_method("sumPrecise", sum_precise);
    math
}

/// Registers the ECMAScript `Math` namespace as `window.Math` and as the
/// global `Math`.
///
/// `window` is the realm's window object; it and the realm share one `Math`
/// object. The object carries the eight read-only constants (`E`, `LN10`,
/// `LN2`, `LOG10E`, `LOG2E`, `PI`, `SQRT1_2`, `SQRT2`) and every standard
/// function: `abs`, `acos`, `acosh`, `asin`, `asinh`, `atan`, `atan2`,
/// `atanh`, `cbrt`, `ceil`, `clz32`, `cos`, `cosh`, `exp`, `expm1`,
/// `f16round`, `floor`, `fround`, `hypot`, `imul`, `log`, `log10`, `log1p`,
/// `log2`, `max`, `min`, `pow`, `random`, `round`, `sign`, `sin`, `sinh`,
/// `sqrt`, `tan`, `tanh`, `trunc`, and `sumPrecise`. Arguments are converted
/// with `ToNumber`, and missing arguments are `undefined` (`NaN`).
///
/// Known limitations:
/// - `Math.sumPrecise()` accepts only this engine's ordinary arrays, not
///   arbitrary iterables, because host functions cannot drive the iterator
///   protocol.
/// - `Math.random()` is a SplitMix64 generator seeded once per registration
///   from the operating system. It is not cryptographically secure; use
///   `crypto.getRandomValues()` for that.
/// - The object has no `Symbol.toStringTag`, so `String(Math)` is
///   `[object Object]` rather than `[object Math]`.
/// - Transcendental functions use the Rust standard library, so results may
///   differ from other engines in the last bit, as the specification allows.
pub fn register_math(realm: &mut Realm, window: &Object) -> JsResult<()> {
    let math = math_object();
    window.set("Math", Value::Object(math.clone()));
    realm.register_global_value("Math", Value::Object(math))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_breaks_ties_toward_positive_infinity() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(0.499_999_999_999_999_94), 0.0);
        assert_eq!(round(4_503_599_627_370_497.0), 4_503_599_627_370_497.0);
        assert!(round(-0.5).is_sign_negative());
        assert!(round(-0.2).is_sign_negative());
        assert!(round(f64::NAN).is_nan());
    }

    #[test]
    fn f16round_rounds_to_nearest_half_float() {
        assert_eq!(f16round(1.5), 1.5);
        assert_eq!(f16round(1.337), 1.336_914_062_5);
        assert_eq!(f16round(65_519.99), 65_504.0);
        assert_eq!(f16round(65_520.0), f64::INFINITY);
        assert_eq!(f16round(-65_520.0), f64::NEG_INFINITY);
        // The smallest subnormal half-float and the tie just below it.
        assert_eq!(f16round(2.0_f64.powi(-24)), 2.0_f64.powi(-24));
        assert_eq!(f16round(2.0_f64.powi(-25)), 0.0);
        assert_eq!(f16round(1.5 * 2.0_f64.powi(-24)), 2.0_f64.powi(-23));
        assert!(f16round(-1e-30).is_sign_negative());
        assert!(f16round(f64::NAN).is_nan());
    }

    #[test]
    fn power_of_two_constants_are_exact() {
        assert_eq!(TWO_POW_1023, 2.0_f64.powi(1023));
        assert_eq!(HALF_MAX_ULP, 2.0_f64.powi(970));
        assert_eq!(f64::MAX - f64::MAX.next_down(), 2.0 * HALF_MAX_ULP);
    }

    #[test]
    fn sum_precise_is_correctly_rounded() {
        assert_eq!(sum_precise(&[1e20, 0.1, -1e20]), 0.1);
        assert_eq!(sum_precise(&[0.1; 10]), 1.0);
        assert_eq!(sum_precise(&[1e308, 1e308, -1e308]), 1e308);
        assert_eq!(sum_precise(&[-1e308, -1e308, 1e308]), -1e308);
        assert_eq!(sum_precise(&[1e308, 1e308]), f64::INFINITY);
        assert_eq!(
            sum_precise(&[f64::MAX, HALF_MAX_ULP, -1.0]),
            f64::MAX,
            "just under the rounding boundary stays finite"
        );
        assert_eq!(sum_precise(&[f64::MAX, HALF_MAX_ULP]), f64::INFINITY);
        assert_eq!(sum_precise(&[1.0, -1.0]), 0.0);
        assert!(sum_precise(&[1.0, -1.0]).is_sign_positive());
    }

    #[test]
    fn sum_precise_handles_zeros_and_non_finite_values() {
        assert!(sum_precise(&[]).is_sign_negative());
        assert!(sum_precise(&[-0.0, -0.0]).is_sign_negative());
        assert!(sum_precise(&[-0.0, 0.0]).is_sign_positive());
        assert_eq!(sum_precise(&[f64::INFINITY, 1.0]), f64::INFINITY);
        assert!(sum_precise(&[f64::INFINITY, f64::NEG_INFINITY]).is_nan());
        assert!(sum_precise(&[1.0, f64::NAN]).is_nan());
    }

    #[test]
    fn string_arguments_follow_string_to_number() {
        assert_eq!(string_to_number("  12.5\n"), 12.5);
        assert_eq!(string_to_number(""), 0.0);
        assert_eq!(string_to_number("0x1F"), 31.0);
        assert_eq!(string_to_number("-Infinity"), f64::NEG_INFINITY);
        assert!(string_to_number("inf").is_nan());
        assert!(string_to_number("1px").is_nan());
    }

    #[test]
    fn random_stays_in_the_unit_interval() {
        let mut state = random_seed();
        let samples: Vec<f64> = (0..1_000).map(|_| next_random(&mut state)).collect();
        assert!(samples.iter().all(|sample| (0.0..1.0).contains(sample)));
        assert!(samples.windows(2).any(|pair| pair[0] != pair[1]));
    }
}
