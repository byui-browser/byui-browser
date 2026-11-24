//! Regression coverage for declaration lists and hexadecimal literals.
use js::{ExecutionMode, Realm, Value, eval_with_mode, parse};

#[test]
fn declarations_preserve_order_scope_and_completion_in_both_engines() {
    let cases = [
        ("let a, b; a === undefined && b === undefined ? 1 : 0", 1.0),
        ("var a = 1, b = a + 2, c; b", 3.0),
        ("let a = 2, b = a * 3; b", 6.0),
        ("const a = 3, b = a + 4; b", 7.0),
        ("var a = (1, 2), b = Math.max(a, 4); b", 4.0),
        ("var a = 1; { let a = 2, b = a; } a", 1.0),
        (
            "function f() { return b; var a = 1, b = 2; } f() === undefined ? 1 : 0",
            1.0,
        ),
        (
            "var total = 0; for (let i = 0, j = 3; i < j; i++) { total += i; } total",
            3.0,
        ),
        ("for (var i = 0, j = 2; i < j; i++) {} i + j", 4.0),
        ("42; var a = 1, b = 2;", 42.0),
        ("var a = 0; if (true) var b = 1, c = 2; b + c", 3.0),
        ("var mask = 0xffff, value = 0X2a; value & mask", 42.0),
    ];
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        for (source, expected) in cases {
            assert_eq!(
                eval_with_mode(source, mode).unwrap(),
                Value::Number(expected),
                "{mode:?}: {source}"
            );
        }
        for source in [
            "let a = b, b = 1;",
            "const a = 1, b = 2; b = 3;",
            "let a = 1, a = 2;",
        ] {
            assert!(eval_with_mode(source, mode).is_err(), "{mode:?}: {source}");
        }
    }
}

#[test]
fn declaration_lists_persist_in_a_realm_and_check_conflicts() {
    let realm = Realm::new();
    realm.evaluate_script("let a = 1, b = 2;").unwrap();
    assert_eq!(realm.evaluate_script("a + b").unwrap(), Value::Number(3.0));
    assert!(realm.evaluate_script("var c = 3, b = 4;").is_err());
    assert_eq!(
        realm.evaluate_script("typeof c").unwrap(),
        Value::String("undefined".into())
    );
    assert!(realm.evaluate_script("let d = 1, d = 2;").is_err());
}

#[test]
fn malformed_lists_and_hexadecimal_literals_are_rejected() {
    for source in [
        "var a,;",
        "let a = 1,;",
        "const a = 1, b;",
        "const a, b = 2;",
        "for (const a = 1, b; false;) {}",
        "for (let a, b of []) {}",
        "0x",
        "0X",
        "0xg",
        "0x1g",
        "0x1n",
        "0x1_0",
        r"0x1\u0061",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
}

#[test]
fn hexadecimal_literals_support_case_signs_and_large_values() {
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        for (source, expected) in [
            ("0x0", 0.0),
            ("0XfF", 255.0),
            ("-0x2A", -42.0),
            ("0x100000000", 4294967296.0),
            ("0x10000000000000800", 18446744073709551616.0),
            ("0x10000000000000801", 18446744073709555712.0),
            ("0xffffffffffffffff", 18446744073709551616.0),
            ("0x10000000000000000", 18446744073709551616.0),
            ("0x0000000000000000002a + 0.5", 42.5),
        ] {
            assert_eq!(
                eval_with_mode(source, mode).unwrap(),
                Value::Number(expected),
                "{source}"
            );
        }
        let overflow = format!("0x1{}", "0".repeat(256));
        assert_eq!(
            eval_with_mode(&overflow, mode).unwrap(),
            Value::Number(f64::INFINITY)
        );
        assert!(
            matches!(eval_with_mode("-0x0", mode).unwrap(), Value::Number(n) if n == 0.0 && n.is_sign_negative())
        );
    }
}
