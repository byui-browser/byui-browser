//! String and Number primitive property lookup through their prototypes.

use js::bytecode::compile;
use js::{ExecutionMode, JsError, Value, eval, eval_with_mode, parse};

fn run_tree(source: &str) -> Result<Value, JsError> {
    eval(source)
}

fn run_vm(source: &str) -> Result<Value, JsError> {
    eval_with_mode(source, ExecutionMode::Bytecode)
}

fn value(source: &str) -> Value {
    let tree = run_tree(source).unwrap_or_else(|error| panic!("tree `{source}`: {error:?}"));
    let vm = run_vm(source).unwrap_or_else(|error| panic!("vm `{source}`: {error:?}"));
    assert_eq!(tree, vm, "engines disagree on `{source}`");
    vm
}

fn number(n: f64) -> Value {
    Value::Number(n)
}

fn string(text: &str) -> Value {
    Value::String(text.into())
}

#[test]
fn string_length_and_indices() {
    assert_eq!(value("'hi'.length"), number(2.0));
    assert_eq!(value("'hi'[0]"), string("h"));
    assert_eq!(value("'hi'[1]"), string("i"));
    assert_eq!(value("'hi'[2]"), Value::Undefined);
    assert_eq!(value("'😀'.length"), number(1.0));
}

#[test]
fn common_string_methods() {
    assert_eq!(value("'hi'.charAt(1)"), string("i"));
    assert_eq!(value("'abc'.slice(1, 3)"), string("bc"));
    assert_eq!(value("'Hello'.toLowerCase()"), string("hello"));
    assert_eq!(value("'  x  '.trim()"), string("x"));
    assert_eq!(value("'ab'.concat('c', 'd')"), string("abcd"));
    assert_eq!(value("'ab'.indexOf('b')"), number(1.0));
    assert_eq!(value("'abab'.lastIndexOf('ab')"), number(2.0));
    assert_eq!(value("'hello'.includes('ell')"), Value::Boolean(true));
    assert_eq!(value("'hello'.startsWith('he')"), Value::Boolean(true));
    assert_eq!(value("'hello'.endsWith('lo')"), Value::Boolean(true));
    assert_eq!(value("'na'.repeat(2)"), string("nana"));
    assert_eq!(value("'a-b-c'.replace('-', '+')"), string("a+b-c"));
}

#[test]
fn string_split_builds_an_array() {
    for run in [run_tree, run_vm] {
        let Value::Array(parts) = run("'a,b,c'.split(',')").expect("split") else {
            panic!("split should return an array");
        };
        assert_eq!(parts.get("length"), Some(number(3.0)));
        assert_eq!(parts.get("0"), Some(string("a")));
        assert_eq!(parts.get("1"), Some(string("b")));
        assert_eq!(parts.get("2"), Some(string("c")));
    }
}

#[test]
fn number_to_fixed_and_conversions() {
    assert_eq!(value("(1.2345).toFixed(2)"), string("1.23"));
    assert_eq!(value("(12).toString()"), string("12"));
    assert_eq!(value("Number('3')"), number(3.0));
    assert_eq!(value("String(9)"), string("9"));
}

#[test]
fn methods_are_shared_on_the_prototype() {
    assert_eq!(
        value("'a'.charAt === String.prototype.charAt"),
        Value::Boolean(true)
    );
    assert_eq!(
        value("(1).toFixed === Number.prototype.toFixed"),
        Value::Boolean(true)
    );
}

#[test]
fn user_properties_on_the_prototype_are_visible_to_primitives() {
    // Function expressions are not in the supported subset yet, so install a
    // data property to prove prototype lookup for script-defined names.
    assert_eq!(
        value("String.prototype.tag = 'marked'; 'hi'.tag"),
        string("marked")
    );
    assert_eq!(value("Number.prototype.tag = 7; (1).tag"), number(7.0));
}

#[test]
fn string_and_number_compile_in_bytecode_mode() {
    let program = parse("'xy'.toUpperCase() + (3.14159).toFixed(2)").unwrap();
    compile(&program).expect("should compile");
    assert_eq!(
        value("'xy'.toUpperCase() + (3.14159).toFixed(2)"),
        string("XY3.14")
    );
}
