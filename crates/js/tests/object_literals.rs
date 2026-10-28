//! Object literals: `{ a: 1, 'b': 2, 3: x, [key]: y, a }`.

mod common;

use std::sync::{Arc, Mutex};

use common::assert_equivalent;
use js::ast::{Expr, ObjectProperty, PropertyKey};
use js::lexer::{Token, tokenize};
use js::{HostFunction, JsErrorCategory, Realm, Value, eval, parse};

fn expression(source: &str) -> Expr {
    match parse(source).unwrap().body.as_slice() {
        [js::Statement::Expression(expression)] => expression.clone(),
        other => panic!("expected one expression statement, got {other:?}"),
    }
}

fn named(key: &str, value: Expr) -> ObjectProperty {
    ObjectProperty {
        key: PropertyKey::Named(key.into()),
        value,
    }
}

fn object(source: &str) -> js::Object {
    match eval(source) {
        Ok(Value::Object(object)) => object,
        other => panic!("{source} should produce an object, got {other:?}"),
    }
}

// Lexing and parsing --------------------------------------------------------------

#[test]
fn lexes_colon() {
    assert_eq!(
        tokenize("{a:1}"),
        vec![
            Token::LeftBrace,
            Token::Identifier("a".into()),
            Token::Colon,
            Token::Number(1.0),
            Token::RightBrace,
        ]
    );
}

#[test]
fn parses_every_key_form() {
    assert_eq!(
        expression("({ a: 1, 'b c': 2, 1.50: 3, if: 4, [k]: 5, d })"),
        Expr::Object(vec![
            named("a", Expr::Number(1.0)),
            named("b c", Expr::Number(2.0)),
            named("1.5", Expr::Number(3.0)),
            named("if", Expr::Number(4.0)),
            ObjectProperty {
                key: PropertyKey::Computed(Box::new(Expr::Identifier("k".into()))),
                value: Expr::Number(5.0),
            },
            named("d", Expr::Identifier("d".into())),
        ])
    );
}

#[test]
fn parses_empty_and_trailing_comma() {
    assert_eq!(expression("({})"), Expr::Object(vec![]));
    assert_eq!(
        expression("({ a: 1, })"),
        Expr::Object(vec![named("a", Expr::Number(1.0))])
    );
}

#[test]
fn brace_at_statement_start_is_still_a_block() {
    assert_eq!(
        parse("{ 1 }").unwrap().body,
        vec![js::Statement::Block(vec![js::Statement::Expression(
            Expr::Number(1.0)
        )])]
    );
}

#[test]
fn rejects_malformed_literals() {
    for source in [
        "x = { a 1 }",
        "x = { a: }",
        "x = { a: 1 b: 2 }",
        "x = { , }",
        "x = { a: 1,, }",
        "x = { 'a' }",
        "x = { [k] }",
        "x = { a: 1",
    ] {
        let error = js::eval(source).expect_err(source);
        assert_eq!(error.category, JsErrorCategory::Syntax, "{source}: {error}");
    }
}

// Evaluation ----------------------------------------------------------------------

#[test]
fn creates_objects_with_properties() {
    let value = object("let k = 'key'; let d = 4; ({ a: 1, 'b': 'two', 3: true, [k]: null, d })");
    assert_eq!(value.keys(), ["a", "b", "3", "key", "d"]);
    assert_eq!(value.get("a"), Some(Value::Number(1.0)));
    assert_eq!(value.get("b"), Some(Value::String("two".into())));
    assert_eq!(value.get("3"), Some(Value::Boolean(true)));
    assert_eq!(value.get("key"), Some(Value::Null));
    assert_eq!(value.get("d"), Some(Value::Number(4.0)));
}

#[test]
fn literal_properties_are_readable_and_writable() {
    assert_eq!(
        eval("let point = { x: 1, y: 2 }; point.x = point.x + point['y']; point.x"),
        Ok(Value::Number(3.0))
    );
    assert_eq!(eval("({ a: { b: 5 } }).a.b"), Ok(Value::Number(5.0)));
}

#[test]
fn repeated_key_keeps_first_position_and_last_value() {
    let value = object("({ a: 1, b: 2, a: 3 })");
    assert_eq!(value.keys(), ["a", "b"]);
    assert_eq!(value.get("a"), Some(Value::Number(3.0)));
}

#[test]
fn each_evaluation_creates_a_new_object() {
    assert_eq!(
        eval("function make() { return {}; } make() === make()"),
        Ok(Value::Boolean(false))
    );
    assert_eq!(
        eval("let a = {}; let b = a; b.x = 1; a.x"),
        Ok(Value::Number(1.0))
    );
}

#[test]
fn evaluates_keys_and_values_in_source_order() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&log);
    let function: HostFunction = Arc::new(move |arguments: &[Value]| {
        let first = arguments.first().cloned().unwrap_or(Value::Undefined);
        recorded.lock().unwrap().push(first.to_string());
        Ok(first)
    });
    let mut realm = Realm::new();
    realm.register_global_function("log", function).unwrap();
    realm
        .evaluate_script("({ a: log(1), [log('k')]: log(2), b: log(3) })")
        .unwrap();
    assert_eq!(*log.lock().unwrap(), ["1", "k", "2", "3"]);
}

#[test]
fn shorthand_of_undeclared_name_is_a_runtime_error() {
    let error = eval("({ missing })").unwrap_err();
    assert_eq!(error.category, JsErrorCategory::Runtime);
}

#[test]
fn objects_display_as_object_object() {
    assert_eq!(
        eval("'' + { a: 1 }"),
        Ok(Value::String("[object Object]".into()))
    );
}

// Bytecode ------------------------------------------------------------------------

#[test]
fn bytecode_matches_tree_walk_interpreter() {
    for source in [
        "({}).constructor",
        "({ a: 1 }).a",
        "({ a: 1, b: 2, a: 3 }).a",
        "let k = 'x'; ({ [k]: 7 }).x",
        "let k = 1; ({ [k]: 7 })['1']",
        "let v = 2; let o = { v, w: v * 3 }; o.v + o.w",
        "let o = { inner: { n: 4 } }; o.inner.n = 5; o.inner.n",
        "({ 1.50: 'n' })['1.5']",
        "({ a: 1 }) === ({ a: 1 })",
        "let o = {}; let p = o; o === p",
        "'' + { a: 1 }",
        "({ a: missing })",
        "({ [missing]: 1 })",
    ] {
        assert_equivalent(source);
    }
}
