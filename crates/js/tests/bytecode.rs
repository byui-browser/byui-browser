use js::bytecode::{Instruction, compile};
use js::{Value, Vm, parse};

#[test]
fn execution_modes_select_vm_for_programs_with_compiled_functions() {
    assert_eq!(
        js::eval_with_mode("let n = 2; n * 3", js::ExecutionMode::Bytecode).unwrap(),
        Value::Number(6.0)
    );
    assert_eq!(
        js::eval_with_mode(
            "function f() { return 4; } f()",
            js::ExecutionMode::Automatic
        )
        .unwrap(),
        Value::Number(4.0)
    );
    let error = js::eval_with_mode("missing", js::ExecutionMode::Automatic).unwrap_err();
    assert_eq!(error.category, js::JsErrorCategory::Runtime);
    assert_eq!(error.message, "`missing` is not defined");

    let source = "if (true) function conditional() { return 1; } 8";
    assert_eq!(
        js::eval_with_mode(source, js::ExecutionMode::Automatic),
        js::eval_with_mode(source, js::ExecutionMode::Interpreter)
    );
}

#[test]
fn vm_runtime_errors_are_not_retried_in_the_interpreter() {
    let error = js::eval_with_mode("const n = 1; n = 2", js::ExecutionMode::Automatic)
        .expect_err("VM const assignment must fail");
    assert_eq!(error.category, js::JsErrorCategory::Runtime);
    assert_eq!(error.message, "Assignment to constant `n`");
}

#[test]
fn function_frames_match_the_evaluator_for_recursion_and_completion() {
    for source in [
        "function factorial(n) { if (n <= 1) return 1; return n * factorial(n - 1); } factorial(6)",
        "function outer(a) { function inner(b) { return a + b; } return inner(5); } outer(7)",
        "function identity(value) { return value; } identity()",
        "function noReturn() { let value = 1; } noReturn()",
        "function early() { return; 9; } early()",
        "let order = 0; function pair(a, b) { return a * 10 + b; } pair(order = order + 1, order = order + 1) + order",
    ] {
        let program = parse(source).expect("source should parse");
        let chunk = compile(&program).expect("function source should compile");
        assert_eq!(
            Vm::new().execute(&chunk),
            js::eval(source),
            "VM differs from evaluator for {source}"
        );
    }
}

#[test]
fn closures_share_captured_slots_and_survive_their_outer_call() {
    let source = "function makeCounter() { let count = 0; function next() { count = count + 1; return count; } return next; } let counter = makeCounter(); counter() + counter()";
    let program = parse(source).expect("source should parse");
    let chunk = compile(&program).expect("closure source should compile");
    assert_eq!(Vm::new().execute(&chunk), js::eval(source));
    assert_eq!(Vm::new().execute(&chunk), Ok(Value::Number(3.0)));
}

#[test]
fn block_closures_capture_their_lexical_environment() {
    let source =
        "let read; { let value = 12; function capture() { return value; } read = capture; } read()";
    let program = parse(source).expect("source should parse");
    let chunk = compile(&program).expect("block closure should compile");
    assert_eq!(Vm::new().execute(&chunk), js::eval(source));
    assert_eq!(Vm::new().execute(&chunk), Ok(Value::Number(12.0)));
}

#[test]
fn call_depth_and_non_function_calls_are_structured_runtime_errors() {
    let source = "function recurse() { return recurse(); } recurse()";
    let program = parse(source).expect("source should parse");
    let chunk = compile(&program).expect("recursive call should compile");
    let limits = js::runtime::ExecutionLimits {
        call_depth_limit: 2,
        ..js::runtime::ExecutionLimits::default()
    };
    let error = Vm::new()
        .execute_with_limits(&chunk, limits)
        .expect_err("recursion should hit call depth limit");
    assert_eq!(error.category, js::JsErrorCategory::Limit);
    assert_eq!(error.message, "Maximum call stack size exceeded");
    assert_eq!(
        error,
        js::runtime::evaluate_program_with_limits(&program, limits).unwrap_err()
    );

    let error = js::eval_with_mode("1()", js::ExecutionMode::Bytecode)
        .expect_err("a number is not callable");
    assert_eq!(error.category, js::JsErrorCategory::Runtime);
    assert_eq!(error.context.as_deref(), Some("function call"));
    assert_eq!(error, js::eval("1()").unwrap_err());
}

fn execute(source: &str) -> Value {
    let program = parse(source).expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    Vm::new().execute(&chunk).expect("bytecode should execute")
}

#[test]
fn compiles_and_executes_expression_subset() {
    assert_eq!(execute("1 + 2 * 3"), Value::Number(7.0));
    assert_eq!(execute("-'4'"), Value::Number(-4.0));
    assert_eq!(
        execute("false || 'fallback'"),
        Value::String("fallback".into())
    );
    assert_eq!(execute("1; { 2 }"), Value::Number(2.0));
}

#[test]
fn logical_operations_short_circuit() {
    assert_eq!(execute("true || (1 / 0)"), Value::Boolean(true));
    assert_eq!(execute("false && (1 / 0)"), Value::Boolean(false));
}

#[test]
fn bindings_compile_and_execute() {
    assert_eq!(
        execute("var value = 1; value = value + 2; value"),
        Value::Number(3.0)
    );
    assert_eq!(
        execute("let value = 1; { let value = 2; value = 3; } value"),
        Value::Number(1.0)
    );
    assert_eq!(execute("const value = 4; value"), Value::Number(4.0));
}

#[test]
fn binding_errors_are_reported() {
    let program = parse("{ value; let value = 1; }").expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    let error = Vm::new().execute(&chunk).expect_err("TDZ read should fail");
    assert_eq!(error.message, "Cannot access `value` before initialization");

    let program = parse("const value = 1; value = 2;").expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    let error = Vm::new()
        .execute(&chunk)
        .expect_err("const write should fail");
    assert_eq!(error.message, "Assignment to constant `value`");

    let program = parse("missing").expect("source should parse");
    let chunk = compile(&program).expect("unresolved names compile for VM diagnostics");
    let error = Vm::new()
        .execute(&chunk)
        .expect_err("unknown read should fail");
    assert_eq!(error.message, "`missing` is not defined");
}

#[test]
fn each_execution_starts_with_fresh_bindings() {
    let program = parse("var value = 1; value").expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    let mut vm = Vm::new();
    assert_eq!(vm.execute(&chunk).unwrap(), Value::Number(1.0));
    assert_eq!(vm.execute(&chunk).unwrap(), Value::Number(1.0));
}

#[test]
fn chunk_keeps_constants_separate_from_instructions() {
    let program = parse("'hello'").expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    assert!(
        chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Constant(0)))
    );
    assert_eq!(chunk.constants, vec![Value::String("hello".into())]);
}

#[test]
fn control_flow_preserves_truthiness_and_completion_values() {
    assert_eq!(execute("if ('yes') { 1 } else { 2 }"), Value::Number(1.0));
    assert_eq!(execute("if (0) 1; else 2"), Value::Number(2.0));
    assert_eq!(execute("if (false) 1"), Value::Undefined);
    assert_eq!(execute("while (false) 1"), Value::Undefined);
    assert_eq!(
        execute("let i = 0; while (i < 3) { i = i + 1; } i"),
        Value::Number(3.0)
    );
    assert_eq!(
        execute("if (true) { if (false) 1; else 2 } else 3"),
        Value::Number(2.0)
    );
}

#[test]
fn control_flow_keeps_short_circuit_evaluation_order() {
    assert_eq!(
        execute("let i = 0; false && (i = 1); true || (i = 2); i"),
        Value::Number(0.0)
    );
    assert_eq!(
        execute("let i = 0; while (i < 2) { i = i + 1; if (i == 1) 10; else 20 }"),
        Value::Number(20.0)
    );
}

#[test]
fn vm_limits_bound_bytecode_execution() {
    let program = parse("while (true) { }").expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    let limits = js::runtime::ExecutionLimits {
        step_limit: 10_000,
        call_depth_limit: 0,
        loop_iteration_limit: 3,
    };
    let error = Vm::new()
        .execute_with_limits(&chunk, limits)
        .expect_err("infinite loop should hit its loop limit");
    assert_eq!(error.category, js::JsErrorCategory::Limit);
    assert_eq!(error.message, "Script exceeded the loop iteration limit");

    let limits = js::runtime::ExecutionLimits {
        step_limit: 2,
        ..js::runtime::ExecutionLimits::default()
    };
    let error = Vm::new()
        .execute_with_limits(&chunk, limits)
        .expect_err("infinite loop should hit its step limit");
    assert_eq!(error.message, "Script exceeded the evaluation step limit");
}
