//! Integration coverage for JavaScript value conversions.

use js::{Value, eval};

#[test]
fn formats_numbers_like_javascript() {
    for (number, expected) in [
        (1.0, "1"),
        (-0.0, "0"),
        (1.5, "1.5"),
        (0.1 + 0.2, "0.30000000000000004"),
        (123_456_789.0, "123456789"),
        (1e21, "1e+21"),
        (1.5e21, "1.5e+21"),
        (1e20, "100000000000000000000"),
        (0.000001, "0.000001"),
        (1e-7, "1e-7"),
        (-2.5e-8, "-2.5e-8"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (f64::NAN, "NaN"),
    ] {
        assert_eq!(Value::Number(number).to_string(), expected, "{number:?}");
    }
}

#[test]
fn converts_strings_like_javascript() {
    for (text, expected) in [
        ("", 0.0),
        ("  \\n\\t ", 0.0),
        (" 42 ", 42.0),
        ("-1.5e3", -1500.0),
        (".5", 0.5),
        ("5.", 5.0),
        ("+Infinity", f64::INFINITY),
        ("-Infinity", f64::NEG_INFINITY),
        ("0x10", 16.0),
        ("0B101", 5.0),
        ("0o17", 15.0),
    ] {
        assert_eq!(
            eval(&format!("'{text}' * 1")),
            Ok(Value::Number(expected)),
            "{text:?}"
        );
    }
    for text in [
        "abc", "inf", "infinity", "NaN", "1e", ".", "0x", "-0x10", "1 2", "e5",
    ] {
        let Ok(Value::Number(value)) = eval(&format!("'{text}' * 1")) else {
            panic!("{text:?} should evaluate to a number");
        };
        assert!(value.is_nan(), "{text:?}");
    }
}
