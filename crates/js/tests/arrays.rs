//! Array indexing, `length`, holes, identity, and host interoperation.
//!
//! Most cases run the same source through the tree-walk evaluator and the
//! bytecode VM and require identical results, since both share one set of
//! array rules. The tests that follow pin down bytecode shape and VM-only
//! behavior.

use std::sync::{Arc, Mutex};

use js::bytecode::{Chunk, Instruction, compile};
use js::runtime::evaluate_program;
use js::{JsError, MAX_ARRAY_LENGTH, Object, Realm, Value, Vm, parse};

fn number(n: f64) -> Value {
    Value::Number(n)
}

fn string(text: &str) -> Value {
    Value::String(text.into())
}

fn compile_source(source: &str) -> Chunk {
    compile(&parse(source).expect("source should parse")).expect("source should compile")
}

fn run_vm(source: &str) -> Result<Value, JsError> {
    Vm::new().execute(&compile_source(source))
}

fn run_tree(source: &str) -> Result<Value, JsError> {
    evaluate_program(&parse(source).expect("source should parse"))
}

/// Runs `source` on both engines and returns the value they agree on.
fn value(source: &str) -> Value {
    let tree = run_tree(source).unwrap_or_else(|error| panic!("tree `{source}`: {error:?}"));
    let vm = run_vm(source).unwrap_or_else(|error| panic!("vm `{source}`: {error:?}"));
    assert_eq!(tree, vm, "engines disagree on `{source}`");
    vm
}

/// Runs `source` on both engines, requires both to fail, and returns the
/// message they agree contains `expected`.
fn fails(source: &str, expected: &str) {
    for (engine, result) in [("tree", run_tree(source)), ("vm", run_vm(source))] {
        let error = result.expect_err(&format!("{engine} `{source}` should fail"));
        assert!(
            error.message.contains(expected),
            "{engine} `{source}`: {error:?}"
        );
    }
}

// Bytecode ------------------------------------------------------------------------------------

#[test]
fn indexing_compiles_object_then_key_then_value() {
    let chunk = compile_source("[][0] = 1");
    assert!(matches!(
        chunk.code.as_slice(),
        [
            Instruction::EnterScope(0),
            Instruction::NewArray(0),
            Instruction::Constant(0),
            Instruction::Constant(1),
            Instruction::StoreComputedProperty,
            Instruction::Return,
        ]
    ));
}

#[test]
fn hand_built_chunk_reads_and_writes_elements() {
    let mut chunk = Chunk::new();
    let zero = chunk.add_constant(number(0.0)).unwrap();
    let seven = chunk.add_constant(number(7.0)).unwrap();
    chunk.emit(Instruction::NewArray(0));
    chunk.emit(Instruction::Dup);
    chunk.emit(Instruction::Constant(zero));
    chunk.emit(Instruction::Constant(seven));
    chunk.emit(Instruction::StoreComputedProperty);
    chunk.emit(Instruction::Pop);
    chunk.emit(Instruction::Constant(zero));
    chunk.emit(Instruction::LoadComputedProperty);
    chunk.emit(Instruction::Return);
    assert_eq!(Vm::new().execute(&chunk), Ok(number(7.0)));
}

#[test]
fn arrays_do_not_convert_to_numbers_on_the_vm() {
    let error = run_vm("[] - 1").expect_err("arrays are not numbers on the VM");
    assert!(error.message.contains("convert"), "{error:?}");
}

#[test]
fn evaluation_order_is_object_then_key_then_value() {
    let log = "let log = ''; function note(x) { log = log + x; return x; } ";
    assert_eq!(
        value(&format!(
            "{log} let a = []; note(a)[note(0)] = note('v'); log"
        )),
        string("0v"),
        "an empty array prints as nothing"
    );
    assert_eq!(
        value(&format!("{log} let a = [5]; note(a)[note(0)]; log")),
        string("50")
    );
    assert_eq!(
        value(&format!("{log} [note('a'), note('b'), note('c')]; log")),
        string("abc")
    );
}

#[test]
fn a_failed_store_happens_after_the_value_is_evaluated() {
    fails(
        "function one() { return 1; } let n = 5; n[0] = one();",
        "a number",
    );
}

// Reads, out-of-range access, and holes -------------------------------------------------------

#[test]
fn literals_index_and_report_length() {
    assert_eq!(value("let a = [10, 20, 30]; a[0]"), number(10.0));
    assert_eq!(value("[10, 20, 30][2]"), number(30.0));
    assert_eq!(value("[10, 20, 30].length"), number(3.0));
    assert_eq!(value("[].length"), number(0.0));
    assert_eq!(value("[1, 2,].length"), number(2.0), "one trailing comma");
}

#[test]
fn out_of_range_reads_are_undefined() {
    for source in ["[1][5]", "[1][-1]", "[1][1.5]", "[][0]", "[1]['x']"] {
        assert_eq!(value(source), Value::Undefined, "{source}");
    }
}

#[test]
fn literal_elisions_count_toward_length_and_read_as_undefined() {
    assert_eq!(value("[1, , 3][1]"), Value::Undefined);
    assert_eq!(value("[1, , 3].length"), number(3.0));
    assert_eq!(value("[,].length"), number(1.0));
    assert_eq!(value("[1,,].length"), number(2.0));
}

#[test]
fn assigning_past_the_end_pads_with_holes() {
    assert_eq!(value("let a = [1]; a[3] = 4; a.length"), number(4.0));
    assert_eq!(value("let a = [1]; a[3] = 4; a[2]"), Value::Undefined);
    assert_eq!(value("let a = [1]; a[3] = 4; a[3]"), number(4.0));
    assert_eq!(value("let a = []; a[0] = 'x'; a.length"), number(1.0));
    assert_eq!(
        value("let a = [1, 2, 3]; a[0] = 9; a.length"),
        number(3.0),
        "overwriting keeps the length"
    );
}

#[test]
fn gaps_left_by_writes_are_real_holes() {
    let array = Object::new();
    array.set("length", number(0.0));
    let mut realm = Realm::new();
    realm
        .register_global_value("list", Value::Array(array.clone()))
        .unwrap();
    realm.evaluate_script("list[3] = 'x';").unwrap();
    assert_eq!(array.get("length"), Some(number(4.0)));
    assert!(!array.contains_key("1"), "index 1 was never assigned");
    assert!(array.contains_key("3"));
}

// Keys ----------------------------------------------------------------------------------------

#[test]
fn numeric_and_string_index_keys_name_the_same_element() {
    assert_eq!(value("let a = [7, 8]; a['1']"), number(8.0));
    assert_eq!(value("let a = [7, 8]; a['1'] = 9; a[1]"), number(9.0));
    assert_eq!(value("let a = [7, 8]; a[-0]"), number(7.0));
    assert_eq!(value("let a = [7, 8]; a[1.0]"), number(8.0));
    assert_eq!(value("let a = [7, 8]; a[true]"), Value::Undefined);
}

#[test]
fn non_index_keys_are_named_properties_outside_length() {
    assert_eq!(
        value("let a = [7]; a['01'] = 1; a['-1'] = 2; a['1.5'] = 3; a.length"),
        number(1.0)
    );
    assert_eq!(value("let a = [7]; a['01'] = 1; a['01']"), number(1.0));
    assert_eq!(value("let a = [7]; a[-1] = 2; a[-1]"), number(2.0));
    assert_eq!(value("let a = [7]; a.label = 'x'; a.label"), string("x"));
    assert_eq!(value("let a = [7]; a.label = 'x'; a.length"), number(1.0));
    assert_eq!(
        value("let a = [7]; a['01'] = 1; a[1]"),
        Value::Undefined,
        "`'01'` is not the index 1"
    );
}

#[test]
fn indexes_at_or_above_two_to_the_32_minus_1_are_named_properties() {
    assert_eq!(
        value("let a = []; a[4294967295] = 1; a.length"),
        number(0.0)
    );
    assert_eq!(
        value("let a = []; a[4294967295] = 1; a[4294967295]"),
        number(1.0)
    );
}

#[test]
fn huge_indexes_and_lengths_are_rejected() {
    let limit = MAX_ARRAY_LENGTH;
    fails(
        &format!("let a = []; a[{limit}] = 1;"),
        "Invalid array length",
    );
    fails(
        &format!("let a = []; a.length = {};", limit + 1),
        "Invalid array length",
    );
    assert_eq!(
        value(&format!("let a = []; a[{}] = 1; a.length", limit - 1)),
        number(limit as f64)
    );
}

// Length --------------------------------------------------------------------------------------

#[test]
fn assigning_length_truncates_and_pads() {
    assert_eq!(
        value("let a = [1, 2, 3]; a.length = 1; a.length"),
        number(1.0)
    );
    assert_eq!(
        value("let a = [1, 2, 3]; a.length = 1; a[1]"),
        Value::Undefined
    );
    assert_eq!(value("let a = [1]; a.length = 3; a.length"), number(3.0));
    assert_eq!(value("let a = [1]; a.length = 3; a[2]"), Value::Undefined);
    assert_eq!(value("let a = [1, 2]; a.length = 0; a.length"), number(0.0));
    assert_eq!(
        value("let a = [1, 2]; a['length'] = 1; a.length"),
        number(1.0)
    );
    assert_eq!(
        value("let a = [1, 2]; a.length = '1'; a.length"),
        number(1.0),
        "length converts with ToNumber"
    );
}

#[test]
fn truncated_elements_do_not_come_back() {
    assert_eq!(
        value("let a = [1, 2, 3]; a.length = 1; a.length = 3; a[2]"),
        Value::Undefined
    );
}

#[test]
fn invalid_lengths_are_errors() {
    for bad in ["-1", "1.5", "'x'", "undefined", "1 / 0"] {
        fails(
            &format!("let a = [1, 2]; a.length = {bad};"),
            "Invalid array length",
        );
    }
}

#[test]
fn a_failed_length_write_leaves_the_array_alone() {
    let array = Object::new();
    array.set("0", number(1.0));
    array.set("length", number(1.0));
    let mut realm = Realm::new();
    realm
        .register_global_value("list", Value::Array(array.clone()))
        .unwrap();
    assert!(realm.evaluate_script("list.length = -1;").is_err());
    assert_eq!(array.get("length"), Some(number(1.0)));
    assert_eq!(array.get("0"), Some(number(1.0)));
}

#[test]
fn push_and_pop_mutate_and_report() {
    assert_eq!(value("let a = [1]; a.push(2, 3)"), number(3.0));
    assert_eq!(value("let a = [1]; a.push(2); a[1]"), number(2.0));
    assert_eq!(value("let a = [1, 2]; a.pop()"), number(2.0));
    assert_eq!(value("let a = [1, 2]; a.pop(); a.length"), number(1.0));
    assert_eq!(value("let a = [1, 2]; a.pop(); a[1]"), Value::Undefined);
    assert_eq!(value("[].pop()"), Value::Undefined);
    assert_eq!(
        value("let a = []; a.push = 1; a.push"),
        number(1.0),
        "a named property wins"
    );
}

// Identity, conversions, and nesting ----------------------------------------------------------

#[test]
fn arrays_have_reference_identity() {
    assert_eq!(value("[] === []"), Value::Boolean(false));
    assert_eq!(value("let a = []; a === a"), Value::Boolean(true));
    assert_eq!(value("let a = []; let b = a; b[0] = 1; a[0]"), number(1.0));
    assert_eq!(
        value("let a = [1]; let b = [1]; a == b"),
        Value::Boolean(false)
    );
}

#[test]
fn each_evaluation_of_a_literal_makes_a_new_array() {
    assert_eq!(
        value("function make() { return [0]; } let a = make(); let b = make(); a[0] = 9; b[0]"),
        number(0.0)
    );
}

#[test]
fn arrays_are_shared_when_passed_to_functions() {
    assert_eq!(
        value(
            "function fill(list) { list[0] = 'x'; list.length = 2; } let a = []; fill(a); a.length"
        ),
        number(2.0)
    );
}

#[test]
fn arrays_are_truthy_and_stringify_with_commas() {
    assert_eq!(value("[] && 1"), number(1.0));
    assert_eq!(value("'' + [1, 2, 3]"), string("1,2,3"));
    assert_eq!(value("'' + [1, , 3]"), string("1,,3"));
    assert_eq!(value("let a = []; a[2] = 3; '' + a"), string(",,3"));
    assert_eq!(value("'' + [[1, 2], [3]]"), string("1,2,3"));
    assert_eq!(value("'' + [null, undefined, 'a']"), string(",,a"));
    assert_eq!(value("let a = [1]; a[1] = a; '' + a"), string("1,"));
}

#[test]
fn nested_arrays_index_and_assign() {
    assert_eq!(value("let m = [[1, 2], [3, 4]]; m[1][0]"), number(3.0));
    assert_eq!(
        value("let m = [[1, 2], [3, 4]]; m[0][1] = 9; m[0][1]"),
        number(9.0)
    );
    assert_eq!(
        value("let m = [[], []]; m[1][2] = 1; m[1].length"),
        number(3.0)
    );
    assert_eq!(value("[[1, [2, [3]]]][0][1][1][0]"), number(3.0));
}

#[test]
fn arrays_hold_functions_and_objects() {
    assert_eq!(
        value("function add(a, b) { return a + b; } let fns = [add]; fns[0](2, 3)"),
        number(5.0)
    );
    assert_eq!(
        value("let list = [{ a: 1 }, { a: 2 }]; list[1].a + list[0].a"),
        number(3.0)
    );
    assert_eq!(value("let o = { items: [4, 5] }; o.items[1]"), number(5.0));
    assert_eq!(
        value("let o = { items: [4, 5] }; o.items[0] = 6; o.items[0]"),
        number(6.0)
    );
    assert_eq!(
        value(
            "let o = { list: [] }; o.list[o.list.length] = 'a'; o.list[o.list.length] = 'b'; o.list.length"
        ),
        number(2.0)
    );
}

#[test]
fn computed_index_expressions_work() {
    assert_eq!(value("let a = [10, 20, 30]; a[1 + 1]"), number(30.0));
    assert_eq!(value("let a = [10, 20, 30]; a[a.length - 1]"), number(30.0));
    assert_eq!(
        value("let a = [0, 0, 0]; let i = 0; while (i < 3) { a[i] = i * i; i = i + 1; } a[2]"),
        number(4.0)
    );
    assert_eq!(value("let a = [5]; a[a[0] - 5]"), number(5.0));
}

#[test]
fn indexing_non_arrays_keeps_existing_errors() {
    fails("let n = 1; n[0]", "a number");
    fails("let u; u[0]", "undefined");
    fails("let o = null; o[0] = 1", "null");
}

#[test]
fn array_methods_and_methods_on_elements_get_this() {
    assert_eq!(
        run_vm(
            "class Box { constructor(n) { this.n = n; } get() { return this.n; } } \
             let boxes = [new Box(1), new Box(2)]; boxes[1].get()"
        ),
        Ok(number(2.0)),
        "method call on an indexed element receives `this`"
    );
}

// Host interoperation -------------------------------------------------------------------------

#[test]
fn host_arrays_are_visible_to_scripts_and_changes_reach_the_host() {
    let list = Object::new();
    list.set("0", number(1.0));
    list.set("1", string("two"));
    list.set("length", number(2.0));
    let mut realm = Realm::new();
    realm
        .register_global_value("list", Value::Array(list.clone()))
        .unwrap();

    assert_eq!(
        realm.evaluate_script("list[1] = 'changed'; list[2] = 'new'; list.length"),
        Ok(number(3.0))
    );
    assert_eq!(list.get("1"), Some(string("changed")));
    assert_eq!(list.get("2"), Some(string("new")));
    assert_eq!(list.get("length"), Some(number(3.0)));
}

#[test]
fn script_arrays_reach_host_functions() {
    let seen = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&seen);
    let mut realm = Realm::new();
    realm
        .register_global_function(
            "collect",
            Arc::new(move |arguments| {
                if let Some(Value::Array(list)) = arguments.first() {
                    *sink.lock().unwrap() = Some(list.clone());
                }
                Ok(Value::Undefined)
            }),
        )
        .unwrap();
    realm
        .evaluate_script("let a = [1, 'x', [2]]; a[4] = 5; collect(a);")
        .unwrap();

    let list = seen.lock().unwrap().take().expect("host received an array");
    assert_eq!(list.get("length"), Some(number(5.0)));
    assert_eq!(list.get("0"), Some(number(1.0)));
    assert_eq!(list.get("1"), Some(string("x")));
    assert!(
        matches!(list.get("2"), Some(Value::Array(inner)) if inner.get("length") == Some(number(1.0)))
    );
    assert!(!list.contains_key("3"), "the gap is a hole");
    assert_eq!(list.get("4"), Some(number(5.0)));
    assert_eq!(Value::Array(list).to_string(), "1,x,2,,5");
}

#[test]
fn arrays_are_compared_by_identity_across_the_host_boundary() {
    let list = Object::new();
    list.set("length", number(0.0));
    let mut realm = Realm::new();
    realm
        .register_global_value("list", Value::Array(list))
        .unwrap();
    assert_eq!(
        realm.evaluate_script("list === list && list !== []"),
        Ok(Value::Boolean(true))
    );
}
