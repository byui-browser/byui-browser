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
fn compiler_rejects_constructs_without_bytecode_support() {
    let program = parse("let value = 1;").expect("source should parse");
    let error = compile(&program).expect_err("declarations are not supported yet");
    assert!(error.message.contains("variable declarations"));
}

#[test]
fn chunk_keeps_constants_separate_from_instructions() {
    let program = parse("'hello'").expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    assert!(matches!(chunk.code[0], Instruction::Constant(0)));
    assert_eq!(chunk.constants, vec![Value::String("hello".into())]);
}
