use js::bytecode::{Instruction, compile};
use js::{Value, Vm, parse};

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
