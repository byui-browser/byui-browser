//! Public-contract tests for `js`.
//!
//! Covers the source-level `parse`/`eval` entry points and the token-level
//! parser and runtime helpers.

use std::sync::Arc;

use js::{HostFunction, JsError, Realm, Value, eval, lexer, parse, parser, runtime};

#[test]
fn evaluates_arithmetic() {
    let tokens = lexer::tokenize("42 + 10");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Ok(Value::Number(52.0)));
}

#[test]
fn evaluates_chained_addition() {
    let tokens = lexer::tokenize("42 + 10 + 8");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Ok(Value::Number(60.0)));
}

#[test]
fn evaluates_other_arithmetic_operators() {
    let tokens = lexer::tokenize("42 - 10 * 2 / 4");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Ok(Value::Number(37.0)));
}

#[test]
fn empty_source_parses_to_empty_program() {
    assert_eq!(parse(""), Ok(js::Program::default()));
    assert_eq!(parse("  \n\t"), Ok(js::Program::default()));
}

#[test]
fn empty_program_evaluates_to_undefined() {
    assert_eq!(eval(""), Ok(Value::Undefined));
}

#[test]
fn evaluates_string_literal() {
    assert_eq!(eval("'hi'"), Ok(Value::String("hi".into())));
}

#[test]
fn syntax_error_is_an_err_not_a_panic() {
    assert!(parse("let = ;").is_err());
}

#[test]
fn evaluates_declaration_read_and_assignment_from_source() {
    assert_eq!(
        eval("let count = 1; count = count + 2; count"),
        Ok(Value::Number(3.0))
    );
}

#[test]
fn nested_blocks_shadow_without_mutating_the_outer_binding() {
    assert_eq!(
        eval("let value = 1; { let value = 2; value = 3; } value"),
        Ok(Value::Number(1.0))
    );
}

#[test]
fn invalid_bindings_return_errors() {
    assert!(eval("missing").is_err());
    assert!(eval("const answer = 42; answer = 7").is_err());
    assert!(eval("let answer = 1; let answer = 2").is_err());
}

#[test]
fn runtime_errors_are_returned_not_panics() {
    for source in ["f();", "let x = 1; x();", "undefined()", "(1 + 2)()"] {
        assert!(eval(source).is_err(), "{source}");
    }
}

#[test]
fn evaluate_reports_errors_instead_of_hiding_them() {
    let tokens = lexer::tokenize("missing + 1");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert!(runtime::evaluate(&ast).is_err());
}

#[test]
fn program_parser_preserves_runtime_short_circuit_behavior() {
    assert_eq!(eval("false && missing"), Ok(Value::Boolean(false)));
    assert_eq!(eval("true || missing"), Ok(Value::Boolean(true)));
    assert_eq!(eval("true && 2"), Ok(Value::Number(2.0)));
    assert_eq!(eval("false || 3"), Ok(Value::Number(3.0)));
}

#[test]
fn realm_evaluates_call_arguments_before_invoking_host_function() {
    let mut realm = Realm::new();
    let function: HostFunction = Arc::new(|arguments| Ok(arguments[0].clone()));
    realm.register_global_function("capture", function).unwrap();

    assert_eq!(
        realm.evaluate_script("capture(1 + 2)"),
        Ok(Value::Number(3.0))
    );
}

#[test]
fn realm_returns_parser_and_host_errors_as_js_errors() {
    let realm = Realm::new();
    assert!(realm.evaluate_script("missing()").is_err());

    let mut realm = Realm::new();
    let function: HostFunction = Arc::new(|_| Err(JsError::new("host failed")));
    realm.register_global_function("fail", function).unwrap();

    assert_eq!(
        realm.evaluate_script("fail()"),
        Err(JsError::new("host failed"))
    );
}
