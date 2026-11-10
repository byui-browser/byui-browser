//! Differential coverage: the evaluator and the bytecode VM must agree on
//! every operator and operand pair in the supported syntax subset.

use js::bytecode::compile;
use js::{JsResult, Value, Vm, eval, parse};

/// Operand literals covering NaN, infinities, signed zero, nullish values,
/// numeric and non-numeric strings, booleans, and objects.
const OPERANDS: &[&str] = &[
    "0",
    "-0",
    "1",
    "-1",
    "2.5",
    "4294967296",
    "2147483648",
    "(1 / 0)",
    "(-1 / 0)",
    "(0 / 0)",
    "null",
    "undefined",
    "true",
    "false",
    "''",
    "' '",
    "'0'",
    "'1'",
    "'abc'",
    "'0x10'",
    "'Infinity'",
    "'b'",
];

const BINARY: &[&str] = &[
    "+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>", ">>>", "<", "<=", ">", ">=", "==", "!=",
    "===", "!==",
];

const UNARY: &[&str] = &["-", "!", "~"];

fn run_vm(source: &str) -> JsResult<Value> {
    let program = parse(source).expect("source should parse");
    let chunk = compile(&program).expect("source should compile");
    Vm::new().execute(&chunk)
}

/// Debug formatting distinguishes `NaN` (which never compares equal) and `-0`.
fn outcome(result: JsResult<Value>) -> String {
    match result {
        Ok(value) => format!("ok {value:?}"),
        Err(error) => format!("err {:?} {}", error.category, error.message),
    }
}

fn assert_agree(source: &str) {
    assert_eq!(
        outcome(run_vm(source)),
        outcome(eval(source)),
        "evaluator and VM disagree on `{source}`"
    );
}

#[test]
fn binary_operators_agree_for_every_operand_pair() {
    for operator in BINARY {
        for left in OPERANDS {
            for right in OPERANDS {
                assert_agree(&format!("{left} {operator} {right}"));
            }
        }
    }
}

#[test]
fn unary_operators_agree_for_every_operand() {
    for operator in UNARY {
        for operand in OPERANDS {
            assert_agree(&format!("{operator}{operand}"));
        }
    }
}

#[test]
fn logical_operators_agree_for_every_operand_pair() {
    for operator in ["&&", "||"] {
        for left in OPERANDS {
            for right in OPERANDS {
                assert_agree(&format!("{left} {operator} {right}"));
            }
        }
    }
}

#[test]
fn spot_checks_match_javascript() {
    for (source, expected) in [
        ("'b' > 'a'", "ok Boolean(true)"),
        ("0 / 0 == 0 / 0", "ok Boolean(false)"),
        ("null == undefined", "ok Boolean(true)"),
        ("null == 0", "ok Boolean(false)"),
        ("'1' == 1", "ok Boolean(true)"),
        ("-0 === 0", "ok Boolean(true)"),
        ("1 + '1'", "ok String(\"11\")"),
        ("-1 >>> 0", "ok Number(4294967295.0)"),
    ] {
        assert_eq!(outcome(run_vm(source)), expected, "{source}");
    }
}
