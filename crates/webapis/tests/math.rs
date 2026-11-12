//! Script-level tests for the `Math` namespace installed with the window.

use js::{Realm, Value};
use webapis::register_window;

fn realm() -> Realm {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
}

fn number(source: &str) -> f64 {
    match realm().evaluate_script(source) {
        Ok(Value::Number(number)) => number,
        other => panic!("`{source}` should evaluate to a number, got {other:?}"),
    }
}

fn is_true(source: &str) -> bool {
    realm().evaluate_script(source) == Ok(Value::Boolean(true))
}

fn close(source: &str, expected: f64) {
    let actual = number(source);
    assert!(
        (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0),
        "`{source}` was {actual}, expected {expected}"
    );
}

fn negative_zero(source: &str) {
    let actual = number(source);
    assert!(
        actual == 0.0 && actual.is_sign_negative(),
        "`{source}` was {actual}, expected -0"
    );
}

#[test]
fn math_is_a_stable_global_shared_with_window() {
    assert!(is_true("window.Math === Math && Math === Math"));
}

#[test]
fn constants_have_their_standard_values_and_are_read_only() {
    for (name, expected) in [
        ("E", std::f64::consts::E),
        ("LN10", std::f64::consts::LN_10),
        ("LN2", std::f64::consts::LN_2),
        ("LOG10E", std::f64::consts::LOG10_E),
        ("LOG2E", std::f64::consts::LOG2_E),
        ("PI", std::f64::consts::PI),
        ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
        ("SQRT2", std::f64::consts::SQRT_2),
    ] {
        assert_eq!(number(&format!("Math.{name}")), expected, "{name}");
    }
    let realm = realm();
    let _ = realm.evaluate_script("Math.PI = 3");
    assert_eq!(
        realm.evaluate_script("Math.PI"),
        Ok(Value::Number(std::f64::consts::PI))
    );
}

#[test]
fn every_standard_function_is_callable() {
    for name in [
        "abs",
        "acos",
        "acosh",
        "asin",
        "asinh",
        "atan",
        "atan2",
        "atanh",
        "cbrt",
        "ceil",
        "clz32",
        "cos",
        "cosh",
        "exp",
        "expm1",
        "f16round",
        "floor",
        "fround",
        "hypot",
        "imul",
        "log",
        "log10",
        "log1p",
        "log2",
        "max",
        "min",
        "pow",
        "random",
        "round",
        "sign",
        "sin",
        "sinh",
        "sqrt",
        "sumPrecise",
        "tan",
        "tanh",
        "trunc",
    ] {
        assert!(
            is_true(&format!("typeof Math.{name} === 'function'")),
            "Math.{name} should be a function"
        );
    }
}

#[test]
fn rounding_functions_follow_ecmascript_rules() {
    assert_eq!(number("Math.abs(-7.5)"), 7.5);
    assert_eq!(number("Math.ceil(1.2)"), 2.0);
    negative_zero("Math.ceil(-0.5)");
    assert_eq!(number("Math.floor(-1.2)"), -2.0);
    assert_eq!(number("Math.trunc(-1.8)"), -1.0);
    assert_eq!(number("Math.round(2.5)"), 3.0);
    assert_eq!(number("Math.round(-2.5)"), -2.0);
    assert_eq!(number("Math.round(2.4)"), 2.0);
    negative_zero("Math.round(-0.5)");
    assert_eq!(number("Math.sign(-3)"), -1.0);
    assert_eq!(number("Math.sign(12)"), 1.0);
    negative_zero("Math.sign(-0)");
    assert!(number("Math.sign('x')").is_nan());
    assert_eq!(number("Math.fround(5.5)"), 5.5);
    assert_eq!(number("Math.fround(5.05)"), f64::from(5.05_f32));
    assert_eq!(number("Math.f16round(1.337)"), 1.336_914_062_5);
    assert_eq!(number("Math.f16round(70000)"), f64::INFINITY);
}

#[test]
fn exponential_and_logarithmic_functions() {
    assert_eq!(number("Math.sqrt(81)"), 9.0);
    assert!(number("Math.sqrt(-1)").is_nan());
    assert_eq!(number("Math.cbrt(-27)"), -3.0);
    assert_eq!(number("Math.pow(2, 10)"), 1024.0);
    assert_eq!(number("Math.pow(0/0, 0)"), 1.0);
    assert!(number("Math.pow(1, 1/0)").is_nan());
    assert!(number("Math.pow(-1, -1/0)").is_nan());
    assert!(number("Math.pow(1, 0/0)").is_nan());
    assert!(number("Math.pow(2)").is_nan());
    close("Math.exp(1)", std::f64::consts::E);
    close("Math.expm1(1)", std::f64::consts::E - 1.0);
    assert_eq!(number("Math.expm1(0)"), 0.0);
    close("Math.log(Math.E)", 1.0);
    assert_eq!(number("Math.log(0)"), f64::NEG_INFINITY);
    assert!(number("Math.log(-1)").is_nan());
    close("Math.log10(1000)", 3.0);
    assert_eq!(number("Math.log2(8)"), 3.0);
    close("Math.log1p(Math.E - 1)", 1.0);
}

#[test]
fn trigonometric_and_hyperbolic_functions() {
    use std::f64::consts::PI;
    assert_eq!(number("Math.sin(0)"), 0.0);
    close("Math.sin(Math.PI / 2)", 1.0);
    close("Math.cos(Math.PI)", -1.0);
    close("Math.tan(Math.PI / 4)", 1.0);
    close("Math.asin(1)", PI / 2.0);
    close("Math.acos(-1)", PI);
    assert!(number("Math.acos(2)").is_nan());
    close("Math.atan(1)", PI / 4.0);
    close("Math.atan2(1, 1)", PI / 4.0);
    close("Math.atan2(1, -1)", 3.0 * PI / 4.0);
    close("Math.atan2(0, -1)", PI);
    close("Math.sinh(1)", 1.0_f64.sinh());
    close("Math.cosh(1)", 1.0_f64.cosh());
    close("Math.tanh(1)", 1.0_f64.tanh());
    close("Math.asinh(1)", 1.0_f64.asinh());
    close("Math.acosh(2)", 2.0_f64.acosh());
    assert!(number("Math.acosh(0.5)").is_nan());
    close("Math.atanh(0.5)", 0.5_f64.atanh());
    assert_eq!(number("Math.atanh(1)"), f64::INFINITY);
}

#[test]
fn variadic_functions_handle_any_argument_count() {
    assert_eq!(number("Math.max()"), f64::NEG_INFINITY);
    assert_eq!(number("Math.min()"), f64::INFINITY);
    assert_eq!(number("Math.max(1, 7, 3)"), 7.0);
    assert_eq!(number("Math.min(1, -7, 3)"), -7.0);
    assert!(number("Math.max(1, 'x', 3)").is_nan());
    assert!(number("Math.min(1, undefined)").is_nan());
    assert!(number("Math.max(-0, 0)").is_sign_positive());
    assert!(number("Math.max(0, -0)").is_sign_positive());
    negative_zero("Math.min(0, -0)");
    negative_zero("Math.min(-0, 0)");
    assert_eq!(number("Math.hypot()"), 0.0);
    assert_eq!(number("Math.hypot(3, 4)"), 5.0);
    assert_eq!(number("Math.hypot(-3)"), 3.0);
    close("Math.hypot(1, 2, 3)", 14.0_f64.sqrt());
    close("Math.hypot(3e200, 4e200)", 5e200);
    assert_eq!(number("Math.hypot(0/0, 1/0)"), f64::INFINITY);
    assert!(number("Math.hypot(0/0, 1)").is_nan());
}

#[test]
fn integer_functions_use_32_bit_semantics() {
    assert_eq!(number("Math.clz32(1)"), 31.0);
    assert_eq!(number("Math.clz32(0)"), 32.0);
    assert_eq!(number("Math.clz32()"), 32.0);
    assert_eq!(number("Math.clz32(-1)"), 0.0);
    assert_eq!(number("Math.clz32(4294967296 + 4)"), 29.0);
    assert_eq!(number("Math.imul(3, 4)"), 12.0);
    assert_eq!(number("Math.imul(-5, 12)"), -60.0);
    assert_eq!(number("Math.imul(4294967295, 5)"), -5.0);
    assert_eq!(number("Math.imul(65536, 65536)"), 0.0);
    assert_eq!(number("Math.imul(2147483647, 2)"), -2.0);
    assert_eq!(number("Math.imul(2)"), 0.0);
}

#[test]
fn arguments_are_converted_with_to_number() {
    assert_eq!(number("Math.abs('-3')"), 3.0);
    assert_eq!(number("Math.abs(null)"), 0.0);
    assert_eq!(number("Math.abs(true)"), 1.0);
    assert_eq!(number("Math.abs([])"), 0.0);
    assert_eq!(number("Math.abs([-4])"), 4.0);
    assert!(number("Math.abs()").is_nan());
    assert!(number("Math.abs({})").is_nan());
    assert!(number("Math.abs('three')").is_nan());
    assert_eq!(number("Math.abs({ valueOf: () => -9 })"), 9.0);
}

#[test]
fn random_returns_varying_numbers_in_the_unit_interval() {
    assert!(is_true(
        "let ok = true; let first = Math.random(); let varied = false; \
         for (let i = 0; i < 200; i = i + 1) { \
             let sample = Math.random(); \
             if (sample < 0 || sample >= 1) { ok = false; } \
             if (sample !== first) { varied = true; } \
         } \
         ok && varied",
    ));
}

#[test]
fn sum_precise_adds_arrays_without_intermediate_rounding() {
    assert_eq!(number("Math.sumPrecise([1e20, 0.1, -1e20])"), 0.1);
    assert_eq!(
        number("Math.sumPrecise([0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1])"),
        1.0
    );
    assert_eq!(number("Math.sumPrecise([1e308, 1e308, -1e308])"), 1e308);
    negative_zero("Math.sumPrecise([])");
    assert!(number("Math.sumPrecise([1/0, -1/0])").is_nan());
}

#[test]
fn sum_precise_rejects_non_arrays_and_non_numbers_with_a_type_error() {
    for source in [
        "Math.sumPrecise()",
        "Math.sumPrecise(5)",
        "Math.sumPrecise([1, '2'])",
    ] {
        assert!(
            is_true(&format!(
                "let name = ''; try {{ {source}; }} catch (error) {{ name = error.name; }} \
                 name === 'TypeError'"
            )),
            "`{source}` should throw a TypeError"
        );
    }
}
