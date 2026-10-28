//! Classes, constructors, `this`, `new`, and prototype lookup through the
//! bytecode compiler and VM.
//!
//! Classes run only on this path: the tree-walk evaluator parses them but
//! reports that they are unsupported, which the last tests pin down.

use js::bytecode::{Chunk, Instruction, compile};
use js::runtime::ExecutionLimits;
use js::{Function, JsError, JsErrorCategory, Value, Vm, eval, parse};
use std::sync::Arc;

fn run(source: &str) -> Result<Value, JsError> {
    let program = parse(source).expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    Vm::new().execute(&chunk)
}

fn value(source: &str) -> Value {
    run(source).unwrap_or_else(|error| panic!("`{source}` failed: {error:?}"))
}

fn error(source: &str) -> JsError {
    run(source).expect_err("source should fail at run time")
}

fn string(text: &str) -> Value {
    Value::String(text.into())
}

fn compile_error(source: &str) -> String {
    let program = parse(source).expect("source should parse");
    compile(&program)
        .expect_err("source should not compile")
        .message
}

// Functions and calls -------------------------------------------------------------------------

#[test]
fn functions_are_hoisted_and_return_values() {
    assert_eq!(
        value("twice(4); function twice(n) { return n * 2; }"),
        Value::Number(8.0)
    );
    assert_eq!(
        value("function none() { } none()"),
        Value::Undefined,
        "falling off the end returns undefined"
    );
    assert_eq!(
        value("function pair(a, b) { return b; } pair(1)"),
        Value::Undefined,
        "missing arguments are undefined"
    );
    assert_eq!(
        value("function first(a) { return a; } first(7, 8, 9)"),
        Value::Number(7.0),
        "extra arguments are ignored"
    );
}

#[test]
fn recursion_and_early_return_work() {
    assert_eq!(
        value(
            "function fib(n) { if (n < 2) { return n; } return fib(n - 1) + fib(n - 2); } fib(10)"
        ),
        Value::Number(55.0)
    );
    assert_eq!(
        value(
            "function count(n) { var total = 0; while (true) { if (n == 0) { return total; } \
             total = total + n; n = n - 1; } } count(4)"
        ),
        Value::Number(10.0),
        "return leaves a loop and its scopes"
    );
}

#[test]
fn vars_declared_in_nested_blocks_belong_to_the_function() {
    let source = "
        function f(n) {
            var total = 0;
            while (n > 0) {
                var step = n;
                { var inner = 1; total = total + step + inner; }
                n = n - 1;
            }
            return total + step + inner;
        }
        f(3)";
    // (3+1) + (2+1) + (1+1) = 9, then `step` and `inner` are still visible: 1 + 1.
    assert_eq!(value(source), Value::Number(11.0));
}

#[test]
fn functions_read_and_write_top_level_bindings() {
    assert_eq!(
        value("let total = 0; function add(n) { total = total + n; } add(2); add(3); total"),
        Value::Number(5.0)
    );
}

#[test]
fn runaway_recursion_hits_the_call_depth_limit() {
    let error = error("function loop() { return loop(); } loop()");
    assert_eq!(error.category, JsErrorCategory::Limit);
    assert_eq!(error.message, "Maximum call stack size exceeded");

    let program = parse("function f(n) { return n == 0 || f(n - 1); } f(5)").unwrap();
    let chunk = compile(&program).unwrap();
    let limits = ExecutionLimits {
        call_depth_limit: 3,
        ..ExecutionLimits::default()
    };
    let error = Vm::new().execute_with_limits(&chunk, limits).unwrap_err();
    assert_eq!(error.category, JsErrorCategory::Limit);
}

#[test]
fn calling_a_non_function_is_an_error() {
    assert_eq!(error("let x = 1; x()").message, "`1` is not a function");
}

// Classes and receivers -----------------------------------------------------------------------

#[test]
fn counter_fixture_runs_through_bytecode() {
    let source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/js/classes.js"))
            .expect("fixture should be readable");
    assert_eq!(value(&source), Value::Number(1.0));
}

#[test]
fn methods_receive_their_instance_as_this() {
    let source = "
        class Counter {
            constructor(start) { this.value = start; }
            increment() { this.value = this.value + 1; return this.value; }
        }
        let a = new Counter(1);
        let b = new Counter(10);
        a.increment(); a.increment(); b.increment();
        a.value * 100 + b.value";
    assert_eq!(value(source), Value::Number(311.0));
}

#[test]
fn computed_method_calls_also_pass_the_receiver() {
    let source = "
        class Box { constructor() { this.n = 4; } get() { return this.n; } }
        const box = new Box();
        box['get']()";
    assert_eq!(value(source), Value::Number(4.0));
}

#[test]
fn a_detached_method_has_no_receiver() {
    let source = "
        class Box { constructor() { this.n = 4; } get() { return this.n; } }
        const get = new Box().get;
        get()";
    assert_eq!(
        error(source).message,
        "Cannot read property `n` of undefined"
    );
}

#[test]
fn this_is_undefined_in_plain_calls_and_at_the_top_level() {
    assert_eq!(value("this"), Value::Undefined);
    assert_eq!(
        value("function who() { return this; } who()"),
        Value::Undefined
    );
}

#[test]
fn methods_can_call_each_other_and_read_arguments() {
    let source = "
        class Math2 {
            double(n) { return n * 2; }
            quad(n) { return this.double(this.double(n)); }
        }
        new Math2().quad(3)";
    assert_eq!(value(source), Value::Number(12.0));
}

#[test]
fn constructors_get_arguments_and_missing_ones_are_undefined() {
    let source = "
        class Pair { constructor(a, b) { this.a = a; this.b = b; } }
        const p = new Pair(1);
        p.a";
    assert_eq!(value(source), Value::Number(1.0));
    let source = "
        class Pair { constructor(a, b) { this.a = a; this.b = b; } }
        new Pair(1).b";
    assert_eq!(value(source), Value::Undefined);
}

#[test]
fn a_class_without_a_constructor_makes_an_empty_instance() {
    let source = "class Empty { hello() { return 'hi'; } } new Empty().hello()";
    assert_eq!(value(source), string("hi"));
}

#[test]
fn new_without_arguments_and_with_member_callees() {
    assert_eq!(
        value("class A { constructor() { this.v = 3; } } new A().v"),
        Value::Number(3.0)
    );
    assert_eq!(
        value("class A { constructor() { this.v = 3; } } new A.prototype.constructor().v"),
        Value::Number(3.0),
        "the callee of `new` is the whole member chain"
    );
}

// Identity and prototypes ---------------------------------------------------------------------

#[test]
fn every_construction_makes_a_distinct_instance() {
    assert_eq!(
        value("class A {} new A() === new A()"),
        Value::Boolean(false)
    );
    assert_eq!(
        value("class A {} const a = new A(); const b = a; a === b"),
        Value::Boolean(true)
    );
}

#[test]
fn instances_link_back_to_their_class() {
    assert_eq!(
        value("class A {} const a = new A(); a.constructor === A"),
        Value::Boolean(true)
    );
    assert_eq!(
        value("class A {} A.prototype.constructor === A"),
        Value::Boolean(true)
    );
}

#[test]
fn instance_properties_shadow_prototype_methods_without_changing_them() {
    let source = "
        class A { who() { return 'proto'; } }
        function own() { return 'own'; }
        const a = new A();
        const b = new A();
        a.who = own;
        a.who() + '/' + b.who()";
    assert_eq!(value(source), string("own/proto"));
}

#[test]
fn prototype_changes_are_visible_to_existing_instances() {
    let source = "
        class A {}
        function late() { return 'late'; }
        const a = new A();
        A.prototype.late = late;
        a.late()";
    assert_eq!(value(source), string("late"));
}

#[test]
fn reading_a_missing_property_is_undefined_like_other_objects() {
    assert_eq!(value("class A {} new A().nope"), Value::Undefined);
}

#[test]
fn plain_functions_work_as_constructors_with_a_prototype() {
    let source = "
        function Point(x, y) { this.x = x; this.y = y; }
        function sum() { return this.x + this.y; }
        Point.prototype.sum = sum;
        new Point(2, 3).sum()";
    assert_eq!(value(source), Value::Number(5.0));
}

#[test]
fn replacing_a_prototype_changes_later_instances_only() {
    let source = "
        class Holder { n() { return 2; } }
        function Thing() { }
        function one() { return 1; }
        Thing.prototype.n = one;
        const before = new Thing();
        Thing.prototype = new Holder();
        before.n() + new Thing().n()";
    assert_eq!(value(source), Value::Number(3.0));
}

// Inheritance ---------------------------------------------------------------------------------

const ANIMALS: &str = "
    class Animal {
        constructor(name) { this.name = name; }
        speak() { return this.name + ' makes a sound'; }
        kind() { return 'animal'; }
    }
    class Dog extends Animal {
        constructor(name) { super(name); this.tricks = 0; }
        speak() { return super.speak() + ' (woof)'; }
    }
";

#[test]
fn derived_classes_inherit_and_override_methods() {
    let source = format!("{ANIMALS} const d = new Dog('Rex'); d.speak() + '|' + d.kind()");
    assert_eq!(value(&source), string("Rex makes a sound (woof)|animal"));
}

#[test]
fn super_calls_run_the_parent_constructor_on_the_same_instance() {
    let source = format!("{ANIMALS} const d = new Dog('Rex'); d.name + d.tricks");
    assert_eq!(value(&source), string("Rex0"));
}

#[test]
fn an_implicit_derived_constructor_forwards_its_arguments() {
    let source = format!("{ANIMALS} class Puppy extends Dog {{ }} new Puppy('Bit').speak()");
    assert_eq!(value(&source), string("Bit makes a sound (woof)"));
}

#[test]
fn deep_chains_use_the_new_target_prototype() {
    let source = "
        class A { constructor() { this.a = 1; } who() { return 'A'; } }
        class B extends A { constructor() { super(); this.b = 2; } who() { return super.who() + 'B'; } }
        class C extends B { constructor() { super(); this.c = 3; } who() { return super.who() + 'C'; } }
        const c = new C();
        c.who() + (c.a + c.b + c.c) + (c.constructor === C)";
    assert_eq!(value(source), string("ABC6true"));
}

#[test]
fn super_methods_receive_the_current_instance() {
    let source = "
        class Base { tag() { return this.id; } }
        class Child extends Base {
            constructor() { super(); this.id = 'child'; }
            tag() { return super.tag(); }
        }
        new Child().tag()";
    assert_eq!(value(source), string("child"));
}

#[test]
fn classes_can_extend_plain_constructor_functions() {
    let source = "
        function Shape(name) { this.name = name; }
        function describe() { return 'shape ' + this.name; }
        Shape.prototype.describe = describe;
        class Circle extends Shape { constructor() { super('circle'); } }
        new Circle().describe()";
    assert_eq!(value(source), string("shape circle"));
}

#[test]
fn extending_a_non_constructor_is_an_error() {
    assert_eq!(
        error("let x = 1; class A extends x {}").message,
        "Class extends value `1` is not a constructor"
    );
    assert_eq!(
        error("class A { m() { } } class B extends A.prototype.m {}").message,
        "Class extends value `function m() { [code] }` is not a constructor",
        "methods are not constructors"
    );
}

// Constructor return rules --------------------------------------------------------------------

#[test]
fn a_constructor_returning_an_object_replaces_the_instance() {
    let source = "
        class Other { constructor() { this.tag = 'other'; } }
        const shared = new Other();
        class A { constructor() { return shared; } }
        new A() === shared";
    assert_eq!(value(source), Value::Boolean(true));
}

#[test]
fn a_constructor_returning_a_primitive_keeps_the_instance() {
    let source = "class A { constructor() { this.v = 1; return 5; } } new A().v";
    assert_eq!(value(source), Value::Number(1.0));
    let source = "class A { constructor() { this.v = 1; return; } } new A().v";
    assert_eq!(value(source), Value::Number(1.0));
}

#[test]
fn a_derived_constructor_may_return_an_object() {
    let source = "
        class Other { }
        const shared = new Other();
        class P { constructor() { this.v = 1; } }
        class D extends P { constructor() { super(); return shared; } }
        new D() === shared";
    assert_eq!(value(source), Value::Boolean(true));
}

#[test]
fn a_derived_constructor_may_not_return_a_primitive() {
    let source = "class P { } class D extends P { constructor() { super(); return 7; } } new D()";
    assert_eq!(
        error(source).message,
        "Derived constructors may only return an object or undefined"
    );
}

#[test]
fn a_derived_constructor_must_call_super() {
    let source = "class P { } class D extends P { constructor() { } } new D()";
    assert_eq!(
        error(source).message,
        "Must call super constructor in derived class before returning from derived constructor"
    );
}

#[test]
fn this_is_unavailable_before_super_returns() {
    let source = "
        class P { }
        class D extends P { constructor() { this.x = 1; super(); } }
        new D()";
    assert_eq!(
        error(source).message,
        "Must call super constructor in derived class before accessing `this`"
    );
}

#[test]
fn super_may_only_be_called_once() {
    let source = "
        class P { }
        class D extends P { constructor() { super(); super(); } }
        new D()";
    assert_eq!(
        error(source).message,
        "Super constructor may only be called once"
    );
}

// Construction errors -------------------------------------------------------------------------

#[test]
fn classes_cannot_be_called_without_new() {
    assert_eq!(
        error("class A { } A()").message,
        "Class constructor `A` cannot be invoked without `new`"
    );
    assert_eq!(
        error("class P { } class D extends P { } D()").message,
        "Class constructor `D` cannot be invoked without `new`"
    );
}

#[test]
fn only_constructors_can_be_constructed() {
    assert_eq!(error("new 1()").message, "`1` is not a constructor");
    assert_eq!(
        error("class A { m() { } } new A.prototype.m()").message,
        "`function m() { [code] }` is not a constructor"
    );
}

#[test]
fn class_bindings_are_unusable_before_their_declaration_runs() {
    assert_eq!(
        error("new A(); class A { }").message,
        "Cannot access `A` before initialization"
    );
    assert_eq!(
        error("class A extends A { }").message,
        "Cannot access `A` before initialization"
    );
}

#[test]
fn classes_declared_in_blocks_are_block_scoped() {
    assert_eq!(
        value("{ class A { v() { return 1; } } new A().v(); }"),
        Value::Number(1.0)
    );
    assert_eq!(
        error("{ class A { } } new A()").message,
        "`A` is not defined"
    );
}

// Compile-time restrictions -------------------------------------------------------------------

#[test]
fn super_calls_are_limited_to_derived_constructors() {
    assert_eq!(
        compile_error("class A { constructor() { super(); } }"),
        "`super(...)` is only valid in a derived class constructor"
    );
    assert_eq!(
        compile_error("class P { } class D extends P { m() { super(); } }"),
        "`super(...)` is only valid in a derived class constructor"
    );
}

#[test]
fn functions_cannot_capture_enclosing_locals() {
    let message = compile_error(
        "function outer() { let x = 1; class A { m() { return x; } } return 1; } outer()",
    );
    assert!(
        message.contains("capture the enclosing local `x`"),
        "{message}"
    );
    let message = compile_error("{ let y = 1; class A { m() { y = 2; } } }");
    assert!(
        message.contains("capture the enclosing local `y`"),
        "{message}"
    );
}

#[test]
fn nested_function_declarations_are_not_compiled_yet() {
    let message = compile_error("{ function inner() { } }");
    assert!(message.contains("function declarations outside the top level"));
}

// Parser --------------------------------------------------------------------------------------

#[test]
fn class_syntax_errors_are_reported() {
    for (source, expected) in [
        (
            "class A { constructor() { } constructor() { } }",
            "A class may only have one constructor",
        ),
        (
            "class A { static m() { } }",
            "static members, accessors, and fields are unsupported",
        ),
        (
            "class A { x = 1; }",
            "static members, accessors, and fields are unsupported",
        ),
        ("super.x", "`super` is only valid inside a class method"),
        (
            "function f() { super(); }",
            "`super` is only valid inside a class method",
        ),
        (
            "class A { m() { super; } }",
            "Expected `(` or `.` after `super`",
        ),
        ("class { }", "Expected an identifier"),
    ] {
        let error = parse(source).expect_err(source);
        assert!(
            error.message.contains(expected),
            "{source}: {}",
            error.message
        );
    }
}

#[test]
fn super_is_not_available_in_functions_nested_in_methods() {
    let source = "class P { } class D extends P { m() { function inner() { super.x; } } }";
    assert!(parse(source).is_err());
}

#[test]
fn keywords_still_work_as_property_names() {
    let source = "class A { new() { return 1; } class() { return 2; } this() { return 3; } }
        const a = new A(); a.new() + a.class() + a.this()";
    assert_eq!(value(source), Value::Number(6.0));
}

#[test]
fn stray_semicolons_between_members_are_allowed() {
    assert_eq!(
        value("class A { ; m() { return 1; }; ; } new A().m()"),
        Value::Number(1.0)
    );
}

// Tree-walk evaluator (documented limitation) -------------------------------------------------

#[test]
fn the_tree_walk_evaluator_reports_classes_as_unsupported() {
    for source in [
        "class A { }",
        "new A()",
        "class P { } class D extends P { constructor() { super(); } }",
    ] {
        let error = eval(source).expect_err(source);
        assert!(
            error
                .message
                .contains("only supported by the bytecode compiler"),
            "{source}: {}",
            error.message
        );
    }
    assert_eq!(eval("this").unwrap_err().message, "`this` is not defined");
}

// Malformed bytecode --------------------------------------------------------------------------

fn run_raw(build: impl FnOnce(&mut Chunk)) -> Result<Value, JsError> {
    let mut chunk = Chunk::new();
    build(&mut chunk);
    chunk.emit(Instruction::Return);
    Vm::new().execute(&chunk)
}

#[test]
fn malformed_class_bytecode_is_an_error_not_a_panic() {
    let error = run_raw(|chunk| {
        chunk.emit(Instruction::DefineMethod {
            function: 0,
            name: 0,
        });
    })
    .unwrap_err();
    assert_eq!(error.category, JsErrorCategory::Runtime);

    assert!(run_raw(|chunk| _ = chunk.emit(Instruction::MakeFunction(9))).is_err());
    assert!(run_raw(|chunk| _ = chunk.emit(Instruction::Call(2))).is_err());
    assert!(run_raw(|chunk| _ = chunk.emit(Instruction::New(0))).is_err());
    assert!(run_raw(|chunk| _ = chunk.emit(Instruction::SuperCall(0))).is_err());
    assert!(run_raw(|chunk| _ = chunk.emit(Instruction::LoadSuperProperty(0))).is_err());
    assert!(
        run_raw(|chunk| {
            chunk.emit(Instruction::DefineClass {
                function: 0,
                has_superclass: true,
            });
        })
        .is_err()
    );
}

#[test]
fn load_this_and_arguments_work_at_the_top_level() {
    assert_eq!(
        run_raw(|chunk| _ = chunk.emit(Instruction::LoadThis)).unwrap(),
        Value::Undefined
    );
    assert_eq!(
        run_raw(|chunk| _ = chunk.emit(Instruction::LoadArgument(3))).unwrap(),
        Value::Undefined
    );
}

#[test]
fn the_vm_can_run_many_programs_in_a_row_after_a_failure() {
    let mut vm = Vm::new();
    let failing = compile(&parse("function f() { return f(); } f()").unwrap()).unwrap();
    assert!(vm.execute(&failing).is_err());
    let fine = compile(&parse("class A { v() { return 2; } } new A().v()").unwrap()).unwrap();
    assert_eq!(vm.execute(&fine).unwrap(), Value::Number(2.0));
}

// Reference cycles ----------------------------------------------------------------------------

#[test]
fn a_class_and_its_prototype_do_not_keep_each_other_alive() {
    // The prototype escapes as the program's result. Once the VM has dropped
    // the class, the prototype must not still own it through `constructor`.
    let Value::Object(prototype) = value("class A { m() { return 1; } } A.prototype") else {
        panic!("the prototype should be an object");
    };
    assert!(prototype.contains_key("m"), "methods stay on the prototype");
    assert_eq!(prototype.get("constructor"), None);
}

#[test]
fn constructor_is_not_an_enumerable_own_property() {
    let Value::Object(prototype) = value("class A { m() { return 1; } } A.prototype") else {
        panic!("the prototype should be an object");
    };
    assert_eq!(prototype.keys(), ["m"]);
}

// Host functions ------------------------------------------------------------------------------

fn run_host_call(emit: impl FnOnce(&mut Chunk, u16)) -> Result<Value, JsError> {
    let double = Function::from_host(
        "double",
        Arc::new(|arguments: &[Value]| match arguments {
            [Value::Number(number)] => Ok(Value::Number(number * 2.0)),
            _ => Err(JsError::new("double expects one number")),
        }),
    );
    let mut chunk = Chunk::new();
    let function = chunk.add_constant(Value::Function(double)).unwrap();
    emit(&mut chunk, function);
    chunk.emit(Instruction::Return);
    Vm::new().execute(&chunk)
}

#[test]
fn bytecode_can_call_host_functions() {
    let result = run_host_call(|chunk, function| {
        let argument = chunk.add_constant(Value::Number(21.0)).unwrap();
        chunk.emit(Instruction::Constant(function));
        chunk.emit(Instruction::Constant(argument));
        chunk.emit(Instruction::Call(1));
    });
    assert_eq!(result, Ok(Value::Number(42.0)));
}

#[test]
fn host_function_errors_propagate_and_hosts_are_not_constructors() {
    let error = run_host_call(|chunk, function| {
        chunk.emit(Instruction::Constant(function));
        chunk.emit(Instruction::Call(0));
    })
    .unwrap_err();
    assert_eq!(error.message, "double expects one number");

    let error = run_host_call(|chunk, function| {
        chunk.emit(Instruction::Constant(function));
        chunk.emit(Instruction::New(0));
    })
    .unwrap_err();
    assert!(error.message.contains("is not a constructor"), "{error:?}");
}
