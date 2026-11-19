//! `parseInt`, `parseFloat`, `isNaN`, and `isFinite` as script globals.

use js::{Realm, Value, eval};

fn number(value: Value) -> f64 {
    match value {
        Value::Number(value) => value,
        other => panic!("expected number, got {other:?}"),
    }
}

#[test]
fn parse_int_follows_the_specification_in_a_realm() {
    let realm = Realm::new();
    let run = |script: &str| number(realm.evaluate_script(script).unwrap());
    assert_eq!(run("parseInt('  42px')"), 42.0);
    assert_eq!(run("parseInt('-0x1A')"), -26.0);
    assert_eq!(run("parseInt('ff', 16)"), 255.0);
    assert_eq!(run("parseInt('0x10', 10)"), 0.0);
    assert_eq!(run("parseInt('111', 2)"), 7.0);
    assert_eq!(run("parseInt(3.99)"), 3.0);
    assert_eq!(run("parseInt('12', '8')"), 10.0);
    assert!(run("parseInt('abc')").is_nan());
    assert!(run("parseInt('10', 40)").is_nan());
    assert!(run("parseInt()").is_nan());
}

#[test]
fn parse_float_reads_the_longest_decimal_prefix_in_a_realm() {
    let realm = Realm::new();
    let run = |script: &str| number(realm.evaluate_script(script).unwrap());
    assert_eq!(run("parseFloat('2.75 meters')"), 2.75);
    assert_eq!(run("parseFloat('\\n -.5e2')"), -50.0);
    assert_eq!(run("parseFloat('Infinityx')"), f64::INFINITY);
    assert_eq!(run("parseFloat('0x1F')"), 0.0);
    assert_eq!(run("parseFloat(7)"), 7.0);
    assert!(run("parseFloat('')").is_nan());
    assert!(run("parseFloat('px')").is_nan());
}

#[test]
fn is_nan_and_is_finite_coerce_with_to_number() {
    let realm = Realm::new();
    let run = |script: &str| realm.evaluate_script(script).unwrap();
    for (script, expected) in [
        ("isNaN('abc')", true),
        ("isNaN(undefined)", true),
        ("isNaN()", true),
        ("isNaN(0 / 0)", true),
        ("isNaN('12')", false),
        ("isNaN('')", false),
        ("isNaN(null)", false),
        ("isNaN(true)", false),
        ("isFinite('12')", true),
        ("isFinite(0)", true),
        ("isFinite(null)", true),
        ("isFinite(1 / 0)", false),
        ("isFinite(-1 / 0)", false),
        ("isFinite('abc')", false),
        ("isFinite()", false),
    ] {
        assert_eq!(run(script), Value::Boolean(expected), "{script}");
    }
}

#[test]
fn standalone_vm_resolves_the_numeric_globals() {
    assert_eq!(eval("parseInt('7a', 16)").unwrap(), Value::Number(122.0));
    assert_eq!(eval("parseFloat('2.5kg')").unwrap(), Value::Number(2.5));
    assert_eq!(eval("isNaN('x')").unwrap(), Value::Boolean(true));
    assert_eq!(eval("isFinite('1e3')").unwrap(), Value::Boolean(true));
}

#[test]
fn numeric_globals_are_functions() {
    let realm = Realm::new();
    assert_eq!(
        realm
            .evaluate_script("typeof parseInt + typeof parseFloat + typeof isNaN + typeof isFinite")
            .unwrap(),
        Value::String("functionfunctionfunctionfunction".into())
    );
}
