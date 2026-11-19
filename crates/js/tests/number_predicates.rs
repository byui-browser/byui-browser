//! Non-coercing Number predicates through both execution engines and realms.

use js::{ExecutionMode, Realm, Value, eval_with_mode};

#[test]
fn number_predicates_accept_only_numeric_primitives() {
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        for (source, expected) in [
            ("Number.isNaN(0 / 0)", true),
            ("Number.isNaN(0)", false),
            ("Number.isNaN('abc')", false),
            ("Number.isNaN()", false),
            ("Number.isNaN(undefined)", false),
            ("Number.isNaN(null)", false),
            ("Number.isNaN({})", false),
            ("Number.isNaN(Symbol())", false),
            ("Number.isFinite(42)", true),
            ("Number.isFinite(-0)", true),
            ("Number.isFinite(1 / 0)", false),
            ("Number.isFinite(-1 / 0)", false),
            ("Number.isFinite(0 / 0)", false),
            ("Number.isFinite('42')", false),
            ("Number.isFinite(null)", false),
            ("Number.isFinite(true)", false),
            ("Number.isFinite()", false),
            (
                "function coerce() { throw 'coerced'; } Number.isFinite({ valueOf: coerce })",
                false,
            ),
            ("var check = Number.isFinite; check(12)", true),
        ] {
            assert_eq!(
                eval_with_mode(source, mode).unwrap(),
                Value::Boolean(expected),
                "{mode:?}: {source}"
            );
        }
    }
    let realm = Realm::new();
    realm
        .evaluate_script("var predicate = Number.isNaN")
        .unwrap();
    assert_eq!(
        realm.evaluate_script("predicate(0/0)").unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn safe_integers_include_both_boundaries_without_coercion() {
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        for (source, expected) in [
            ("Number.isSafeInteger(0)", true),
            ("Number.isSafeInteger(-0)", true),
            ("Number.isSafeInteger(42)", true),
            ("Number.isSafeInteger(9007199254740991)", true),
            ("Number.isSafeInteger(-9007199254740991)", true),
            ("Number.isSafeInteger(9007199254740992)", false),
            ("Number.isSafeInteger(-9007199254740992)", false),
            ("Number.isSafeInteger(0.5)", false),
            ("Number.isSafeInteger(1 / 0)", false),
            ("Number.isSafeInteger(0 / 0)", false),
            ("Number.isSafeInteger('42')", false),
            ("Number.isSafeInteger(true)", false),
            ("Number.isSafeInteger(null)", false),
            ("Number.isSafeInteger()", false),
            ("Number.isSafeInteger({})", false),
            ("Number.isSafeInteger(Symbol())", false),
        ] {
            assert_eq!(
                eval_with_mode(source, mode).unwrap(),
                Value::Boolean(expected),
                "{mode:?}: {source}"
            );
        }
    }
    let realm = Realm::new();
    assert_eq!(
        realm.evaluate_script("Number.isSafeInteger(1)").unwrap(),
        Value::Boolean(true)
    );
}
