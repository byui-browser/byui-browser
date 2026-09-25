//! Public-contract tests for `js`.
//!
//! Run ignored tests with `cargo test -p js -- --ignored` to see the backlog.

use js::{Value, eval, lexer, parse, parser, runtime};

#[test]
fn evaluates_arithmetic() {
    let tokens = lexer::tokenize("42 + 10");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Value::Number(52.0));
}

#[test]
fn evaluates_chained_addition() {
    let tokens = lexer::tokenize("42 + 10 + 8");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Value::Number(60.0));
}

#[test]
fn evaluates_other_arithmetic_operators() {
    let tokens = lexer::tokenize("42 - 10 * 2 / 4");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Value::Number(37.0));
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
#[ignore = "TODO(js): string literals not implemented"]
fn evaluates_string_literal() {
    assert_eq!(eval("'hi'"), Ok(Value::String("hi".into())));
}

#[test]
#[ignore = "TODO(js): parser error reporting not implemented"]
fn syntax_error_is_an_err_not_a_panic() {
    assert!(parse("let = ;").is_err());
}
