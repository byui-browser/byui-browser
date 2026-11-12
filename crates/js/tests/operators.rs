//! Source-level regression tests for the documented value and operator subset
//! (see `docs/js/value-operator-semantics.md`). Every case runs through the
//! tree-walk interpreter.

use js::{ExecutionMode, JsErrorCategory, JsResult, Value, eval_with_mode};

fn run(source: &str) -> JsResult<Value> {
    eval_with_mode(source, ExecutionMode::Interpreter)
}

fn number(value: f64) -> JsResult<Value> {
    Ok(Value::Number(value))
}

fn string(value: &str) -> JsResult<Value> {
    Ok(Value::String(value.into()))
}

fn boolean(value: bool) -> JsResult<Value> {
    Ok(Value::Boolean(value))
}

fn is_nan(source: &str) -> bool {
    matches!(run(source), Ok(Value::Number(value)) if value.is_nan())
}

#[test]
fn arithmetic_operators_on_numbers() {
    for (source, expected) in [
        ("1 + 2", 3.0),
        ("5 - 8", -3.0),
        ("6 * 7", 42.0),
        ("7 / 2", 3.5),
        ("7 % 3", 1.0),
        ("-7 % 3", -1.0),
        ("7 % -3", 1.0),
        ("5.5 % 2", 1.5),
        ("2 + 3 * 4 % 5", 4.0),
    ] {
        assert_eq!(run(source), number(expected), "{source}");
    }
}

#[test]
fn division_by_zero_follows_ieee_754_and_never_errors() {
    assert_eq!(run("1 / 0"), number(f64::INFINITY));
    assert_eq!(run("-1 / 0"), number(f64::NEG_INFINITY));
    assert_eq!(run("1 / -0"), number(f64::NEG_INFINITY));
    assert!(is_nan("0 / 0"));
    assert!(is_nan("1 % 0"));
    assert!(is_nan("(1 / 0) % 2"));
    assert_eq!(run("5 % (1 / 0)"), number(5.0));
    assert_eq!(
        run("(1 / 0) - (1 / 0) === (1 / 0) - (1 / 0)"),
        boolean(false)
    );
}

#[test]
fn remainder_is_lexed_parsed_and_evaluated_at_multiplicative_precedence() {
    assert_eq!(run("10 % 4"), number(2.0));
    assert_eq!(run("10%4"), number(2.0));
    assert_eq!(run("1 + 10 % 4"), number(3.0));
    assert_eq!(run("20 % 7 % 4"), number(2.0));
    assert_eq!(run("let n = 9; n % 4"), number(1.0));
    assert_eq!(run("'9' % '4'"), number(1.0));
    assert!(is_nan("'x' % 2"));
}

#[test]
fn numeric_conversion_covers_every_primitive_type() {
    for (source, expected) in [
        ("true * 1", 1.0),
        ("false * 1", 0.0),
        ("null * 1", 0.0),
        ("'' * 1", 0.0),
        ("'  7  ' * 1", 7.0),
        ("'1e3' * 1", 1000.0),
        ("-'5'", -5.0),
        ("-true", -1.0),
        ("-null", -0.0),
    ] {
        assert_eq!(run(source), number(expected), "{source}");
    }
    for source in [
        "undefined * 1",
        "undefined - 1",
        "'abc' - 1",
        "-undefined",
        "-'abc'",
        "0 / 0 + 1",
    ] {
        assert!(is_nan(source), "{source} should be NaN");
    }
}

#[test]
fn nan_is_unequal_to_everything_and_falsy() {
    assert_eq!(run("0 / 0 == 0 / 0"), boolean(false));
    assert_eq!(run("0 / 0 === 0 / 0"), boolean(false));
    assert_eq!(run("0 / 0 != 0 / 0"), boolean(true));
    assert_eq!(run("0 / 0 !== 0 / 0"), boolean(true));
    assert_eq!(run("0 / 0 < 1"), boolean(false));
    assert_eq!(run("0 / 0 >= 1"), boolean(false));
    assert_eq!(run("!(0 / 0)"), boolean(true));
    assert_eq!(run("0 / 0 || 'fallback'"), string("fallback"));
    assert_eq!(run("'' + 0 / 0"), string("NaN"));
}

#[test]
fn string_plus_string_concatenates() {
    assert_eq!(run("'foo' + 'bar'"), string("foobar"));
    assert_eq!(run("'' + ''"), string(""));
    assert_eq!(run("'a' + 'b' + 'c'"), string("abc"));
    assert_eq!(run("let s = 'x'; s + s"), string("xx"));
    assert_eq!(run("'héllo ' + 'wörld'"), string("héllo wörld"));
}

#[test]
fn string_plus_primitive_converts_the_primitive_to_text() {
    for (source, expected) in [
        ("'n=' + 1", "n=1"),
        ("1 + 'n'", "1n"),
        ("'n=' + 1.5", "n=1.5"),
        ("'n=' + -0", "n=0"),
        ("'b=' + true", "b=true"),
        ("false + '!'", "false!"),
        ("'v=' + null", "v=null"),
        ("'v=' + undefined", "v=undefined"),
        ("'inf=' + 1 / 0", "inf=Infinity"),
        ("'big=' + 1e21", "big=1e+21"),
        ("1 + 2 + 'x'", "3x"),
        ("'x' + 1 + 2", "x12"),
        ("'x' + (1 + 2)", "x3"),
    ] {
        assert_eq!(run(source), string(expected), "{source}");
    }
}

#[test]
fn plus_without_a_string_adds_numerically() {
    assert_eq!(run("true + true"), number(2.0));
    assert_eq!(run("null + null"), number(0.0));
    assert_eq!(run("true + null"), number(1.0));
    assert!(is_nan("undefined + 1"));
    assert!(is_nan("undefined + undefined"));
}

#[test]
fn only_addition_concatenates() {
    assert_eq!(run("'6' - '2'"), number(4.0));
    assert_eq!(run("'6' * '2'"), number(12.0));
    assert_eq!(run("'6' / '2'"), number(3.0));
    assert_eq!(run("'6' - 2"), number(4.0));
    assert!(is_nan("'a' * 2"));
}

#[test]
fn strict_equality_never_coerces_unlike_types() {
    for (source, expected) in [
        ("1 === 1", true),
        ("'1' === 1", false),
        ("1 === '1'", false),
        ("true === 1", false),
        ("false === 0", false),
        ("false === ''", false),
        ("null === undefined", false),
        ("null === 0", false),
        ("undefined === 0", false),
        ("'' === 0", false),
        ("'a' === 'a'", true),
        ("'a' === 'A'", false),
        ("0 === -0", true),
        ("'1' !== 1", true),
        ("1 !== 1", false),
        ("null !== undefined", true),
    ] {
        assert_eq!(run(source), boolean(expected), "{source}");
    }
}

#[test]
fn loose_equality_coerces_unlike_types() {
    for (source, expected) in [
        ("1 == '1'", true),
        ("'1' == 1", true),
        ("0 == ''", true),
        ("0 == '0'", true),
        ("'' == '0'", false),
        ("true == 1", true),
        ("false == 0", true),
        ("false == ''", true),
        ("true == '1'", true),
        ("true == 2", false),
        ("null == undefined", true),
        ("null == 0", false),
        ("null == false", false),
        ("undefined == 0", false),
        ("undefined == ''", false),
        ("'a' == 'a'", true),
        ("1 != '1'", false),
        ("1 != '2'", true),
    ] {
        assert_eq!(run(source), boolean(expected), "{source}");
    }
}

#[test]
fn loose_and_strict_equality_disagree_exactly_when_types_differ() {
    for (left, right) in [
        ("1", "'1'"),
        ("0", "false"),
        ("''", "0"),
        ("null", "undefined"),
    ] {
        assert_eq!(run(&format!("{left} == {right}")), boolean(true));
        assert_eq!(run(&format!("{left} === {right}")), boolean(false));
        assert_eq!(run(&format!("{left} != {right}")), boolean(false));
        assert_eq!(run(&format!("{left} !== {right}")), boolean(true));
    }
}

#[test]
fn comparison_operators_cover_numbers_strings_and_mixed_operands() {
    for (source, expected) in [
        ("1 < 2", true),
        ("2 < 1", false),
        ("2 > 1", true),
        ("2 <= 2", true),
        ("2 >= 3", false),
        ("'a' < 'b'", true),
        ("'B' < 'a'", true),
        ("'10' < '9'", true),
        ("'10' < 9", false),
        ("'2' > 1", true),
        ("true > false", true),
        ("null >= 0", true),
        ("null > 0", false),
        ("undefined < 1", false),
        ("undefined >= 1", false),
    ] {
        assert_eq!(run(source), boolean(expected), "{source}");
    }
}

#[test]
fn logical_operators_return_operands_and_short_circuit() {
    assert_eq!(run("1 && 2"), number(2.0));
    assert_eq!(run("0 && 2"), number(0.0));
    assert_eq!(run("'' || 'x'"), string("x"));
    assert_eq!(run("'a' || 'x'"), string("a"));
    assert_eq!(run("null && 1"), Ok(Value::Null));
    assert_eq!(run("undefined || null"), Ok(Value::Null));
    // The right side must never run (or fail) once the result is known.
    assert_eq!(run("false && missing"), boolean(false));
    assert_eq!(run("true || missing"), boolean(true));
    assert_eq!(run("let n = 0; 0 && (n = 1); n"), number(0.0));
    assert_eq!(run("let n = 0; 1 || (n = 1); n"), number(0.0));
    assert_eq!(run("let n = 0; 1 && (n = 1); n"), number(1.0));
}

#[test]
fn unary_operators_convert_their_operand() {
    for (source, expected) in [
        ("-5", number(-5.0)),
        ("- -5", number(5.0)),
        ("-'3'", number(-3.0)),
        ("!true", boolean(false)),
        ("!0", boolean(true)),
        ("!''", boolean(true)),
        ("!'0'", boolean(false)),
        ("!null", boolean(true)),
        ("!undefined", boolean(true)),
        ("!!'x'", boolean(true)),
        ("~0", number(-1.0)),
        ("~'5'", number(-6.0)),
        ("~(0 / 0)", number(-1.0)),
    ] {
        assert_eq!(run(source), expected, "{source}");
    }
}

#[test]
fn truthiness_table_for_every_supported_value_type() {
    for (source, expected) in [
        ("undefined", false),
        ("null", false),
        ("false", false),
        ("true", true),
        ("0", false),
        ("-0", false),
        ("0 / 0", false),
        ("1", true),
        ("-1", true),
        ("1 / 0", true),
        ("''", false),
        ("'0'", true),
        ("'false'", true),
        ("' '", true),
    ] {
        assert_eq!(run(&format!("!!({source})")), boolean(expected), "{source}");
    }
}

#[test]
fn operator_precedence_mixes_all_operator_families() {
    assert_eq!(run("1 + 2 * 3 % 4"), number(3.0));
    assert_eq!(run("1 + 1 === 2 && 'a' + 'b' === 'ab'"), boolean(true));
    assert_eq!(run("'a' + 1 == 'a1'"), boolean(true));
    assert_eq!(run("-2 % 3"), number(-2.0));
    assert_eq!(run("!1 == 0"), boolean(true));
}

#[test]
fn operator_failures_are_js_errors_not_panics() {
    for source in [
        "missing + 1",
        "1 + missing",
        "'a' + missing",
        "-missing",
        "!missing",
        "missing % 2",
        "2 % missing",
        "missing == 1",
        "1 === missing",
        "missing && 1",
        "let x = 1; x()",
    ] {
        let error = run(source).expect_err(source);
        assert_eq!(error.category, JsErrorCategory::Runtime, "{source}");
        assert!(!error.message.is_empty(), "{source}");
    }
}

#[test]
fn operator_failures_in_nested_expressions_stop_evaluation() {
    let error = run("let n = 0; function f() { return missing % 2; } n = 1 + f(); n").unwrap_err();
    assert_eq!(error.category, JsErrorCategory::Runtime);
    assert_eq!(error.message, "`missing` is not defined");
}

#[test]
fn pathological_operands_do_not_panic() {
    let long = String::from("'x'") + &" + 'x'".repeat(200);
    assert!(matches!(run(&long), Ok(Value::String(text)) if text.len() == 201));
    for source in [
        "1e308 * 10",
        "-1e308 * 10",
        "1e308 % 1e-308",
        "4294967296 << 1",
        "-4294967297 >>> 1",
        "'\u{0}' * 1",
        "1 << (0 / 0)",
        "~(1 / 0)",
    ] {
        assert!(run(source).is_ok(), "{source}");
    }
}

#[test]
fn unsupported_operator_syntax_is_a_syntax_error() {
    for source in ["typeof 1", "+1", "1 ** 2", "x += 1", "1 ?? 2", "a++"] {
        let error = run(source).expect_err(source);
        assert_eq!(error.category, JsErrorCategory::Syntax, "{source}");
    }
}
