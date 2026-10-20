//! Property access through the bytecode compiler and VM, checked against the
//! tree-walk evaluator.
//!
//! The bytecode compiler cannot lower identifiers yet, so host objects reach
//! compiled code through the constant pool: each case is written once with a
//! `$` placeholder for the object. The VM path replaces `$` with a string
//! literal and patches that first constant with the host object; the
//! evaluator path replaces `$` with a global named `o`.

use std::sync::Arc;

use js::bytecode::{Chunk, Instruction, compile};
use js::{HostFunction, JsError, JsErrorCategory, Object, Realm, Value, Vm, eval, parse};

fn host_method() -> HostFunction {
    Arc::new(|_| Ok(Value::Undefined))
}

/// A fresh object: `a`, `"1"`, a `nested` object with `b`, and a `method`.
fn fixture() -> Object {
    let nested = Object::new();
    nested.set("b", Value::Number(2.0));
    let object = Object::new();
    object.set("a", Value::Number(1.0));
    object.set("1", Value::String("one".into()));
    object.set("nested", Value::Object(nested));
    object.set_method("method", host_method());
    object
}

fn compile_source(source: &str) -> Chunk {
    compile(&parse(source).expect("source should parse")).expect("source should compile")
}

fn run_vm(template: &str, object: &Object) -> Result<Value, JsError> {
    let mut chunk = compile_source(&template.replace('$', "'root'"));
    assert_eq!(chunk.constants[0], Value::String("root".into()));
    chunk.constants[0] = Value::Object(object.clone());
    Vm::new().execute(&chunk)
}

fn run_evaluator(template: &str, object: &Object) -> Result<Value, JsError> {
    let mut realm = Realm::new();
    realm
        .register_global_value("o", Value::Object(object.clone()))
        .unwrap();
    realm.evaluate_script(&template.replace('$', "o"))
}

/// Everything observable about an object: its keys and each property's value,
/// rendered with `Debug` because separate fixtures hold distinct nested
/// objects and functions that never compare equal by identity.
fn snapshot(object: &Object) -> Vec<String> {
    object
        .keys()
        .into_iter()
        .map(|key| format!("{key}: {:?}", object.get(&key).unwrap()))
        .collect()
}

#[test]
fn reads_match_the_evaluator() {
    for template in [
        "$.a",
        "$['a']",
        "$[1]",
        "$['1']",
        "$.nested.b",
        "$['nested']['b']",
        "$['ne' + 'sted'].b",
        "$[0 + 1]",
        "$.nested['b']",
    ] {
        let vm = run_vm(template, &fixture());
        let evaluator = run_evaluator(template, &fixture());
        assert!(vm.is_ok(), "`{template}` should run: {vm:?}");
        assert_eq!(vm, evaluator, "`{template}` differs between paths");
    }
}

#[test]
fn writes_match_the_evaluator() {
    for template in [
        "$.a = 5",
        "$.fresh = 'x'",
        "$[1] = 'uno'",
        "$['k' + 1] = 2",
        "$.nested.b = 9",
        "$.nested.c = 3",
        "$['nested']['b'] = 1 + 2",
    ] {
        let (vm_object, evaluator_object) = (fixture(), fixture());
        let vm = run_vm(template, &vm_object);
        let evaluator = run_evaluator(template, &evaluator_object);
        assert!(vm.is_ok(), "`{template}` should run: {vm:?}");
        assert_eq!(vm, evaluator, "`{template}` result differs");
        assert_eq!(
            snapshot(&vm_object),
            snapshot(&evaluator_object),
            "`{template}` left different properties"
        );
    }
}

#[test]
fn property_errors_match_the_evaluator() {
    for template in [
        "$.missing",
        "$['missing']",
        "$.nested.missing",
        "$.a.b",
        "$.a['b']",
        "$.a.b = 1",
        "$.missing.deeper",
    ] {
        let vm = run_vm(template, &fixture()).expect_err(template);
        let evaluator = run_evaluator(template, &fixture()).expect_err(template);
        assert_eq!(vm.category, JsErrorCategory::Runtime, "`{template}`");
        assert_eq!(vm.category, evaluator.category, "`{template}`");
        assert_eq!(vm.context, evaluator.context, "`{template}`");
    }
}

#[test]
fn missing_and_non_object_errors_describe_the_property() {
    let missing = run_vm("$.missing", &fixture()).unwrap_err();
    assert_eq!(missing.message, "Property `missing` is not defined");
    assert_eq!(missing.context.as_deref(), Some("property access"));

    let read = run_vm("$.a.b", &fixture()).unwrap_err();
    assert_eq!(read.message, "Cannot read property `b` of a number");
    assert_eq!(read.context.as_deref(), Some("property access"));

    let write = run_vm("$.a.b = 1", &fixture()).unwrap_err();
    assert_eq!(write.message, "Cannot set property `b` of a number");
    assert_eq!(write.context.as_deref(), Some("property assignment"));
}

#[test]
fn non_object_targets_match_the_evaluator() {
    for source in [
        "'text'.length",
        "'text'[0]",
        "null.a",
        "undefined.a",
        "true.a",
        "null.a = 1",
        "'text'.length = 1",
        "(1)[0] = 2",
    ] {
        let vm = Vm::new()
            .execute(&compile_source(source))
            .expect_err(source);
        let evaluator = eval(source).expect_err(source);
        assert_eq!(vm, evaluator, "`{source}`");
    }
}

#[test]
fn assignment_evaluates_the_value_before_checking_the_target() {
    // The value expression fails first, so the non-object target is never
    // reported.
    for source in ["1['k'] = -'x'", "null.k = -'x'"] {
        let error = Vm::new().execute(&compile_source(source)).unwrap_err();
        assert_eq!(error.message, "cannot convert string to number", "{source}");
    }
}

#[test]
fn compiled_order_is_object_then_key_then_value() {
    let chunk = compile_source("'o'['k'] = 1");
    assert!(matches!(
        chunk.code.as_slice(),
        [
            Instruction::Constant(0),
            Instruction::Constant(1),
            Instruction::Constant(2),
            Instruction::StoreComputedProperty,
            Instruction::Return,
        ]
    ));

    // The property name sits in the pool before the value; only the
    // instruction order, object then value, is observable.
    let chunk = compile_source("'o'.k = 1");
    assert!(matches!(
        chunk.code.as_slice(),
        [
            Instruction::Constant(0),
            Instruction::Constant(2),
            Instruction::StoreProperty(1),
            Instruction::Return,
        ]
    ));
    assert_eq!(chunk.constants[1], Value::String("k".into()));

    let chunk = compile_source("'o'.a['b']");
    assert!(matches!(
        chunk.code.as_slice(),
        [
            Instruction::Constant(0),
            Instruction::LoadProperty(1),
            Instruction::Constant(2),
            Instruction::LoadComputedProperty,
            Instruction::Return,
        ]
    ));
}

#[test]
fn host_methods_load_as_the_registered_function() {
    let object = fixture();
    let method = object.get("method").unwrap();
    assert!(matches!(method, Value::Function(_)));
    assert_eq!(run_vm("$.method", &object), Ok(method));
}

#[test]
fn stores_are_visible_to_the_host() {
    let object = Object::new();
    assert_eq!(run_vm("$.count = 3", &object), Ok(Value::Number(3.0)));
    assert_eq!(object.get("count"), Some(Value::Number(3.0)));
    assert_eq!(run_vm("$['count'] + 1", &object), Ok(Value::Number(4.0)));
}

#[test]
fn malformed_property_bytecode_is_an_error_not_a_panic() {
    let mut chunk = Chunk::new();
    chunk.constants.push(Value::Object(Object::new()));
    chunk.constants.push(Value::Number(1.0));
    chunk.emit(Instruction::Constant(0));
    chunk.emit(Instruction::LoadProperty(1));
    chunk.emit(Instruction::Return);
    let error = Vm::new().execute(&chunk).unwrap_err();
    assert!(error.message.contains("property name constant"));

    let mut chunk = Chunk::new();
    chunk.emit(Instruction::LoadComputedProperty);
    chunk.emit(Instruction::Return);
    let error = Vm::new().execute(&chunk).unwrap_err();
    assert!(error.message.contains("underflow"));
}
