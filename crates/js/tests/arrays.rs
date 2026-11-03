//! Array literals, indexing, `length`, and identity through the bytecode
//! compiler and VM.
//!
//! Arrays run only on this path: the tree-walk evaluator parses them but
//! reports that they are unsupported, which the last tests pin down.

use std::sync::{Arc, Mutex};

use js::bytecode::{Chunk, Instruction, compile};
use js::{Array, Function, JsError, MAX_ARRAY_LENGTH, Object, Value, Vm, eval, parse};

fn compile_source(source: &str) -> Chunk {
    compile(&parse(source).expect("source should parse")).expect("source should compile")
}

fn run(source: &str) -> Result<Value, JsError> {
    Vm::new().execute(&compile_source(source))
}

fn value(source: &str) -> Value {
    run(source).unwrap_or_else(|error| panic!("`{source}` failed: {error:?}"))
}

fn error(source: &str) -> JsError {
    run(source).expect_err("source should fail at run time")
}

fn number(n: f64) -> Value {
    Value::Number(n)
}

fn string(text: &str) -> Value {
    Value::String(text.into())
}

// Bytecode ------------------------------------------------------------------------------------

#[test]
fn array_literal_compiles_to_new_array_and_appends() {
    let chunk = compile_source("[1, , 2]");
    assert!(matches!(
        chunk.code.as_slice(),
        [
            Instruction::EnterScope(0),
            Instruction::NewArray,
            Instruction::Constant(0),
            Instruction::AppendElement,
            Instruction::AppendHole,
            Instruction::Constant(1),
            Instruction::AppendElement,
            Instruction::Return,
        ]
    ));
}

#[test]
fn indexing_compiles_object_then_key_then_value() {
    let chunk = compile_source("[][0] = 1");
    assert!(matches!(
        chunk.code.as_slice(),
        [
            Instruction::EnterScope(0),
            Instruction::NewArray,
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
    chunk.emit(Instruction::NewArray);
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
fn append_to_a_non_array_is_an_error() {
    let mut chunk = Chunk::new();
    let one = chunk.add_constant(number(1.0)).unwrap();
    chunk.emit(Instruction::NewObject);
    chunk.emit(Instruction::Constant(one));
    chunk.emit(Instruction::AppendElement);
    chunk.emit(Instruction::Return);
    assert!(Vm::new().execute(&chunk).is_err());
}

#[test]
fn disassembly_names_array_instructions() {
    let listing = compile_source("[1]").disassemble();
    assert!(listing.contains("NewArray"), "{listing}");
    assert!(listing.contains("AppendElement"), "{listing}");
}

// Literals and reads --------------------------------------------------------------------------

#[test]
fn literals_index_and_report_length() {
    assert_eq!(value("let a = [10, 20, 30]; a[0]"), number(10.0));
    assert_eq!(value("let a = [10, 20, 30]; a[2]"), number(30.0));
    assert_eq!(value("[10, 20, 30][1]"), number(20.0));
    assert_eq!(value("[10, 20, 30].length"), number(3.0));
    assert_eq!(value("[].length"), number(0.0));
    assert_eq!(value("[1, 2,].length"), number(2.0), "one trailing comma");
}

#[test]
fn elements_are_evaluated_in_source_order() {
    assert_eq!(
        value(
            "let log = ''; \
             function note(x) { log = log + x; return x; } \
             [note('a'), note('b'), note('c')]; log"
        ),
        string("abc")
    );
}

#[test]
fn index_expressions_are_evaluated_object_key_then_value() {
    assert_eq!(
        value(
            "let log = ''; \
             function note(x) { log = log + x; return x; } \
             let a = []; \
             note(a)[note(0)] = note('v'); log"
        ),
        string("0v"),
        "object, then key, then value (an empty array prints as nothing)"
    );
    assert_eq!(
        value(
            "let log = ''; \
             function note(x) { log = log + x; return x; } \
             let a = [5]; \
             note(a)[note(0)]; log"
        ),
        string("50"),
        "reads evaluate the object, then the key"
    );
}

#[test]
fn a_failed_store_happens_after_the_value_is_evaluated() {
    let failure = error("function one() { return 1; } let n = 5; n[0] = one();");
    assert!(failure.message.contains("number"), "{failure:?}");
}

// Out of range, holes, and keys ---------------------------------------------------------------

#[test]
fn out_of_range_reads_are_undefined() {
    assert_eq!(value("[1][5]"), Value::Undefined);
    assert_eq!(value("[1][-1]"), Value::Undefined);
    assert_eq!(value("[1][1.5]"), Value::Undefined);
    assert_eq!(value("[][0]"), Value::Undefined);
    assert_eq!(value("[1]['x']"), Value::Undefined);
}

#[test]
fn holes_read_as_undefined_and_count_toward_length() {
    assert_eq!(value("[1, , 3][1]"), Value::Undefined);
    assert_eq!(value("[1, , 3].length"), number(3.0));
    assert_eq!(value("[,].length"), number(1.0));
    assert_eq!(value("[1,,].length"), number(2.0));
    assert_eq!(value("[, 2][1]"), number(2.0));
}

#[test]
fn assigning_past_the_end_pads_with_holes() {
    assert_eq!(value("let a = [1]; a[3] = 4; a.length"), number(4.0));
    assert_eq!(value("let a = [1]; a[3] = 4; a[2]"), Value::Undefined);
    assert_eq!(value("let a = [1]; a[3] = 4; a[3]"), number(4.0));
    assert_eq!(value("let a = []; a[0] = 'x'; a.length"), number(1.0));
}

#[test]
fn numeric_and_string_index_keys_name_the_same_element() {
    assert_eq!(value("let a = [7, 8]; a['1']"), number(8.0));
    assert_eq!(value("let a = [7, 8]; a['1'] = 9; a[1]"), number(9.0));
    assert_eq!(value("let a = [7, 8]; a[-0]"), number(7.0));
    assert_eq!(value("let a = [7, 8]; a[1.0]"), number(8.0));
    assert_eq!(
        value("let a = [7, 8]; a[true]"),
        Value::Undefined,
        "key `true`"
    );
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
    assert!(
        error(&format!("let a = []; a[{limit}] = 1;"))
            .message
            .contains("Invalid array length")
    );
    assert!(
        error(&format!("let a = []; a.length = {};", limit + 1))
            .message
            .contains("Invalid array length")
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
fn invalid_lengths_are_errors_that_leave_the_array_alone() {
    for bad in ["-1", "1.5", "'x'", "undefined", "1 / 0"] {
        let failure = error(&format!("let a = [1, 2]; a.length = {bad};"));
        assert!(
            failure.message.contains("Invalid array length"),
            "{bad}: {failure:?}"
        );
    }
    assert_eq!(value("let a = [1, 2]; a.length = 1; a.length"), number(1.0));
}

#[test]
fn push_and_pop_mutate_and_report() {
    assert_eq!(value("let a = [1]; a.push(2, 3)"), number(3.0));
    assert_eq!(value("let a = [1]; a.push(2); a[1]"), number(2.0));
    assert_eq!(value("let a = [1, 2]; a.pop()"), number(2.0));
    assert_eq!(value("let a = [1, 2]; a.pop(); a.length"), number(1.0));
    assert_eq!(value("[].pop()"), Value::Undefined);
    assert_eq!(
        value("let a = [1, ,]; a.pop()"),
        Value::Undefined,
        "hole pops as undefined"
    );
    assert_eq!(
        value("let a = []; a.push = 1; a.push"),
        number(1.0),
        "a named property wins"
    );
}

// Identity and sharing ------------------------------------------------------------------------

#[test]
fn arrays_have_reference_identity() {
    assert_eq!(value("[] === []"), Value::Boolean(false));
    assert_eq!(value("let a = []; a === a"), Value::Boolean(true));
    assert_eq!(value("let a = []; let b = a; b[0] = 1; a[0]"), number(1.0));
    assert_eq!(
        value("let a = [1]; let b = [1]; a == b"),
        Value::Boolean(false)
    );
    assert_eq!(value("let a = []; a != a"), Value::Boolean(false));
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
    assert_eq!(value("'' + [[1, 2], [3]]"), string("1,2,3"));
    assert_eq!(value("'' + [null, undefined, 'a']"), string(",,a"));
    assert_eq!(value("let a = [1]; a[1] = a; '' + a"), string("1,"));
}

#[test]
fn arrays_do_not_convert_to_numbers() {
    assert!(error("[] - 1").message.contains("convert"));
}

// Nesting and contents ------------------------------------------------------------------------

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
        value("function two() { return 2; } let fns = [two, 1]; fns[0]()"),
        number(2.0)
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
            "class Box { constructor(n) { this.n = n; } get() { return this.n; } } \
               let boxes = [new Box(1), new Box(2)]; boxes[1].get()"
        ),
        number(2.0),
        "method call on an indexed element receives `this`"
    );
    assert_eq!(
        value(
            "class Box { constructor() { this.list = []; } add(x) { this.list[this.list.length] = x; } } \
               let b = new Box(); b.add('a'); b.add('b'); b.list.length"
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
    assert!(error("let n = 1; n[0]").message.contains("a number"));
    assert!(error("let u; u[0]").message.contains("undefined"));
    assert!(error("let o = null; o[0] = 1").message.contains("null"));
}

// Host interoperation -------------------------------------------------------------------------

#[test]
fn host_arrays_are_visible_to_scripts_and_scripts_changes_are_visible_to_the_host() {
    let host = Array::from_values([number(1.0), string("two")]).unwrap();
    let mut chunk = compile_source("'input'[1] = 'changed'; 'input'.length");
    assert_eq!(chunk.constants[0], string("input"));
    chunk.constants[0] = Value::Array(host.clone());
    // The second `'input'` constant is separate; point it at the same array.
    for constant in chunk.constants.iter_mut() {
        if *constant == string("input") {
            *constant = Value::Array(host.clone());
        }
    }
    assert_eq!(Vm::new().execute(&chunk), Ok(number(2.0)));
    assert_eq!(host.get(1), Some(string("changed")));
    assert_eq!(host.len(), 2);
}

#[test]
fn script_arrays_reach_host_functions() {
    let seen = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&seen);
    let host = Function::from_host(
        "collect",
        Arc::new(move |arguments| {
            if let Some(Value::Array(list)) = arguments.first() {
                *sink.lock().unwrap() = Some(list.clone());
            }
            Ok(Value::Undefined)
        }),
    );
    let mut chunk = compile_source("'collect'([1, , 'x', [2]]);");
    chunk.constants[0] = Value::Function(host);
    Vm::new().execute(&chunk).unwrap();

    let list = seen.lock().unwrap().take().expect("host received an array");
    assert_eq!(list.len(), 4);
    assert_eq!(list.get(0), Some(number(1.0)));
    assert!(!list.has(1), "the hole stays a hole");
    assert_eq!(list.get(2), Some(string("x")));
    assert!(matches!(list.get(3), Some(Value::Array(inner)) if inner.len() == 1));
    assert_eq!(
        list.to_vec()[1],
        Value::Undefined,
        "holes are undefined in the flat view"
    );
}

#[test]
fn host_array_api_matches_script_semantics() {
    let array = Array::new();
    assert!(array.is_empty());
    array.set(2, number(1.0)).unwrap();
    assert_eq!(array.len(), 3);
    assert!(!array.has(0));
    assert_eq!(array.get(5), None);
    assert_eq!(array.push(number(2.0)).unwrap(), 4);
    array.set_len(1).unwrap();
    assert_eq!(array.len(), 1);
    assert!(array.set(MAX_ARRAY_LENGTH, Value::Null).is_err());
    assert!(array.set_len(MAX_ARRAY_LENGTH + 1).is_err());
    assert_eq!(array.len(), 1, "failed writes change nothing");
    assert_eq!(array.get_property("length"), number(1.0));
    array.set_property("3", number(9.0)).unwrap();
    assert_eq!(array.get_property("3"), number(9.0));
    assert_eq!(array.to_string(), ",,,9");

    let clone = array.clone();
    assert_eq!(array, clone);
    assert_ne!(array, Array::new());
    assert_eq!(Value::Array(array), Value::Array(clone));
}

#[test]
fn objects_can_hold_arrays_for_the_host() {
    let object = Object::new();
    object.set(
        "list",
        Value::Array(Array::from_values([number(1.0)]).unwrap()),
    );
    let mut chunk = compile_source("'o'.list[0] = 5; 'o'.list.length");
    chunk.constants[0] = Value::Object(object.clone());
    for constant in chunk.constants.iter_mut() {
        if *constant == string("o") {
            *constant = Value::Object(object.clone());
        }
    }
    assert_eq!(Vm::new().execute(&chunk), Ok(number(1.0)));
    let Some(Value::Array(list)) = object.get("list") else {
        panic!("list should still be an array");
    };
    assert_eq!(list.get(0), Some(number(5.0)));
}

// Tree-walk evaluator -------------------------------------------------------------------------

#[test]
fn array_literals_parse_and_are_vm_only() {
    let failure = eval("[1, 2]").expect_err("the evaluator does not run arrays");
    assert!(
        failure
            .message
            .contains("only supported by the bytecode compiler"),
        "{failure:?}"
    );
}

#[test]
fn array_literal_syntax_errors() {
    for source in ["[1 2]", "[1", "[", "[1, 2"] {
        assert!(parse(source).is_err(), "{source}");
    }
}
