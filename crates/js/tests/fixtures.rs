//! Integration tests for JavaScript fixtures under `crates/js/tests/js`.

use js::{Object, Realm, Value, Vm, eval, parse};

fn execute_bytecode(source: &str) -> Result<Value, js::JsError> {
    let program = parse(source).expect("fixture should parse");
    let chunk = js::bytecode::compile(&program).expect("Sprint 4 fixture should compile");
    Vm::new().execute(&chunk)
}

fn fixture(name: &str) -> &'static str {
    match name {
        "arithmetic.js" => include_str!("js/arithmetic.js"),
        "control-flow.js" => include_str!("js/control-flow.js"),
        "functions.js" => include_str!("js/functions.js"),
        "automatic-semicolon-insertion.js" => include_str!("js/automatic-semicolon-insertion.js"),
        "objects.js" => include_str!("js/objects.js"),
        "comments-and-whitespace.js" => include_str!("js/comments-and-whitespace.js"),
        "invalid-syntax.js" => include_str!("js/invalid-syntax.js"),
        "arrays.js" => include_str!("js/arrays.js"),
        "arrow-functions.js" => include_str!("js/arrow-functions.js"),
        "classes.js" => include_str!("js/classes.js"),
        "for-loop.js" => include_str!("js/for-loop.js"),
        "try-catch.js" => include_str!("js/try-catch.js"),
        other => panic!("unknown JS fixture: {other}"),
    }
}

#[test]
fn arithmetic_fixture_evaluates() {
    assert_eq!(eval(fixture("arithmetic.js")), Ok(Value::Number(15.0)));
}

#[test]
fn control_flow_fixture_evaluates() {
    assert_eq!(eval(fixture("control-flow.js")), Ok(Value::Number(20.0)));
}

#[test]
fn functions_fixture_evaluates() {
    assert_eq!(eval(fixture("functions.js")), Ok(Value::Number(15.0)));
}

#[test]
fn automatic_semicolon_insertion_fixture_evaluates() {
    assert_eq!(
        eval(fixture("automatic-semicolon-insertion.js")),
        Ok(Value::Number(4.0))
    );
}

#[test]
fn object_fixture_evaluates_with_a_registered_object() {
    let target = Object::new();
    let mut realm = Realm::new();
    realm
        .register_global_value("target", Value::Object(target.clone()))
        .expect("fixture global should be valid");

    assert_eq!(
        realm.evaluate_script(fixture("objects.js")),
        Ok(Value::Number(7.0))
    );
}

#[test]
fn comments_and_whitespace_fixture_evaluates() {
    assert_eq!(
        eval(fixture("comments-and-whitespace.js")),
        Ok(Value::Number(42.0))
    );
}

#[test]
fn invalid_syntax_fixture_returns_a_syntax_error() {
    let error = eval(fixture("invalid-syntax.js")).expect_err("fixture should be invalid");
    assert_eq!(error.category, js::JsErrorCategory::Syntax);
}

#[test]
fn arrays_fixture_evaluates() {
    assert_eq!(eval(fixture("arrays.js")), Ok(Value::Number(1.0)));
}

#[test]
fn arrow_functions_fixture_evaluates() {
    assert!(eval(fixture("arrow-functions.js")).is_ok());
}

#[test]
fn sprint_four_fixtures_compile_and_execute_through_the_vm() {
    for (name, expected) in [
        ("arrays.js", Value::Number(1.0)),
        ("arrow-functions.js", Value::Number(3.0)),
        ("for-loop.js", Value::Number(4.0)),
        ("classes.js", Value::Number(1.0)),
    ] {
        assert_eq!(execute_bytecode(fixture(name)), Ok(expected), "{name}");
    }

    let thrown =
        execute_bytecode(fixture("try-catch.js")).expect("try/catch fixture should run in the VM");
    let Value::Object(error) = thrown else {
        panic!("expected the caught error object, got {thrown:?}");
    };
    assert_eq!(error.get("name"), Some(Value::String("Error".into())));
}

#[test]
#[ignore = "Classes run only on the bytecode VM; see tests/bytecode_classes.rs"]
fn classes_fixture_will_evaluate_when_classes_are_supported() {
    assert!(eval(fixture("classes.js")).is_ok());
}

#[test]
fn for_loop_fixture_will_evaluate_when_for_is_supported() {
    assert!(eval(fixture("for-loop.js")).is_ok());
}

#[test]
fn try_catch_fixture_evaluates() {
    assert!(eval(fixture("try-catch.js")).is_ok());
}
