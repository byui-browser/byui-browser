//! Public-contract tests for `js`.
//!
//! Run ignored tests with `cargo test -p js -- --ignored` to see the backlog.

use js::{Value, eval, parse};

#[test]
#[ignore = "TODO(js): arithmetic not implemented"]
fn evaluates_arithmetic() {
    assert_eq!(eval("1 + 2"), Ok(Value::Number(3.0)));
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
