//! Integration tests for JavaScript fixtures under `crates/js/tests/js`.

use js::{Object, Realm, Value, eval};

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
#[ignore = "Array literals and indexing are not implemented yet"]
fn arrays_fixture_will_evaluate_when_arrays_are_supported() {
    assert!(eval(fixture("arrays.js")).is_ok());
}

#[test]
#[ignore = "Arrow functions are not implemented yet"]
fn arrow_functions_fixture_will_evaluate_when_arrows_are_supported() {
    assert!(eval(fixture("arrow-functions.js")).is_ok());
}

#[test]
#[ignore = "Classes and new are not implemented yet"]
fn classes_fixture_will_evaluate_when_classes_are_supported() {
    assert!(eval(fixture("classes.js")).is_ok());
}

#[test]
#[ignore = "For statements are not implemented yet"]
fn for_loop_fixture_will_evaluate_when_for_is_supported() {
    assert!(eval(fixture("for-loop.js")).is_ok());
}

#[test]
#[ignore = "Exception handling is not implemented yet"]
fn try_catch_fixture_will_evaluate_when_exceptions_are_supported() {
    assert!(eval(fixture("try-catch.js")).is_ok());
}
