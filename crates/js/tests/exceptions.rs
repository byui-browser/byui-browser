//! `throw`, `try`/`catch`/`finally`, `break`, and `continue`.
//!
//! The bytecode VM resolves exceptions itself with its handler stack; the
//! tree-walk interpreter implements the same semantics, and the differential
//! tests at the end pin the two together.

mod common;

use common::assert_equivalent;
use js::bytecode::{Chunk, Instruction, compile};
use js::runtime::ExecutionLimits;
use js::{Function, JsError, JsErrorCategory, Object, Realm, Value, Vm, eval, parse};
use std::sync::Arc;

fn run(source: &str) -> Result<Value, JsError> {
    run_with_limits(source, ExecutionLimits::default())
}

fn run_with_limits(source: &str, limits: ExecutionLimits) -> Result<Value, JsError> {
    let program = parse(source).expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    Vm::new().execute_with_limits(&chunk, limits)
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

fn number(number: f64) -> Value {
    Value::Number(number)
}

fn syntax_error(source: &str) -> String {
    parse(source).expect_err("source should not parse").message
}

// Throwing and catching -----------------------------------------------------------------------

#[test]
fn any_value_can_be_thrown_and_caught() {
    assert_eq!(value("try { throw 7 } catch (e) { e }"), number(7.0));
    assert_eq!(
        value("try { throw 'text' } catch (e) { e }"),
        string("text")
    );
    assert_eq!(
        value("try { throw undefined } catch (e) { e }"),
        Value::Undefined
    );
    assert_eq!(value("try { throw null } catch (e) { e }"), Value::Null);
    assert_eq!(
        value("try { throw false } catch (e) { e }"),
        Value::Boolean(false)
    );
    assert_eq!(
        value("let o = {}; let same; try { throw o } catch (e) { same = e === o } same"),
        Value::Boolean(true)
    );
    assert_eq!(
        value("let a = [1, 2]; try { throw a } catch (e) { e[1] }"),
        number(2.0)
    );
    assert_eq!(
        value("function f() {} try { throw f } catch (e) { e === f }"),
        Value::Boolean(true)
    );
}

#[test]
fn catch_binding_is_scoped_to_the_clause() {
    let error = error("try { throw 1 } catch (e) {} e");
    assert_eq!(error.category, JsErrorCategory::Runtime);
    assert!(error.message.contains("`e` is not defined"), "{error:?}");
    assert_eq!(
        value("let e = 'outer'; try { throw 1 } catch (e) { e = 2 } e"),
        string("outer")
    );
}

#[test]
fn catch_binding_is_optional() {
    assert_eq!(
        value("try { throw 1 } catch { 'handled' }"),
        string("handled")
    );
}

#[test]
fn skipped_catch_does_not_run() {
    assert_eq!(
        value("let ran = false; try { 1 } catch (e) { ran = true } ran"),
        Value::Boolean(false)
    );
}

#[test]
fn try_statements_complete_with_the_value_of_the_block_that_finished() {
    assert_eq!(value("try { 1 } catch (e) { 2 }"), number(1.0));
    assert_eq!(value("try { throw 0 } catch (e) { 2 }"), number(2.0));
    assert_eq!(value("try { 1 } finally { 2 }"), number(1.0));
    assert_eq!(
        value("try { throw 0 } catch (e) { 2 } finally { 3 }"),
        number(2.0)
    );
    assert_eq!(value("5; try { } catch (e) { }"), Value::Undefined);
}

#[test]
fn engine_errors_are_catchable_as_error_objects() {
    assert_eq!(
        value("try { missing } catch (e) { e.message }"),
        string("`missing` is not defined")
    );
    assert_eq!(
        value("try { missing } catch (e) { e.name }"),
        string("Error")
    );
    assert_eq!(
        value("let u; try { u.x } catch (e) { e.name }"),
        string("Error")
    );
    assert_eq!(
        value("let n = 1; try { n() } catch (e) { e.message }"),
        string("`1` is not a function")
    );
    assert_eq!(
        value("const c = 1; try { c = 2 } catch (e) { e.message }"),
        string("Assignment to constant `c`")
    );
}

#[test]
fn an_uncaught_throw_is_a_structured_error() {
    let failure = error("1; throw 'oops'");
    assert_eq!(failure.category, JsErrorCategory::Thrown);
    assert_eq!(failure.message, "Uncaught oops");
    assert_eq!(failure.thrown, Some(string("oops")));
    assert_eq!(failure.context.as_deref(), Some("throw statement"));

    let object = error("throw { name: 'TypeError', message: 'bad input' }");
    assert_eq!(object.category, JsErrorCategory::Thrown);
    assert_eq!(object.message, "Uncaught TypeError: bad input");
    let Some(Value::Object(thrown)) = object.thrown else {
        panic!("the thrown object should be preserved");
    };
    assert_eq!(thrown.get("message"), Some(string("bad input")));

    assert_eq!(error("throw 12").message, "Uncaught 12");
}

#[test]
fn uncaught_errors_keep_their_original_category() {
    let failure = error("missing");
    assert_eq!(failure.category, JsErrorCategory::Runtime);
    assert_eq!(failure.thrown, None);

    // A cleanup block does not change what an uncaught runtime error is.
    let failure = error("try { missing } finally { 1 }");
    assert_eq!(failure.category, JsErrorCategory::Runtime);
    assert_eq!(failure.message, "`missing` is not defined");
}

#[test]
fn rethrowing_a_caught_value_preserves_it() {
    let failure = error("let o = { id: 1 }; try { throw o } catch (e) { throw e }");
    let Some(Value::Object(thrown)) = failure.thrown else {
        panic!("expected a thrown object");
    };
    assert_eq!(thrown.get("id"), Some(number(1.0)));
}

#[test]
fn a_vm_can_run_again_after_an_uncaught_throw() {
    let mut vm = Vm::new();
    let throwing =
        compile(&parse("function f() { try { throw 1 } finally { 2 } } f()").unwrap()).unwrap();
    assert_eq!(
        vm.execute(&throwing).unwrap_err().category,
        JsErrorCategory::Thrown
    );
    let fine = compile(&parse("try { throw 1 } catch (e) { e + 1 }").unwrap()).unwrap();
    assert_eq!(vm.execute(&fine), Ok(number(2.0)));
}

// Nesting and unwinding -----------------------------------------------------------------------

#[test]
fn nested_handlers_run_innermost_first() {
    assert_eq!(
        value(
            "let log = '';
             try {
               try { throw 'a' } catch (e) { log = log + e; throw 'b' }
             } catch (e) {
               log = log + e;
             }
             log"
        ),
        string("ab")
    );
    assert_eq!(
        value(
            "try { try { try { throw 1 } catch (e) { throw e + 1 } } catch (e) { throw e + 1 } } catch (e) { e }"
        ),
        number(3.0)
    );
}

#[test]
fn an_exception_skips_handlers_that_do_not_cover_it() {
    assert_eq!(
        value("let r = 0; try { try { 1 } catch (e) { r = 99 } throw 5 } catch (e) { r = e } r"),
        number(5.0)
    );
}

#[test]
fn exceptions_unwind_call_frames() {
    assert_eq!(
        value(
            "function a() { throw 'deep' }
             function b() { a(); return 'unreached' }
             function c() { return b() }
             try { c() } catch (e) { e }"
        ),
        string("deep")
    );
}

#[test]
fn a_handler_in_a_middle_frame_catches_and_the_caller_continues() {
    assert_eq!(
        value(
            "function inner() { throw 'boom' }
             function outer() {
               let before = 1;
               try { inner() } catch (e) { before = before + 1 }
               return before;
             }
             outer() + outer()"
        ),
        number(4.0)
    );
}

#[test]
fn callee_frames_do_not_leak_operand_stack_values() {
    // The throw happens in the middle of an expression with operands pending.
    assert_eq!(
        value(
            "function bad() { throw 'x' }
             let total = 0;
             let i = 0;
             while (i < 50) {
               i = i + 1;
               try { total = total + (1 + (2 * bad())) } catch (e) { total = total + 1 }
             }
             total"
        ),
        number(50.0)
    );
}

#[test]
fn finally_blocks_run_while_frames_unwind() {
    assert_eq!(
        value(
            "let log = '';
             function a() { try { throw 'x' } finally { log = log + 'a' } }
             function b() { try { a() } finally { log = log + 'b' } }
             try { b() } catch (e) { log = log + e }
             log"
        ),
        string("abx")
    );
}

#[test]
fn catching_restores_the_scope_depth() {
    assert_eq!(
        value(
            "let x = 'outer';
             try { let x = 'inner'; { let x = 'deeper'; throw 1 } } catch (e) { x }"
        ),
        string("outer")
    );
    assert_eq!(
        value("{ let y = 1; try { { let z = 2; throw 0 } } catch (e) { y = y + 1 } y }"),
        number(2.0)
    );
    assert_eq!(
        value(
            "function f(a) {
               let seen = 0;
               try { { let b = 5; { let c = 6; throw b + c } } } catch (e) { seen = e + a }
               return seen;
             }
             f(1)"
        ),
        number(12.0)
    );
}

#[test]
fn many_caught_exceptions_do_not_grow_the_vm_state() {
    assert_eq!(
        value(
            "let n = 0; let i = 0;
             while (i < 500) { i = i + 1; try { throw i } catch (e) { n = n + e } }
             n"
        ),
        number(125_250.0)
    );
}

// Finally and abrupt completion ---------------------------------------------------------------

#[test]
fn finally_runs_after_normal_completion() {
    assert_eq!(
        value("let log = ''; try { log = log + 't' } finally { log = log + 'f' } log"),
        string("tf")
    );
    assert_eq!(
        value(
            "let log = ''; try { log = 't' } catch (e) { log = 'c' } finally { log = log + 'f' } log"
        ),
        string("tf")
    );
}

#[test]
fn finally_runs_after_a_caught_exception() {
    assert_eq!(
        value(
            "let log = '';
             try { throw 1 } catch (e) { log = log + 'c' } finally { log = log + 'f' }
             log"
        ),
        string("cf")
    );
}

#[test]
fn finally_runs_before_an_uncaught_exception_continues() {
    assert_eq!(
        value(
            "let log = '';
             try { try { throw 'x' } finally { log = log + 'f' } } catch (e) { log = log + e }
             log"
        ),
        string("fx")
    );
}

#[test]
fn an_exception_in_catch_still_runs_finally() {
    assert_eq!(
        value(
            "let log = '';
             try {
               try { throw 1 } catch (e) { throw 2 } finally { log = log + 'f' }
             } catch (e) { log = log + e }
             log"
        ),
        string("f2")
    );
}

#[test]
fn finally_runs_on_return_and_keeps_the_returned_value() {
    assert_eq!(
        value(
            "let log = '';
             function f() { try { return 'r' } finally { log = log + 'f' } }
             f() + log"
        ),
        string("rf")
    );
    assert_eq!(
        value(
            "function f(a) {
               let mark = 0;
               { let b = 2; try { { let c = 3; return a + b + c } } finally { mark = 1; b = 100 } }
             }
             f(1)"
        ),
        number(6.0)
    );
}

#[test]
fn return_from_catch_runs_finally() {
    assert_eq!(
        value(
            "let log = '';
             function f() {
               try { throw 1 } catch (e) { return e + 1 } finally { log = log + 'f' }
             }
             f() + log"
        ),
        string("2f")
    );
}

#[test]
fn nested_finally_blocks_run_innermost_first_on_return() {
    assert_eq!(
        value(
            "let log = '';
             function f() {
               try { try { return 'r' } finally { log = log + 'i' } } finally { log = log + 'o' }
             }
             f() + log"
        ),
        string("rio")
    );
}

#[test]
fn return_inside_a_loop_inside_try_discards_loop_state() {
    assert_eq!(
        value(
            "let log = '';
             function f() {
               let i = 0;
               while (i < 10) {
                 i = i + 1;
                 try { if (i == 3) { return i } } finally { log = log + i }
               }
               return -1;
             }
             f() + ':' + log"
        ),
        string("3:123")
    );
}

#[test]
fn finally_return_overrides_the_pending_outcome() {
    assert_eq!(
        value("function f() { try { return 1 } finally { return 2 } } f()"),
        number(2.0)
    );
    assert_eq!(
        value("function f() { try { throw 1 } finally { return 2 } } f()"),
        number(2.0)
    );
    assert_eq!(
        value("function f() { try { throw 1 } catch (e) { throw 2 } finally { return 3 } } f()"),
        number(3.0)
    );
}

#[test]
fn an_exception_from_finally_replaces_the_pending_outcome() {
    assert_eq!(
        value("try { try { throw 1 } finally { throw 2 } } catch (e) { e }"),
        number(2.0)
    );
    assert_eq!(
        value(
            "function f() { try { return 1 } finally { throw 'late' } } try { f() } catch (e) { e }"
        ),
        string("late")
    );
}

#[test]
fn break_leaves_the_loop_through_finally() {
    assert_eq!(
        value(
            "let log = ''; let i = 0;
             while (true) { i = i + 1; try { if (i == 3) { break } log = log + i } finally { log = log + 'f' } }
             log + ':' + i"
        ),
        string("1f2ff:3")
    );
}

#[test]
fn continue_runs_finally_and_starts_the_next_iteration() {
    assert_eq!(
        value(
            "let log = ''; let i = 0;
             while (i < 5) {
               i = i + 1;
               try {
                 if (i == 2) { continue }
                 if (i == 4) { break }
                 log = log + i;
               } finally { log = log + 'f' }
             }
             log"
        ),
        string("1ff3ff")
    );
}

#[test]
fn break_and_continue_work_without_exception_handling() {
    assert_eq!(
        value(
            "let i = 0; let s = 0; while (i < 10) { i = i + 1; if (i % 2 == 0) { continue } if (i > 7) { break } s = s + i } s"
        ),
        number(16.0)
    );
    // The loop's completion value stays what the last finished iteration set.
    assert_eq!(
        value("let i = 0; while (true) { i = i + 1; if (i == 3) { break } i } "),
        number(2.0)
    );
}

#[test]
fn break_targets_the_innermost_loop() {
    assert_eq!(
        value(
            "let out = 0; let i = 0;
             while (i < 3) {
               i = i + 1;
               let j = 0;
               while (true) { j = j + 1; if (j == 2) { break } out = out + 10 }
               out = out + 1;
             }
             out"
        ),
        number(33.0)
    );
}

#[test]
fn break_from_nested_blocks_restores_scopes() {
    assert_eq!(
        value(
            "let hits = 0; let i = 0;
             while (i < 3) {
               i = i + 1;
               { let a = 1; { let b = 2; try { { let c = 3; if (i == 2) { break } hits = hits + a + b + c } } finally { hits = hits + 100 } } }
             }
             hits + ':' + i"
        ),
        string("206:2")
    );
}

#[test]
fn break_in_finally_swallows_an_exception() {
    assert_eq!(
        value("while (true) { try { throw 1 } finally { break } } 'done'"),
        string("done")
    );
    assert_eq!(
        value("function f() { while (true) { try { return 1 } finally { break } } return 2 } f()"),
        number(2.0)
    );
    assert_eq!(
        value(
            "let i = 0;
             while (i < 3) { i = i + 1; try { throw i } finally { continue } }
             i"
        ),
        number(3.0)
    );
}

#[test]
fn a_loop_inside_finally_does_not_confuse_the_outer_unwind() {
    assert_eq!(
        value(
            "let log = '';
             function f() {
               try { return 'r' } finally {
                 let k = 0;
                 while (true) { k = k + 1; if (k == 3) { break } }
                 log = log + k;
               }
             }
             f() + log"
        ),
        string("r3")
    );
}

#[test]
fn exceptions_cross_loops_and_the_vm_stays_consistent() {
    assert_eq!(
        value(
            "let caught = 0; let i = 0;
             while (i < 4) {
               i = i + 1;
               try {
                 let j = 0;
                 while (true) { j = j + 1; if (j == i) { throw j } }
               } catch (e) { caught = caught + e }
             }
             caught"
        ),
        number(10.0)
    );
}

// Host errors and limits ----------------------------------------------------------------------

fn run_host_try(
    host: impl Fn(&[Value]) -> Result<Value, JsError> + Send + Sync + 'static,
) -> Result<Value, JsError> {
    let mut chunk = Chunk::new();
    let function = chunk
        .add_constant(Value::Function(Function::from_host("host", Arc::new(host))))
        .unwrap();
    chunk.emit(Instruction::EnterTry(5));
    chunk.emit(Instruction::Constant(function));
    chunk.emit(Instruction::Call(0));
    chunk.emit(Instruction::ExitTry);
    chunk.emit(Instruction::Return);
    chunk.emit(Instruction::Return);
    Vm::new().execute(&chunk)
}

#[test]
fn host_errors_are_catchable_exceptions() {
    let caught = run_host_try(|_| {
        Err(JsError::with_context(
            JsErrorCategory::Host,
            "disk full",
            "save",
        ))
    });
    let Ok(Value::Object(caught)) = caught else {
        panic!("expected a caught error object, got {caught:?}");
    };
    assert_eq!(caught.get("name"), Some(string("HostError")));
    assert_eq!(caught.get("message"), Some(string("disk full")));
}

#[test]
fn a_host_function_can_throw_any_value() {
    let marker = Object::new();
    let thrown = marker.clone();
    let caught = run_host_try(move |_| Err(JsError::thrown(Value::Object(thrown.clone()))));
    assert_eq!(caught, Ok(Value::Object(marker)));
}

#[test]
fn an_uncaught_host_error_keeps_its_category() {
    let program = parse("function f() { }").unwrap();
    let _ = compile(&program).unwrap();
    let mut chunk = Chunk::new();
    let function = chunk
        .add_constant(Value::Function(Function::from_host(
            "host",
            Arc::new(|_| Err(JsError::with_context(JsErrorCategory::Host, "nope", "save"))),
        )))
        .unwrap();
    chunk.emit(Instruction::Constant(function));
    chunk.emit(Instruction::Call(0));
    chunk.emit(Instruction::Return);
    let failure = Vm::new().execute(&chunk).unwrap_err();
    assert_eq!(failure.category, JsErrorCategory::Host);
    assert_eq!(failure.message, "nope");
}

#[test]
fn limit_errors_cannot_be_caught_and_skip_finally() {
    let limits = ExecutionLimits {
        loop_iteration_limit: 50,
        ..ExecutionLimits::default()
    };
    let failure =
        run_with_limits("try { while (true) { } } catch (e) { 'caught' }", limits).unwrap_err();
    assert_eq!(failure.category, JsErrorCategory::Limit);

    let failure = run_with_limits(
        "let ran = false; try { while (true) { } } finally { ran = true }",
        limits,
    )
    .unwrap_err();
    assert_eq!(failure.category, JsErrorCategory::Limit);

    let failure = error("function f() { return f() } try { f() } catch (e) { 'caught' }");
    assert_eq!(failure.category, JsErrorCategory::Limit);

    let steps = ExecutionLimits {
        step_limit: 200,
        ..ExecutionLimits::default()
    };
    let failure = run_with_limits(
        "let i = 0; while (true) { try { i = i + 1 } catch (e) { } }",
        steps,
    )
    .unwrap_err();
    assert_eq!(failure.category, JsErrorCategory::Limit);
}

// Compiler and bytecode -----------------------------------------------------------------------

#[test]
fn disassembly_lists_the_handler_instructions() {
    let chunk = compile(&parse("try { throw 1 } catch (e) { e }").unwrap()).unwrap();
    let text = chunk.disassemble();
    assert!(text.contains("ENTER_TRY ->"), "{text}");
    assert!(text.contains("ExitTry"), "{text}");
    assert!(text.contains("Throw"), "{text}");
}

#[test]
fn handler_instructions_are_balanced_in_compiled_code() {
    for source in [
        "try { 1 } catch (e) { 2 }",
        "try { 1 } finally { 2 }",
        "try { 1 } catch (e) { 2 } finally { 3 }",
        "function f() { try { return 1 } catch (e) { return 2 } finally { return 3 } } f()",
        "while (true) { try { break } finally { continue } }",
    ] {
        let chunk = compile(&parse(source).unwrap()).unwrap();
        let mut chunks = vec![&chunk];
        chunks.extend(chunk.functions.iter().map(|template| &*template.chunk));
        let count = |matcher: fn(&Instruction) -> bool| {
            chunks
                .iter()
                .flat_map(|chunk| chunk.code.iter())
                .filter(|instruction| matcher(instruction))
                .count()
        };
        let enters = count(|instruction| matches!(instruction, Instruction::EnterTry(_)));
        let exits = count(|instruction| matches!(instruction, Instruction::ExitTry));
        assert!(enters >= 1, "{source}");
        assert!(exits >= enters, "{source}: every handler needs an exit");
    }
}

#[test]
fn malformed_handler_bytecode_is_an_error_not_a_panic() {
    let mut chunk = Chunk::new();
    chunk.emit(Instruction::ExitTry);
    chunk.emit(Instruction::Return);
    let failure = Vm::new().execute(&chunk).unwrap_err();
    assert_eq!(failure.category, JsErrorCategory::Runtime);

    let mut chunk = Chunk::new();
    chunk.emit(Instruction::EnterTry(99));
    chunk.emit(Instruction::Return);
    assert!(Vm::new().execute(&chunk).is_err());

    let mut chunk = Chunk::new();
    chunk.emit(Instruction::Throw);
    let failure = Vm::new().execute(&chunk).unwrap_err();
    assert!(failure.message.contains("underflow"), "{failure:?}");
}

#[test]
fn break_and_continue_outside_loops_do_not_compile() {
    use js::ast::{Program, Statement};
    for statement in [Statement::Break, Statement::Continue] {
        let program = Program {
            body: vec![statement],
        };
        let failure = compile(&program).expect_err("must not compile");
        assert!(!failure.is_unsupported());
    }
}

// Parser --------------------------------------------------------------------------------------

#[test]
fn throw_requires_an_expression_on_the_same_line() {
    assert!(syntax_error("throw\n1").contains("Illegal newline after throw"));
    assert!(parse("throw").is_err());
    assert!(parse("throw 1;").is_ok());
}

#[test]
fn try_needs_catch_or_finally() {
    assert!(syntax_error("try { }").contains("catch or finally"));
    assert!(parse("try { } catch (e) { }").is_ok());
    assert!(parse("try { } finally { }").is_ok());
    assert!(parse("try { } catch { } finally { }").is_ok());
    assert!(parse("try { } catch (e").is_err());
    assert!(parse("try { } catch () { }").is_err());
    assert!(parse("try 1").is_err());
}

#[test]
fn break_and_continue_need_an_enclosing_loop_in_the_same_function() {
    assert!(syntax_error("break;").contains("enclosing loop"));
    assert!(syntax_error("continue;").contains("enclosing loop"));
    assert!(parse("if (true) { break }").is_err());
    assert!(parse("while (true) { function f() { break } }").is_err());
    assert!(parse("while (true) { let f = () => { continue }; }").is_err());
    assert!(parse("while (true) { break }").is_ok());
    assert!(parse("while (true) { if (true) { continue } }").is_ok());
    assert!(parse("while (true) { try { break } finally { continue } }").is_ok());
}

// Tree-walk interpreter and Realm -------------------------------------------------------------

#[test]
fn the_interpreter_implements_the_same_exception_rules() {
    assert_eq!(eval("try { throw 3 } catch (e) { e + 1 }"), Ok(number(4.0)));
    assert_eq!(
        eval(
            "let log = ''; function f() { try { return 'r' } finally { log = log + 'f' } } f() + log"
        ),
        Ok(string("rf"))
    );
    let failure = eval("throw 'bad'").unwrap_err();
    assert_eq!(failure.category, JsErrorCategory::Thrown);
    assert_eq!(failure.thrown, Some(string("bad")));
}

#[test]
fn the_interpreter_restores_call_depth_after_a_caught_exception() {
    let limits = ExecutionLimits {
        call_depth_limit: 8,
        ..ExecutionLimits::default()
    };
    let source = "function bad() { throw 1 }
                  function mid() { return bad() }
                  let n = 0; let i = 0;
                  while (i < 100) { i = i + 1; try { mid() } catch (e) { n = n + 1 } }
                  n";
    assert_eq!(js::eval_with_limits(source, limits), Ok(number(100.0)));
}

#[test]
fn realm_scripts_can_use_exceptions() {
    let mut realm = Realm::new();
    realm
        .register_global_function(
            "fail",
            Arc::new(|_| Err(JsError::thrown(Value::String("from host".into())))),
        )
        .unwrap();
    assert_eq!(
        realm.evaluate_script("try { fail() } catch (e) { e }"),
        Ok(string("from host"))
    );
    let failure = realm.evaluate_script("fail()").unwrap_err();
    assert_eq!(failure.category, JsErrorCategory::Thrown);
}

#[test]
fn the_try_catch_fixture_evaluates() {
    let source = include_str!("js/try-catch.js");
    let Ok(Value::Object(caught)) = eval(source) else {
        panic!("the fixture should evaluate to the caught error");
    };
    assert_eq!(caught.get("name"), Some(string("Error")));
}

#[test]
fn both_engines_agree_on_exception_behavior() {
    for source in [
        "try { throw 1 } catch (e) { e }",
        "try { 1 } finally { 2 }",
        "try { throw 1 } catch (e) { 2 } finally { 3 }",
        "try { try { throw 1 } finally { throw 2 } } catch (e) { e }",
        "let log = ''; try { try { throw 'x' } finally { log = log + 'f' } } catch (e) { log = log + e } log",
        "function f() { try { return 1 } finally { return 2 } } f()",
        "function f() { try { throw 1 } finally { return 2 } } f()",
        "function a() { throw 'deep' } function b() { a() } try { b() } catch (e) { e }",
        "let i = 0; let log = ''; while (i < 5) { i = i + 1; try { if (i == 2) { continue } if (i == 4) { break } log = log + i } finally { log = log + 'f' } } log",
        "while (true) { try { throw 1 } finally { break } } 'done'",
        "function f() { while (true) { try { return 1 } finally { break } } return 2 } f()",
        "let x = 'outer'; try { let x = 'inner'; { throw 1 } } catch (e) { x }",
        "try { throw 1 } catch { 'optional' }",
        "throw 'uncaught'",
        "throw { message: 'bad', name: 'Oops' }",
        "try { throw 1 } finally { 2 }",
        "try { missing } catch (e) { 1 }",
        "let e = 1; try { throw 2 } catch (e) { e = 3 } e",
    ] {
        assert_equivalent(source);
    }
}

#[test]
fn rethrowing_a_caught_engine_error_restores_the_original_error() {
    for source in [
        "try { missing } catch (e) { throw e }",
        "try { try { missing } finally { 1 } } catch (e) { throw e }",
    ] {
        let failure = error(source);
        assert_eq!(failure.category, JsErrorCategory::Runtime, "{source}");
        assert_eq!(failure.message, "`missing` is not defined", "{source}");
        assert_eq!(failure.thrown, None, "{source}");
    }
    // A new object with the same fields is a different value.
    let failure = error("try { missing } catch (e) { throw { message: e.message } }");
    assert_eq!(failure.category, JsErrorCategory::Thrown);
}
