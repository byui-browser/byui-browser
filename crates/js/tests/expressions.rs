//! Expression forms beyond the Sprint 4 baseline: conditional, comma,
//! update, compound assignment, `typeof`/`void`/`delete`, `??`, `?.`, `in`,
//! `instanceof`, template literals, spread, and `**`.
//!
//! Most cases run one source through the tree-walk evaluator and through the
//! bytecode VM and require identical results, errors, and side effects. The
//! later sections pin down the lexer, the parser, the emitted bytecode, the
//! stack effect of each new instruction, and malformed bytecode.

use js::ast::{AssignmentOperator, BinaryOperator, Expr, LogicalOperator, MemberProperty};
use js::bytecode::{Chunk, Instruction, compile};
use js::lexer::{Token, tokenize};
use js::runtime::evaluate_program;
use js::{ExecutionMode, JsError, JsErrorCategory, Object, Value, Vm, eval_with_mode, parse};

fn compile_source(source: &str) -> Chunk {
    compile(&parse(source).expect("source should parse")).expect("source should compile")
}

fn run_vm(source: &str) -> Result<Value, JsError> {
    Vm::new().execute(&compile_source(source))
}

fn run_tree(source: &str) -> Result<Value, JsError> {
    evaluate_program(&parse(source).expect("source should parse"))
}

/// Debug text that tells `NaN`, `-0`, and string/number apart.
fn outcome(result: &Result<Value, JsError>) -> String {
    match result {
        Ok(value) => format!("ok {value:?}"),
        Err(error) => format!("err {:?} {}", error.category, error.message),
    }
}

/// Runs `source` on both engines, requires the same result, and returns it.
fn show(source: &str) -> Value {
    let tree = run_tree(source);
    let vm = run_vm(source);
    assert_eq!(
        outcome(&tree),
        outcome(&vm),
        "engines disagree on `{source}`"
    );
    vm.unwrap_or_else(|error| panic!("`{source}` failed: {error:?}"))
}

fn number(value: f64) -> Value {
    Value::Number(value)
}

fn string(text: &str) -> Value {
    Value::String(text.into())
}

fn boolean(value: bool) -> Value {
    Value::Boolean(value)
}

/// Requires both engines to fail with a message containing `expected`.
fn fails(source: &str, expected: &str) {
    for (engine, result) in [("tree", run_tree(source)), ("vm", run_vm(source))] {
        let error = result.expect_err(&format!("{engine} `{source}` should fail"));
        assert!(
            error.message.contains(expected),
            "{engine} `{source}`: {error:?}"
        );
    }
}

/// Requires both engines to fail at run time. Messages may differ: the
/// evaluator names a callee by its path and the VM by its value.
fn fails_at_run_time(source: &str) {
    for (engine, result) in [("tree", run_tree(source)), ("vm", run_vm(source))] {
        let error = result.expect_err(&format!("{engine} `{source}` should fail"));
        assert_eq!(
            error.category,
            JsErrorCategory::Runtime,
            "{engine} `{source}`"
        );
    }
}

fn is_nan(source: &str) {
    assert_eq!(show(source).to_string(), "NaN", "{source}");
}

fn syntax_error(source: &str) -> JsError {
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        let error = eval_with_mode(source, mode).expect_err(source);
        assert_eq!(error.category, JsErrorCategory::Syntax, "{source}: {error}");
    }
    eval_with_mode(source, ExecutionMode::Interpreter).unwrap_err()
}

// Conditional ---------------------------------------------------------------------------------

#[test]
fn conditional_selects_one_branch() {
    assert_eq!(show("true ? 1 : 2"), number(1.0));
    assert_eq!(show("0 ? 1 : 2"), number(2.0));
    assert_eq!(show("'' ? 'a' : 'b'"), string("b"));
    assert_eq!(show("null ? 1 : undefined"), Value::Undefined);
    assert_eq!(show("1 + 1 === 2 ? 'yes' : 'no'"), string("yes"));
}

#[test]
fn conditional_is_right_associative_and_nests() {
    assert_eq!(show("1 ? 2 : 3 ? 4 : 5"), number(2.0));
    assert_eq!(show("0 ? 1 : 0 ? 2 : 3"), number(3.0));
    assert_eq!(show("1 ? 0 ? 'a' : 'b' : 'c'"), string("b"));
}

#[test]
fn conditional_evaluates_only_the_selected_branch() {
    assert_eq!(show("let n = 0; true ? (n = 1) : (n = 2); n"), number(1.0));
    assert_eq!(show("let n = 0; false ? (n = 1) : (n = 2); n"), number(2.0));
    assert_eq!(show("0 ? missing : 2"), number(2.0));
    assert_eq!(show("1 ? 3 : missing"), number(3.0));
    fails("missing ? 1 : 2", "`missing` is not defined");
    fails("1 ? missing : 2", "`missing` is not defined");
}

#[test]
fn conditional_is_a_completion_value_and_a_function_body() {
    assert_eq!(show("1; true ? 2 : 3"), number(2.0));
    assert_eq!(
        show("function f(a) { return a; } f(1 ? 'y' : 'n')"),
        string("y")
    );
    assert_eq!(
        show("const sign = x => x > 0 ? 'pos' : 'neg'; sign(1) + sign(-1)"),
        string("posneg")
    );
    assert_eq!(show("let x = 0; x = true ? 1 : 2; x"), number(1.0));
}

#[test]
fn question_dot_before_a_digit_is_a_conditional() {
    assert_eq!(show("true?.5:1"), number(0.5));
    assert_eq!(show("false?.5:1"), number(1.0));
}

// Comma ---------------------------------------------------------------------------------------

#[test]
fn comma_evaluates_every_operand_and_yields_the_last() {
    assert_eq!(show("(1, 2, 3)"), number(3.0));
    assert_eq!(show("1, 2"), number(2.0));
    assert_eq!(show("let a = 0; (a = 1, a = a + 1, a * 10)"), number(20.0));
    assert_eq!(
        show("let log = ''; (log += 'a', log += 'b'); log"),
        string("ab")
    );
    assert_eq!(show("let a = (1, 2); a"), number(2.0));
}

#[test]
fn comma_is_not_a_sequence_in_lists() {
    assert_eq!(
        show("function f(a, b) { return a + b; } f(1, 2)"),
        number(3.0)
    );
    assert_eq!(show("[1, 2, 3].length"), number(3.0));
    assert_eq!(show("[10, 20, 30][0, 2]"), number(30.0));
    assert_eq!(show("let o = { a: 1, b: 2 }; o.a + o.b"), number(3.0));
}

#[test]
fn comma_works_in_loop_clauses() {
    assert_eq!(
        show("let n = 0; for (let i = 0; i < 3; i++, n += 2) {} n"),
        number(6.0)
    );
    assert_eq!(
        show("let i = 0; let n = 0; while (i++, i < 3) { n += 1; } n"),
        number(2.0)
    );
}

#[test]
fn comma_fails_at_the_first_failing_operand() {
    fails("(1, missing, 3)", "`missing` is not defined");
    assert_eq!(show("let n = 0; (n = 1, 2); n"), number(1.0));
}

// Update --------------------------------------------------------------------------------------

#[test]
fn update_on_variables() {
    assert_eq!(show("let i = 1; i++"), number(1.0));
    assert_eq!(show("let i = 1; ++i"), number(2.0));
    assert_eq!(show("let i = 1; i--"), number(1.0));
    assert_eq!(show("let i = 1; --i"), number(0.0));
    assert_eq!(show("let i = 1; i++; i"), number(2.0));
    assert_eq!(show("let i = 1; i++ + i++"), number(3.0));
    assert_eq!(show("let i = 1; ++i + ++i"), number(5.0));
    assert_eq!(show("var v = 0; v++; v++; v"), number(2.0));
}

#[test]
fn update_converts_the_old_value_to_a_number() {
    assert_eq!(show("let s = '5'; s++"), number(5.0));
    assert_eq!(show("let s = '5'; s++; s"), number(6.0));
    assert_eq!(show("let b = true; ++b"), number(2.0));
    assert_eq!(show("let n = null; n--"), number(0.0));
    is_nan("let u; u++; u");
    is_nan("let s = 'x'; ++s");
}

#[test]
fn update_on_members_evaluates_the_reference_once() {
    assert_eq!(show("let o = { n: 1 }; o.n++"), number(1.0));
    assert_eq!(show("let o = { n: 1 }; o.n++; o.n"), number(2.0));
    assert_eq!(show("let o = { n: 1 }; ++o.n"), number(2.0));
    assert_eq!(show("let o = { n: 1 }; --o.n; o.n"), number(0.0));
    assert_eq!(show("let a = [5]; a[0]--"), number(5.0));
    assert_eq!(show("let a = [5]; a[0]--; a[0]"), number(4.0));
    assert_eq!(show("let a = [5]; --a[0]"), number(4.0));
    assert_eq!(
        show("let a = [1, 2]; let i = 0; a[i++]++; a[0] * 10 + i"),
        number(21.0)
    );
    assert_eq!(
        show("let a = [1, 2]; let i = 0; ++a[i++]; a[0] * 10 + i"),
        number(21.0)
    );
    assert_eq!(
        show(
            "let log = ''; let o = { n: 1 }; function k() { log += 'k'; return 'n'; } o[k()]++; log + o.n"
        ),
        string("k2")
    );
}

#[test]
fn update_reports_reference_errors() {
    fails("const c = 1; c++", "Assignment to constant `c`");
    fails("const c = 1; --c", "Assignment to constant `c`");
    fails("missing++", "`missing` is not defined");
    fails("++missing", "`missing` is not defined");
    fails("let n = 1; n.x++", "Cannot read property `x` of a number");
    fails("let o = null; o.x++", "Cannot read property `x` of null");
}

#[test]
fn postfix_update_needs_the_operator_on_the_same_line() {
    assert_eq!(
        show("let a = 1; let b = 5;\na\n++b\na * 100 + b"),
        number(106.0)
    );
}

// Compound assignment -------------------------------------------------------------------------

#[test]
fn compound_assignment_applies_each_operator() {
    for (operator, expected) in [
        ("+=", 9.0),
        ("-=", 3.0),
        ("*=", 18.0),
        ("/=", 2.0),
        ("%=", 0.0),
        ("**=", 216.0),
        ("<<=", 48.0),
        (">>=", 0.0),
        (">>>=", 0.0),
        ("&=", 2.0),
        ("|=", 7.0),
        ("^=", 5.0),
    ] {
        let source = format!("let x = 6; x {operator} 3");
        assert_eq!(show(&source), number(expected), "{source}");
        let source = format!("let x = 6; x {operator} 3; x");
        assert_eq!(show(&source), number(expected), "{source}");
    }
}

#[test]
fn compound_assignment_uses_the_binary_operator_conversions() {
    assert_eq!(show("let s = 'a'; s += 1; s"), string("a1"));
    assert_eq!(show("let n = 1; n += '1'; n"), string("11"));
    assert_eq!(show("let n = '6'; n -= 2; n"), number(4.0));
    assert_eq!(show("let n = 5; n /= 0; n"), number(f64::INFINITY));
    is_nan("let n = 5; n -= 'x'; n");
}

#[test]
fn compound_assignment_on_members() {
    assert_eq!(show("let o = { n: 1 }; o.n += 4; o.n"), number(5.0));
    assert_eq!(show("let o = { n: 1 }; o.n += 4"), number(5.0));
    assert_eq!(show("let o = { n: 2 }; o['n'] *= 5; o.n"), number(10.0));
    assert_eq!(
        show("let a = [1, 2]; let i = 0; a[i++] += 10; a[0] * 100 + a[1] * 10 + i"),
        number(1121.0)
    );
    assert_eq!(show("let o = {}; o.n += 1; o.n").to_string(), "NaN");
}

#[test]
fn compound_assignment_evaluates_object_key_then_value() {
    assert_eq!(
        show(
            "let log = ''; let o = { n: 1 }; \
             function k() { log += 'k'; return 'n'; } \
             function v() { log += 'v'; return 2; } \
             o[k()] += v(); log + o.n"
        ),
        string("kv3")
    );
}

#[test]
fn logical_assignment_short_circuits() {
    for (source, expected) in [
        ("let a = 0; a ||= 5; a", number(5.0)),
        ("let a = 1; a ||= 5; a", number(1.0)),
        ("let a = 1; a &&= 5; a", number(5.0)),
        ("let a = 0; a &&= 5; a", number(0.0)),
        ("let a = null; a ??= 7; a", number(7.0)),
        ("let a = undefined; a ??= 7; a", number(7.0)),
        ("let a = 0; a ??= 7; a", number(0.0)),
        ("let a = ''; a ??= 7; a", string("")),
        ("let a = 3; a ||= 9", number(3.0)),
        ("let a = 0; a ||= 9", number(9.0)),
        ("let a = 0; a &&= 9", number(0.0)),
        ("let a = 2; a &&= 9", number(9.0)),
        ("let n = 0; let a = 1; a ||= (n = 9); n", number(0.0)),
        ("let n = 0; let a = null; a &&= (n = 9); n", number(0.0)),
        ("let n = 0; let a = 1; a ??= (n = 9); n", number(0.0)),
        ("let n = 0; let a = 0; a ||= (n = 9); n", number(9.0)),
    ] {
        assert_eq!(show(source), expected, "{source}");
    }
}

#[test]
fn logical_assignment_on_members() {
    for (source, expected) in [
        ("let o = {}; o.x ??= 4; o.x", number(4.0)),
        ("let o = { x: 1 }; o.x ??= 4; o.x", number(1.0)),
        ("let o = { x: 0 }; o.x ||= 8", number(8.0)),
        ("let o = { x: 2 }; o.x ||= 8", number(2.0)),
        ("let o = { x: 2 }; o.x &&= 8; o.x", number(8.0)),
        ("let o = { x: 0 }; o.x &&= 8; o.x", number(0.0)),
        (
            "let o = { x: 0 }; let k = 'x'; o[k] ||= 3; o[k]",
            number(3.0),
        ),
        (
            "let o = { x: 5 }; let k = 'x'; o[k] &&= 1; o[k]",
            number(1.0),
        ),
        ("let o = { x: 0 }; let k = 'x'; o[k] &&= 1", number(0.0)),
        ("let o = { x: 5 }; let k = 'x'; o[k] ||= 1", number(5.0)),
        ("let o = {}; let k = 'x'; o[k] ??= 6", number(6.0)),
        ("let o = { x: 1 }; let k = 'x'; o[k] ??= 6", number(1.0)),
        (
            "let a = [0]; let i = 0; a[i++] ||= 4; a[0] * 10 + i",
            number(41.0),
        ),
        (
            "let a = [1]; let i = 0; a[i++] ||= 4; a[0] * 10 + i",
            number(11.0),
        ),
        (
            "let n = 0; let o = { x: 1 }; o.x ||= (n = 1); n",
            number(0.0),
        ),
    ] {
        assert_eq!(show(source), expected, "{source}");
    }
}

#[test]
fn logical_assignment_only_stores_when_it_assigns() {
    assert_eq!(show("const c = 1; c ||= 2"), number(1.0));
    assert_eq!(show("const c = 0; c &&= 2"), number(0.0));
    fails("const c = 0; c ||= 2", "Assignment to constant `c`");
    fails("const c = 1; c += 2", "Assignment to constant `c`");
}

#[test]
fn compound_assignment_reports_reference_errors() {
    fails("missing += 1", "`missing` is not defined");
    fails("missing ||= 1", "`missing` is not defined");
    fails(
        "let n = 1; n.x += 1",
        "Cannot read property `x` of a number",
    );
    fails(
        "let o = null; o.x ??= 1",
        "Cannot read property `x` of null",
    );
    fails("let n = 1; n += missing", "`missing` is not defined");
}

// Nullish coalescing --------------------------------------------------------------------------

#[test]
fn nullish_coalescing_only_replaces_null_and_undefined() {
    for (source, expected) in [
        ("null ?? 1", number(1.0)),
        ("undefined ?? 2", number(2.0)),
        ("0 ?? 3", number(0.0)),
        ("'' ?? 4", string("")),
        ("false ?? 5", boolean(false)),
        ("null ?? undefined ?? 6", number(6.0)),
        ("null ?? null ?? 'last'", string("last")),
        ("null ?? (0 || 7)", number(7.0)),
        ("(null || 0) ?? 8", number(0.0)),
        ("1 ?? missing", number(1.0)),
    ] {
        assert_eq!(show(source), expected, "{source}");
    }
    fails("null ?? missing", "`missing` is not defined");
    fails("missing ?? 1", "`missing` is not defined");
}

#[test]
fn nullish_coalescing_mixed_with_logical_operators_needs_parentheses() {
    for source in ["1 ?? 2 || 3", "1 || 2 ?? 3", "1 && 2 ?? 3", "1 ?? 2 && 3"] {
        let error = syntax_error(source);
        assert!(
            error.message.contains("Cannot mix `??`"),
            "{source}: {error}"
        );
    }
    assert_eq!(show("(1 || 2) ?? 3"), number(1.0));
    assert_eq!(show("1 ?? (2 && 3)"), number(1.0));
}

// typeof, void, unary plus --------------------------------------------------------------------

#[test]
fn typeof_names_each_value_type() {
    for (source, expected) in [
        ("typeof 1", "number"),
        ("typeof 'a'", "string"),
        ("typeof true", "boolean"),
        ("typeof undefined", "undefined"),
        ("typeof null", "object"),
        ("typeof {}", "object"),
        ("typeof []", "object"),
        ("function f() {} typeof f", "function"),
        ("typeof (() => 1)", "function"),
        ("typeof typeof 1", "string"),
        ("typeof (1 + 1)", "number"),
        ("typeof ('a' + 1)", "string"),
    ] {
        assert_eq!(show(source), string(expected), "{source}");
    }
}

#[test]
fn typeof_of_an_undeclared_name_is_undefined_but_other_errors_remain() {
    assert_eq!(show("typeof missing"), string("undefined"));
    assert_eq!(show("typeof missing === 'undefined'"), boolean(true));
    assert_eq!(show("typeof typeof missing"), string("string"));
    fails(
        "typeof x; let x = 1;",
        "Cannot access `x` before initialization",
    );
    fails("typeof missing.x", "`missing` is not defined");
    fails("typeof (missing + 1)", "`missing` is not defined");
}

#[test]
fn void_evaluates_its_operand_and_yields_undefined() {
    assert_eq!(show("void 0"), Value::Undefined);
    assert_eq!(show("void 1 === undefined"), boolean(true));
    assert_eq!(show("let n = 0; void (n = 5); n"), number(5.0));
    fails("void missing", "`missing` is not defined");
}

#[test]
fn unary_plus_converts_with_to_number() {
    assert_eq!(show("+'3'"), number(3.0));
    assert_eq!(show("+true"), number(1.0));
    assert_eq!(show("+null"), number(0.0));
    assert_eq!(show("+''"), number(0.0));
    assert_eq!(show("+' 12 '"), number(12.0));
    assert_eq!(show("+-1"), number(-1.0));
    assert_eq!(show("1 + +'2'"), number(3.0));
    is_nan("+undefined");
    is_nan("+'x'");
}

// delete --------------------------------------------------------------------------------------

#[test]
fn delete_removes_own_properties() {
    assert_eq!(show("let o = { a: 1, b: 2 }; delete o.a"), boolean(true));
    assert_eq!(
        show("let o = { a: 1, b: 2 }; delete o.a; 'a' in o"),
        boolean(false)
    );
    assert_eq!(show("let o = { a: 1, b: 2 }; delete o.a; o.b"), number(2.0));
    assert_eq!(
        show("let o = { a: 1 }; delete o['a']; o.a"),
        Value::Undefined
    );
    assert_eq!(
        show("let o = { a: 1 }; let k = 'a'; delete o[k]; 'a' in o"),
        boolean(false)
    );
    assert_eq!(show("let o = { a: 1 }; delete o.missing"), boolean(true));
    assert_eq!(
        show("let o = { a: 1 }; delete o.a; delete o.a"),
        boolean(true)
    );
}

#[test]
fn delete_on_arrays_leaves_a_hole() {
    assert_eq!(
        show("let a = [1, 2, 3]; delete a[1]; a[1]"),
        Value::Undefined
    );
    assert_eq!(
        show("let a = [1, 2, 3]; delete a[1]; a.length"),
        number(3.0)
    );
    assert_eq!(
        show("let a = [1, 2, 3]; delete a[1]; (1 in a) + ',' + a.length"),
        string("false,3")
    );
    assert_eq!(show("let a = [1]; delete a.length"), boolean(false));
    assert_eq!(show("let a = [1]; delete a.length; a.length"), number(1.0));
}

#[test]
fn delete_of_other_operands() {
    assert_eq!(show("delete 1"), boolean(true));
    assert_eq!(show("let n = 0; delete (n = 1); n"), number(1.0));
    assert_eq!(show("delete 'abc'.length"), boolean(false));
    assert_eq!(show("delete 'abc'.x"), boolean(true));
    assert_eq!(show("delete (1).x"), boolean(true));
    fails("delete missing.x", "`missing` is not defined");
}

#[test]
fn delete_of_null_or_undefined_is_a_runtime_error() {
    fails("delete null.x", "Cannot delete property `x` of null");
    fails(
        "let u; delete u.x",
        "Cannot delete property `x` of undefined",
    );
    fails(
        "let u; delete u['k']",
        "Cannot delete property `k` of undefined",
    );
}

#[test]
fn delete_of_a_bare_name_or_optional_chain_is_a_syntax_error() {
    let error = syntax_error("let x = 1; delete x;");
    assert!(error.message.contains("unqualified identifier"), "{error}");
    let error = syntax_error("let a = {}; delete a?.b;");
    assert!(error.message.contains("optional chain"), "{error}");
}

// in and instanceof ---------------------------------------------------------------------------

#[test]
fn in_tests_for_properties() {
    for (source, expected) in [
        ("'a' in { a: 1 }", true),
        ("'b' in { a: 1 }", false),
        ("1 in { 1: 'x' }", true),
        ("'1' in { 1: 'x' }", true),
        ("0 in [5]", true),
        ("1 in [5]", false),
        ("'length' in []", true),
        ("'push' in []", true),
        ("'nope' in []", false),
        ("let k = 'a'; k in { a: undefined }", true),
        ("!('a' in { a: 1 })", false),
        ("'a' in { a: 1 } && 'b' in { b: 1 }", true),
        ("1 + 1 in [0, 1, 2]", true),
    ] {
        assert_eq!(show(source), boolean(expected), "{source}");
    }
}

#[test]
fn in_requires_an_object_on_the_right() {
    fails(
        "'x' in 5",
        "Cannot use 'in' operator to search for `x` in `5`",
    );
    fails("'a' in 'abc'", "Cannot use 'in' operator");
    fails("'a' in null", "Cannot use 'in' operator");
    fails("'a' in undefined", "Cannot use 'in' operator");
    fails("'a' in true", "Cannot use 'in' operator");
}

#[test]
fn instanceof_rejects_non_constructors_in_both_engines() {
    fails(
        "({}) instanceof 1",
        "Right-hand side of 'instanceof' is not callable",
    );
    fails(
        "1 instanceof ({})",
        "Right-hand side of 'instanceof' is not callable",
    );
    fails(
        "1 instanceof null",
        "Right-hand side of 'instanceof' is not callable",
    );
    fails(
        "1 instanceof undefined",
        "Right-hand side of 'instanceof' is not callable",
    );
}

#[test]
fn classes_support_in_and_instanceof_on_the_vm() {
    let classes = "class A { m() { return 1; } } class B extends A {} ";
    for (source, expected) in [
        ("const b = new B(); b instanceof B", true),
        ("const b = new B(); b instanceof A", true),
        ("const a = new A(); a instanceof B", false),
        ("({}) instanceof A", false),
        ("1 instanceof A", false),
        ("'x' instanceof A", false),
        ("null instanceof A", false),
        ("[] instanceof A", false),
        ("'m' in new A()", true),
        ("'m' in new B()", true),
        ("'zzz' in new B()", false),
        ("const a = new A(); a.own = 1; 'own' in a", true),
        ("!(new A() instanceof B)", true),
    ] {
        let source = format!("{classes}{source}");
        assert_eq!(run_vm(&source), Ok(boolean(expected)), "{source}");
    }
}

#[test]
fn instanceof_on_a_function_without_a_prototype_is_an_error() {
    let source = "const f = () => 1; ({}) instanceof f";
    let error = run_vm(source).expect_err(source);
    assert_eq!(error.category, JsErrorCategory::Runtime);
    assert!(error.message.contains("non-object prototype"), "{error:?}");
}

// Exponent ------------------------------------------------------------------------------------

#[test]
fn exponent_is_right_associative_and_binds_above_multiplication() {
    for (source, expected) in [
        ("2 ** 3", 8.0),
        ("2 ** 3 ** 2", 512.0),
        ("(2 ** 3) ** 2", 64.0),
        ("2 * 3 ** 2", 18.0),
        ("(-2) ** 2", 4.0),
        ("-(2 ** 2)", -4.0),
        ("2 ** -1", 0.5),
        ("2 ** 0", 1.0),
        ("0 ** 0", 1.0),
        ("(0 / 0) ** 0", 1.0),
        ("let x = 3; x **= 2; x", 9.0),
        ("let x = 2; x **= 3 ** 2; x", 512.0),
    ] {
        assert_eq!(show(source), number(expected), "{source}");
    }
}

#[test]
fn exponent_follows_javascript_nan_rules() {
    is_nan("1 ** (1 / 0)");
    is_nan("(-1) ** (1 / 0)");
    is_nan("2 ** (0 / 0)");
    is_nan("(0 / 0) ** 2");
    assert_eq!(show("2 ** (1 / 0)"), number(f64::INFINITY));
    assert_eq!(show("2 ** (-1 / 0)"), number(0.0));
}

#[test]
fn a_unary_operator_before_exponent_needs_parentheses() {
    for source in [
        "-2 ** 2",
        "+2 ** 2",
        "!1 ** 2",
        "~1 ** 2",
        "typeof 1 ** 2",
        "void 1 ** 2",
    ] {
        let error = syntax_error(source);
        assert!(
            error.message.contains("needs parentheses"),
            "{source}: {error}"
        );
    }
}

// Template literals ---------------------------------------------------------------------------

#[test]
fn template_literals_join_text_and_substitutions() {
    for (source, expected) in [
        ("`abc`", "abc"),
        ("``", ""),
        ("`a${1}b${2}c`", "a1b2c"),
        ("`${1}${2}`", "12"),
        ("`${'x'}`", "x"),
        ("`${1 + 2}`", "3"),
        ("`${null} ${undefined} ${true}`", "null undefined true"),
        ("`a${`b${1}`}c`", "ab1c"),
        ("`${1, 2}`", "2"),
        ("`${ {a: 1}.a }`", "1"),
        ("`${'}'}`", "}"),
        ("`${{}}`", "[object Object]"),
        ("`${[1, 2]}`", "1,2"),
        ("`${0 / 0}`", "NaN"),
        ("`${1 / 0}`", "Infinity"),
        ("`${-0}`", "0"),
        ("let n = 3; `n=${n}`", "n=3"),
        ("let a = 1; let b = 2; `${a}+${b}=${a + b}`", "1+2=3"),
        ("`${true ? 'y' : 'n'}`", "y"),
        ("`${(x => x * 2)(4)}`", "8"),
    ] {
        assert_eq!(show(source), string(expected), "{source}");
    }
}

#[test]
fn template_literals_decode_escapes_and_keep_line_breaks() {
    assert_eq!(show("`a\nb`"), string("a\nb"));
    assert_eq!(show("`a\r\nb`"), string("a\nb"));
    assert_eq!(show(r"`A\n`"), string("A\n"));
    assert_eq!(show(r"`\${x}`"), string("${x}"));
    assert_eq!(show(r"`\``"), string("`"));
    assert_eq!(show("`line one\\\nline two`"), string("line oneline two"));
    assert_eq!(show("`$`"), string("$"));
    assert_eq!(show("`$a`"), string("$a"));
    assert_eq!(show("`{}`"), string("{}"));
}

#[test]
fn template_substitutions_run_in_order_and_report_errors() {
    assert_eq!(
        show("let log = ''; function t(x) { log += x; return x; } `${t('a')}${t('b')}`; log"),
        string("ab")
    );
    fails("`${missing}`", "`missing` is not defined");
    fails("`a${1}${missing}`", "`missing` is not defined");
}

#[test]
fn template_syntax_errors_are_reported_at_the_template() {
    for source in [
        "`abc", "`${1", "`${`", "`${1 +}`", "`${}`", "`${1 2}`", "`\\1`",
    ] {
        syntax_error(source);
    }
    let error = parse("let a = 1;\n  `x${1 +}`").unwrap_err();
    assert_eq!((error.line, error.column), (Some(2), Some(3)));
    let error = syntax_error("let tag = 1; tag`x`");
    assert!(error.message.contains("Tagged template"), "{error}");
}

// Spread --------------------------------------------------------------------------------------

#[test]
fn array_spread_expands_arrays_and_strings() {
    for (source, expected) in [
        ("'' + [...[1, 2], 3]", "1,2,3"),
        ("'' + [0, ...[1, 2], ...'ab']", "0,1,2,a,b"),
        ("'' + [...[]]", ""),
        ("'' + [...'']", ""),
        ("'' + [...[...[1], ...[2]]]", "1,2"),
        ("'' + [1, , ...[3]]", "1,,3"),
        ("let a = [1, 2]; '' + [...a, ...a]", "1,2,1,2"),
        ("let a = [1]; let b = [...a]; b[0] = 9; '' + a + b", "19"),
    ] {
        assert_eq!(show(source), string(expected), "{source}");
    }
    assert_eq!(show("[...[1, , 3]].length"), number(3.0));
    assert_eq!(show("[...[1, , 3]][1]"), Value::Undefined);
    assert_eq!(show("[...'héllo'].length"), number(5.0));
}

#[test]
fn spread_calls_expand_arguments_in_order() {
    let sum = "function sum(a, b, c) { return a + b + c; } ";
    for (source, expected) in [
        ("sum(...[1, 2, 3])", number(6.0)),
        ("sum(1, ...[2, 3])", number(6.0)),
        ("sum(...[1], ...[2], 3)", number(6.0)),
        ("sum(...'ab', 'c')", string("abc")),
        ("sum(...[1, 2, 3, 4])", number(6.0)),
        ("let o = { sum: sum }; o.sum(...[1, 2, 3])", number(6.0)),
        ("let o = { sum: sum }; o['sum'](...[1, 2], 3)", number(6.0)),
    ] {
        let source = format!("{sum}{source}");
        assert_eq!(show(&source), expected, "{source}");
    }
    is_nan("function sum(a, b, c) { return a + b + c; } sum(...[1, 2])");
    assert_eq!(
        show(
            "let log = ''; function t(x) { log += x; return [x]; } function f() {} f(...t('a'), ...t('b')); log"
        ),
        string("ab")
    );
}

#[test]
fn object_spread_copies_own_properties_in_order() {
    for (source, expected) in [
        (
            "let o = { a: 1, b: 2 }; let c = { ...o, b: 3, d: 4 }; c.a * 100 + c.b * 10 + c.d",
            134.0,
        ),
        ("let o = { a: 1 }; let c = { b: 2, ...o }; c.a + c.b", 3.0),
        ("let o = { a: 1 }; let c = { a: 5, ...o }; c.a", 1.0),
        (
            "let c = { ...null, ...undefined, ...5, ...true, a: 1 }; c.a",
            1.0,
        ),
        ("let c = { ...[7, 8] }; c[0] + c[1]", 15.0),
        ("let o = { a: 1 }; let c = { ...o }; c.a = 2; o.a", 1.0),
    ] {
        assert_eq!(show(source), number(expected), "{source}");
    }
    assert_eq!(show("({ ...'ab' })[1]"), string("b"));
    assert_eq!(show("typeof { ...[1] }.length"), string("undefined"));
    assert_eq!(
        show("let k = 'x'; ({ ...{ x: 1 }, [k]: 2 }).x"),
        number(2.0)
    );
}

#[test]
fn spread_of_a_non_iterable_is_a_runtime_error() {
    fails("[...1]", "`1` is not iterable");
    fails("[...null]", "`null` is not iterable");
    fails("[...undefined]", "`undefined` is not iterable");
    fails("[...{}]", "is not iterable");
    fails("function f() {} f(...1)", "`1` is not iterable");
    fails("function f() {} f(...missing)", "`missing` is not defined");
}

#[test]
fn spread_cannot_be_used_as_an_expression_or_a_parameter() {
    for source in [
        "...[1]",
        "let a = ...[1];",
        "(...[1])",
        "function f(...a) {}",
    ] {
        syntax_error(source);
    }
}

#[test]
fn new_accepts_spread_arguments_on_the_vm() {
    let source = "class P { constructor(a, b) { this.s = a + b; } } new P(...[1, 2]).s";
    assert_eq!(run_vm(source), Ok(number(3.0)));
    let source = "class P { constructor(a, b) { this.s = a + b; } } new P(1, ...[2]).s";
    assert_eq!(run_vm(source), Ok(number(3.0)));
}

// Optional chaining ---------------------------------------------------------------------------

#[test]
fn optional_member_access_short_circuits_on_nullish_bases() {
    for (source, expected) in [
        ("let o = { a: { b: 1 } }; o?.a?.b", number(1.0)),
        ("let o = null; o?.a", Value::Undefined),
        ("let o; o?.a", Value::Undefined),
        ("let o = {}; o.a?.b", Value::Undefined),
        ("let o = null; o?.a.b.c", Value::Undefined),
        ("let o = {}; o?.a?.b.c", Value::Undefined),
        ("let o = { a: 0 }; o?.a", number(0.0)),
        ("let o = { a: 1 }; o?.['a']", number(1.0)),
        ("let o = null; o?.['a']", Value::Undefined),
        ("let a = [1, 2]; a?.[1]", number(2.0)),
        ("let a = null; a?.[0]", Value::Undefined),
        ("undefined?.x", Value::Undefined),
        ("null?.[0]", Value::Undefined),
        ("let o = null; o?.a ?? 'dflt'", string("dflt")),
        ("let o = { a: 0 }; o?.a ?? 5", number(0.0)),
        ("let o = null; o?.a ? 1 : 2", number(2.0)),
        ("let o = { a: { b: 7 } }; o?.a.b", number(7.0)),
    ] {
        assert_eq!(show(source), expected, "{source}");
    }
}

#[test]
fn optional_chain_does_not_evaluate_skipped_operands() {
    assert_eq!(show("let n = 0; let o = null; o?.[n = 1]; n"), number(0.0));
    assert_eq!(show("let n = 0; let o = null; o?.a[n = 1]; n"), number(0.0));
    assert_eq!(show("let n = 0; let g = null; g?.(n = 1); n"), number(0.0));
    assert_eq!(show("let n = 0; let o = null; o?.f(n = 1); n"), number(0.0));
    assert_eq!(show("let n = 0; let o = {}; o.f?.(n = 1); n"), number(0.0));
    assert_eq!(
        show("let n = 0; let o = { f: null }; o.f?.(n = 1); n"),
        number(0.0)
    );
    assert_eq!(
        show("let n = 0; let o = { a: 1 }; o?.[n = 1]; n"),
        number(1.0)
    );
}

#[test]
fn optional_calls() {
    let f = "function f(x) { return x + 1; } ";
    for (source, expected) in [
        ("f?.(1)", number(2.0)),
        ("let g = null; g?.(1)", Value::Undefined),
        ("let g; g?.(1)", Value::Undefined),
        ("let o = { f: f }; o.f?.(2)", number(3.0)),
        ("let o = {}; o.f?.(2)", Value::Undefined),
        ("let o = { f: f }; o?.f(2)", number(3.0)),
        ("let o = null; o?.f(2)", Value::Undefined),
        ("let o = { f: f }; o?.['f'](2)", number(3.0)),
        ("let o = { f: f }; o.f?.(2) + 1", number(4.0)),
        ("let o = { g: { f: f } }; o?.g.f(1)", number(2.0)),
        ("let o = { g: null }; o?.g?.f(1)", Value::Undefined),
        ("function h() { return null; } h()?.x", Value::Undefined),
        ("function h() { return { x: 3 }; } h()?.x", number(3.0)),
        ("function h() { return f; } h()?.(4)", number(5.0)),
    ] {
        let source = format!("{f}{source}");
        assert_eq!(show(&source), expected, "{source}");
    }
}

#[test]
fn optional_chain_still_reports_non_nullish_errors() {
    fails(
        "let o = {}; o?.a.b",
        "Cannot read property `b` of undefined",
    );
    fails(
        "let o = { a: null }; o?.a.b",
        "Cannot read property `b` of null",
    );
    fails("let n = 1; n?.a.b", "Cannot read property `a` of a number");
    fails("missing?.a", "`missing` is not defined");
    fails_at_run_time("let o = { f: 1 }; o.f?.()");
    fails_at_run_time("let o = { f: 1 }; o?.f()");
    fails_at_run_time("let g = 1; g?.()");
}

#[test]
fn a_parenthesized_chain_ends_the_short_circuit() {
    fails(
        "let o = null; (o?.a).b",
        "Cannot read property `b` of undefined",
    );
    assert_eq!(show("let o = null; (o?.a)?.b"), Value::Undefined);
}

#[test]
fn optional_chain_syntax_errors() {
    for source in [
        "let a = {}; a?.b = 1;",
        "let a = {}; a?.b++;",
        "let a = {}; a?.b += 1;",
        "let a = {}; new a?.b();",
        "let a = {}; a?.b`x`;",
        "let a = {}; a?.;",
    ] {
        syntax_error(source);
    }
}

// Evaluation order and reference errors across forms ------------------------------------------

#[test]
fn forms_compose_and_keep_evaluation_order() {
    assert_eq!(
        show("let log = ''; function t(x) { log += x; return x; } t(0) || t(1) ? t(2) : t(3); log"),
        string("012")
    );
    assert_eq!(
        show("let log = ''; function t(x) { log += x; return x; } t(null) ?? t(1) ?? t(2); log"),
        string("null1")
    );
    assert_eq!(
        show("let o = { n: 0 }; let i = 0; o.n += i++ + 1; o.n * 10 + i"),
        number(11.0)
    );
    assert_eq!(show("let a = 1; let b = 2; a++ + ++b * 2"), number(7.0));
    assert_eq!(
        show("let o = { a: [1, 2, 3] }; `${o.a.length}:${[...o.a].length}:${o?.a?.[1]}`"),
        string("3:3:2")
    );
}

#[test]
fn every_form_reports_the_same_reference_errors_in_both_engines() {
    for source in [
        "missing ? 1 : 2",
        "(missing, 1)",
        "missing++",
        "--missing",
        "missing += 1",
        "missing ??= 1",
        "typeof missing.x",
        "void missing",
        "delete missing.x",
        "'a' in missing",
        "missing instanceof Object",
        "`${missing}`",
        "[...missing]",
        "missing?.x",
        "missing ** 2",
    ] {
        fails(source, "`missing` is not defined");
    }
}

// Differential coverage of operands -----------------------------------------------------------

const OPERANDS: &[&str] = &[
    "0",
    "(-0)",
    "1",
    "(-1)",
    "2.5",
    "(1 / 0)",
    "(-1 / 0)",
    "(0 / 0)",
    "null",
    "undefined",
    "true",
    "false",
    "''",
    "'0'",
    "'1'",
    "'abc'",
    "'0x10'",
    "'Infinity'",
];

#[test]
fn exponent_agrees_for_every_operand_pair() {
    for left in OPERANDS {
        for right in OPERANDS {
            let source = format!("{left} ** {right}");
            assert_eq!(
                outcome(&run_tree(&source)),
                outcome(&run_vm(&source)),
                "{source}"
            );
        }
    }
}

#[test]
fn nullish_and_in_agree_for_every_operand_pair() {
    for left in OPERANDS {
        for right in OPERANDS {
            for source in [
                format!("{left} ?? {right}"),
                format!("{left} in {right}"),
                format!("{left} instanceof {right}"),
            ] {
                assert_eq!(
                    outcome(&run_tree(&source)),
                    outcome(&run_vm(&source)),
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn new_unary_operators_agree_for_every_operand() {
    for operand in OPERANDS {
        for source in [
            format!("+{operand}"),
            format!("typeof {operand}"),
            format!("void {operand}"),
            format!("delete {operand}"),
            format!("let x = {operand}; x++"),
            format!("let x = {operand}; --x; x"),
            format!("`${{{operand}}}`"),
            format!("{operand} ? 'yes' : 'no'"),
            format!("[...{operand}]"),
            format!("({{ ...{operand} }}) instanceof ({operand})"),
        ] {
            assert_eq!(
                outcome(&run_tree(&source)),
                outcome(&run_vm(&source)),
                "{source}"
            );
        }
    }
}

#[test]
fn compound_assignment_agrees_for_every_operand_pair() {
    for operator in [
        "+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", ">>>=", "&=", "|=", "^=", "&&=", "||=",
        "??=",
    ] {
        for left in OPERANDS {
            for right in OPERANDS {
                let source = format!("let x = {left}; x {operator} {right}; x");
                assert_eq!(
                    outcome(&run_tree(&source)),
                    outcome(&run_vm(&source)),
                    "{source}"
                );
                let source = format!("let o = {{ x: {left} }}; o.x {operator} {right}");
                assert_eq!(
                    outcome(&run_tree(&source)),
                    outcome(&run_vm(&source)),
                    "{source}"
                );
            }
        }
    }
}

// No evaluator fallback -----------------------------------------------------------------------

#[test]
fn every_supported_form_compiles_and_runs_in_bytecode_mode() {
    for source in [
        "1 ? 2 : 3",
        "(1, 2)",
        "let i = 0; i++",
        "let i = 0; ++i",
        "let o = { n: 0 }; o.n++",
        "let o = { n: 0 }; o['n']--",
        "let x = 1; x += 1",
        "let o = { n: 1 }; o.n **= 2",
        "let x = null; x ??= 1",
        "let x = 0; x ||= 1",
        "let x = 1; x &&= 2",
        "null ?? 1",
        "typeof 1",
        "typeof missing",
        "void 0",
        "+'1'",
        "let o = { a: 1 }; delete o.a",
        "let o = { a: 1 }; delete o['a']",
        "'a' in { a: 1 }",
        "class A {} new A() instanceof A",
        "`a${1}`",
        "[...[1]]",
        "function f() {} f(...[1])",
        "({ ...{ a: 1 } })",
        "let o = null; o?.a",
        "let o = null; o?.[0]",
        "let f = null; f?.()",
        "2 ** 3",
    ] {
        let program = parse(source).unwrap_or_else(|error| panic!("`{source}`: {error}"));
        compile(&program).unwrap_or_else(|error| panic!("`{source}` did not compile: {error}"));
        let bytecode = eval_with_mode(source, ExecutionMode::Bytecode);
        assert!(
            bytecode.is_ok(),
            "`{source}` failed in bytecode mode: {bytecode:?}"
        );
    }
}

#[test]
fn bytecode_mode_agrees_with_interpreter_mode() {
    for source in [
        "let n = 0; for (let i = 0; i < 4; i++) { n += i; } n",
        "let o = { a: 1 }; o.a ??= 2; o.b ??= 3; o.a * 10 + o.b",
        "let s = ''; for (let i = 0; i < 3; i++) { s += `${i},`; } s",
        "let a = [3, 1, 2]; `${[...a, ...a].length} ${typeof a} ${0 in a}`",
        "const pick = (o, k) => o?.[k] ?? 'none'; pick({ a: 1 }, 'a') + pick(null, 'a') + pick({}, 'b')",
    ] {
        assert_eq!(
            outcome(&eval_with_mode(source, ExecutionMode::Bytecode)),
            outcome(&eval_with_mode(source, ExecutionMode::Interpreter)),
            "{source}"
        );
    }
}

// Lexer ---------------------------------------------------------------------------------------

fn ident(name: &str) -> Token {
    Token::Identifier(name.into())
}

#[test]
fn lexer_recognizes_the_new_operators() {
    for (source, expected) in [
        ("?", Token::Question),
        ("?.", Token::QuestionDot),
        ("??", Token::NullishCoalesce),
        ("??=", Token::NullishAssign),
        ("++", Token::Increment),
        ("--", Token::Decrement),
        ("**", Token::StarStar),
        ("**=", Token::StarStarAssign),
        ("+=", Token::PlusAssign),
        ("-=", Token::MinusAssign),
        ("*=", Token::StarAssign),
        ("/=", Token::SlashAssign),
        ("%=", Token::PercentAssign),
        ("<<=", Token::ShiftLeftAssign),
        (">>=", Token::ShiftRightAssign),
        (">>>=", Token::UnsignedShiftRightAssign),
        ("&=", Token::BitAndAssign),
        ("|=", Token::BitOrAssign),
        ("^=", Token::BitXorAssign),
        ("&&=", Token::AndAssign),
        ("||=", Token::OrAssign),
        ("...", Token::Ellipsis),
        ("typeof", Token::Typeof),
        ("void", Token::Void),
        ("delete", Token::Delete),
        ("in", Token::In),
        ("instanceof", Token::InstanceOf),
    ] {
        assert_eq!(tokenize(source), vec![expected], "{source}");
    }
}

#[test]
fn lexer_takes_the_longest_operator_and_keeps_neighbors() {
    assert_eq!(
        tokenize("a?.b"),
        vec![ident("a"), Token::QuestionDot, ident("b")]
    );
    assert_eq!(
        tokenize("a?.5:1"),
        vec![
            ident("a"),
            Token::Question,
            Token::Number(0.5),
            Token::Colon,
            Token::Number(1.0)
        ]
    );
    assert_eq!(
        tokenize("a+++b"),
        vec![ident("a"), Token::Increment, Token::Plus, ident("b")]
    );
    assert_eq!(tokenize(".."), vec![Token::Dot, Token::Dot]);
    assert_eq!(tokenize("a ? b : c")[1], Token::Question);
    assert_eq!(tokenize("x.in"), vec![ident("x"), Token::Dot, Token::In]);
    assert_eq!(tokenize("instanceofx"), vec![ident("instanceofx")]);
}

#[test]
fn lexer_splits_templates_into_text_and_substitutions() {
    assert_eq!(
        tokenize("`a${b}c`"),
        vec![Token::Template {
            quasis: vec!["a".into(), "c".into()],
            expressions: vec![vec![ident("b")]],
        }]
    );
    assert_eq!(
        tokenize("`${ {a: 1}.a }`"),
        vec![Token::Template {
            quasis: vec![String::new(), String::new()],
            expressions: vec![vec![
                Token::LeftBrace,
                ident("a"),
                Token::Colon,
                Token::Number(1.0),
                Token::RightBrace,
                Token::Dot,
                ident("a"),
            ]],
        }]
    );
    assert_eq!(
        tokenize("`a${`b`}` + 1"),
        vec![
            Token::Template {
                quasis: vec!["a".into(), String::new()],
                expressions: vec![vec![Token::Template {
                    quasis: vec!["b".into()],
                    expressions: vec![],
                }]],
            },
            Token::Plus,
            Token::Number(1.0),
        ]
    );
}

#[test]
fn lexer_reports_malformed_templates() {
    for source in ["`abc", "`${`", "`a${b", "`\\1`", "`\\u{110000}`"] {
        let tokens = tokenize(source);
        assert!(
            tokens
                .iter()
                .any(|token| matches!(token, Token::Invalid(_))),
            "{source}: {tokens:?}"
        );
    }
    assert_eq!(
        tokenize("`abc"),
        vec![Token::Invalid("unterminated template literal".into())]
    );
    let deep = format!("{}1{}", "`${".repeat(40), "}`".repeat(40));
    assert!(format!("{:?}", tokenize(&deep)).contains("Invalid"));
}

// Parser --------------------------------------------------------------------------------------

fn id(name: &str) -> Expr {
    Expr::Identifier(name.into())
}

fn expression(source: &str) -> Expr {
    let program = parse(source).unwrap_or_else(|error| panic!("`{source}`: {error}"));
    match program.body.as_slice() {
        [js::Statement::Expression(expression)] => expression.clone(),
        other => panic!("`{source}` is not one expression statement: {other:?}"),
    }
}

#[test]
fn parser_builds_the_new_expression_nodes() {
    assert_eq!(
        expression("a ? b : c ? d : e"),
        Expr::Conditional {
            test: Box::new(id("a")),
            consequent: Box::new(id("b")),
            alternate: Box::new(Expr::Conditional {
                test: Box::new(id("c")),
                consequent: Box::new(id("d")),
                alternate: Box::new(id("e")),
            }),
        }
    );
    assert_eq!(
        expression("a, b, c"),
        Expr::Sequence(vec![id("a"), id("b"), id("c")])
    );
    assert_eq!(
        expression("a ?? b"),
        Expr::Logical {
            left: Box::new(id("a")),
            operator: LogicalOperator::Nullish,
            right: Box::new(id("b")),
        }
    );
    assert_eq!(
        expression("a += b"),
        Expr::CompoundAssign {
            target: Box::new(id("a")),
            operator: AssignmentOperator::Binary(BinaryOperator::Add),
            value: Box::new(id("b")),
        }
    );
    assert_eq!(
        expression("a ??= b"),
        Expr::CompoundAssign {
            target: Box::new(id("a")),
            operator: AssignmentOperator::Logical(LogicalOperator::Nullish),
            value: Box::new(id("b")),
        }
    );
    assert_eq!(
        expression("2 ** 3 ** 2"),
        Expr::Binary {
            left: Box::new(Expr::Number(2.0)),
            operator: BinaryOperator::Exponent,
            right: Box::new(Expr::Binary {
                left: Box::new(Expr::Number(3.0)),
                operator: BinaryOperator::Exponent,
                right: Box::new(Expr::Number(2.0)),
            }),
        }
    );
    assert_eq!(
        expression("a in b"),
        Expr::Binary {
            left: Box::new(id("a")),
            operator: BinaryOperator::In,
            right: Box::new(id("b")),
        }
    );
}

#[test]
fn parser_wraps_each_optional_chain_once() {
    let member = |object: Expr, name: &str| Expr::Member {
        object: Box::new(object),
        property: MemberProperty::Named(name.into()),
    };
    assert_eq!(
        expression("a?.b.c"),
        Expr::OptionalChain(Box::new(member(
            Expr::OptionalMember {
                object: Box::new(id("a")),
                property: MemberProperty::Named("b".into()),
            },
            "c"
        )))
    );
    assert_eq!(
        expression("a?.(1)"),
        Expr::OptionalChain(Box::new(Expr::OptionalCall {
            callee: Box::new(id("a")),
            arguments: vec![Expr::Number(1.0)],
        }))
    );
    assert_eq!(
        expression("a?.[k]"),
        Expr::OptionalChain(Box::new(Expr::OptionalMember {
            object: Box::new(id("a")),
            property: MemberProperty::Computed(Box::new(id("k"))),
        }))
    );
    // A chain with several `?.` is still one chain.
    let Expr::OptionalChain(inner) = expression("a?.b?.c") else {
        panic!("expected an optional chain");
    };
    assert!(matches!(*inner, Expr::OptionalMember { .. }));
    // No `?.`, no wrapper.
    assert!(matches!(expression("a.b.c"), Expr::Member { .. }));
}

#[test]
fn parser_builds_update_template_and_spread_nodes() {
    use js::ast::UpdateOperator;
    assert_eq!(
        expression("a++"),
        Expr::Update {
            target: Box::new(id("a")),
            operator: UpdateOperator::Increment,
            prefix: false,
        }
    );
    assert_eq!(
        expression("--a.b"),
        Expr::Update {
            target: Box::new(Expr::Member {
                object: Box::new(id("a")),
                property: MemberProperty::Named("b".into()),
            }),
            operator: UpdateOperator::Decrement,
            prefix: true,
        }
    );
    assert_eq!(
        expression("`a${b}c${d}`"),
        Expr::Template {
            quasis: vec!["a".into(), "c".into(), String::new()],
            expressions: vec![id("b"), id("d")],
        }
    );
    assert_eq!(
        expression("[...a, b]"),
        Expr::Array(vec![Some(Expr::Spread(Box::new(id("a")))), Some(id("b"))])
    );
    assert_eq!(
        expression("f(...a)"),
        Expr::Call {
            callee: Box::new(id("f")),
            arguments: vec![Expr::Spread(Box::new(id("a")))],
        }
    );
    assert!(matches!(expression("delete a.b"), Expr::Delete(_)));
}

#[test]
fn parser_rejects_unsupported_forms_with_specific_messages() {
    for (source, message) in [
        (
            "let r = /a/;",
            "Regular expression literals are not supported",
        ),
        ("/=/", "Regular expression literals are not supported"),
        (
            "let t = 1; t`x`",
            "Tagged template literals are not supported",
        ),
        ("-1 ** 2", "needs parentheses"),
        ("1 ?? 2 || 3", "Cannot mix `??`"),
        (
            "1++",
            "Invalid left-hand side expression in postfix operation",
        ),
        (
            "++1",
            "Invalid left-hand side expression in prefix operation",
        ),
        (
            "1 += 2",
            "Assignment target must be an identifier or member expression",
        ),
        (
            "let x = 1; delete x",
            "Delete of an unqualified identifier is not supported",
        ),
        (
            "a?.b = 1",
            "Assignment target must be an identifier or member expression",
        ),
    ] {
        let error = parse(source).expect_err(source);
        assert!(error.message.contains(message), "{source}: {error}");
        assert!(error.line.is_some(), "{source} has no location");
    }
}

#[test]
fn new_keywords_are_still_valid_property_names() {
    assert_eq!(
        show("let o = { typeof: 1, void: 2, in: 3 }; o.typeof + o.void + o.in"),
        number(6.0)
    );
    assert_eq!(
        show("let o = { delete: 4, instanceof: 5 }; o.delete + o.instanceof"),
        number(9.0)
    );
}

#[test]
fn deeply_nested_new_forms_hit_the_parser_limits_instead_of_the_stack() {
    for source in [
        format!("{}1{}", "(1, ".repeat(2000), ")".repeat(2000)),
        format!("{}1{}", "1 ? ".repeat(2000), " : 1".repeat(2000)),
        format!("{}1{}", "`${".repeat(2000), "}`".repeat(2000)),
        format!("a{}", "?.b".repeat(3000)),
        format!("{}1", "typeof ".repeat(3000)),
        format!("{}a", "++".repeat(3000)),
    ] {
        assert!(
            parse(&source).is_err(),
            "{}",
            &source[..source.len().min(40)]
        );
    }
}

// Bytecode shape and stack effects ------------------------------------------------------------

fn chunk(code: Vec<Instruction>, constants: Vec<Value>) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.constants = constants;
    for instruction in code {
        chunk.emit(instruction);
    }
    chunk
}

fn execute(code: Vec<Instruction>, constants: Vec<Value>) -> Result<Value, JsError> {
    Vm::new().execute(&chunk(code, constants))
}

#[test]
fn conditional_lowers_to_a_branch_that_pops_the_test() {
    assert_eq!(
        compile_source("true ? 1 : 2").code,
        vec![
            Instruction::EnterScope(0),
            Instruction::Constant(0),
            Instruction::JumpIfFalse(6),
            Instruction::Pop,
            Instruction::Constant(1),
            Instruction::Jump(8),
            Instruction::Pop,
            Instruction::Constant(2),
            Instruction::Return,
        ]
    );
}

#[test]
fn nullish_coalescing_lowers_without_changing_and_or() {
    assert_eq!(
        compile_source("null ?? 1").code,
        vec![
            Instruction::EnterScope(0),
            Instruction::Constant(0),
            Instruction::JumpIfNullish(4),
            Instruction::Jump(6),
            Instruction::Pop,
            Instruction::Constant(1),
            Instruction::Return,
        ]
    );
    assert_eq!(
        compile_source("0 || 1").code,
        vec![
            Instruction::EnterScope(0),
            Instruction::Constant(0),
            Instruction::JumpIfTrue(5),
            Instruction::Pop,
            Instruction::Constant(1),
            Instruction::Return,
        ]
    );
}

#[test]
fn templates_lower_to_string_additions() {
    let code = compile_source("`a${1}b`").code;
    assert_eq!(
        code,
        vec![
            Instruction::EnterScope(0),
            Instruction::Constant(0),
            Instruction::Constant(1),
            Instruction::Binary(BinaryOperator::Add),
            Instruction::Constant(2),
            Instruction::Binary(BinaryOperator::Add),
            Instruction::Return,
        ]
    );
}

#[test]
fn optional_chain_lowers_to_nullish_jumps_and_a_landing_pad() {
    // The normal path jumps over the pad; the pad pops the checked base and
    // pushes `undefined`.
    assert_eq!(
        compile_source("null?.a").code,
        vec![
            Instruction::EnterScope(0),
            Instruction::Constant(0),
            Instruction::JumpIfNullish(5),
            Instruction::LoadProperty(1),
            Instruction::Jump(8),
            Instruction::Pop,
            Instruction::Constant(2),
            Instruction::Jump(8),
            Instruction::Return,
        ]
    );
    assert_eq!(run_vm("null?.a"), Ok(Value::Undefined));
}

#[test]
fn spread_calls_use_the_array_based_call_instructions() {
    let code = compile_source("function f() {} f(...[1])").code;
    assert!(code.contains(&Instruction::CallSpread), "{code:?}");
    assert!(code.contains(&Instruction::ArrayExtend), "{code:?}");
    let code = compile_source("let o = { f: 1 }; o.f(...[1])").code;
    assert!(code.contains(&Instruction::CallMethodSpread), "{code:?}");
    let code = compile_source("class A {} new A(...[])").code;
    assert!(code.contains(&Instruction::NewSpread), "{code:?}");
    let code = compile_source("function f() {} f(1, 2)").code;
    assert!(!code.contains(&Instruction::CallSpread), "{code:?}");
    assert!(code.contains(&Instruction::Call(2)), "{code:?}");
}

#[test]
fn compound_assignment_on_a_computed_member_duplicates_the_reference() {
    let code = compile_source("let o = {}; o['k'] += 1").code;
    assert!(code.contains(&Instruction::Dup2), "{code:?}");
    let code = compile_source("let o = {}; o.k += 1").code;
    assert!(!code.contains(&Instruction::Dup2), "{code:?}");
    let code = compile_source("let o = { k: 0 }; o['k']++").code;
    assert!(code.contains(&Instruction::MoveTop(3)), "{code:?}");
    let code = compile_source("let o = { k: 0 }; o.k++").code;
    assert!(code.contains(&Instruction::MoveTop(2)), "{code:?}");
    let code = compile_source("let o = { k: 0 }; ++o.k").code;
    assert!(
        !code.iter().any(|i| matches!(i, Instruction::MoveTop(_))),
        "{code:?}"
    );
}

#[test]
fn new_instructions_disassemble_by_name() {
    let text = compile_source("null ?? 1").disassemble();
    assert!(text.contains("JUMP_IF_NULLISH -> 0004"), "{text}");
}

#[test]
fn dup2_duplicates_the_top_two_values_in_order() {
    // [1, 2, 1, 2] folds to 1 - (2 - (1 - 2)) = -2; a wrong order would not.
    let code = vec![
        Instruction::Constant(0),
        Instruction::Constant(1),
        Instruction::Dup2,
        Instruction::Binary(BinaryOperator::Subtract),
        Instruction::Binary(BinaryOperator::Subtract),
        Instruction::Binary(BinaryOperator::Subtract),
        Instruction::Return,
    ];
    assert_eq!(
        execute(code, vec![number(1.0), number(2.0)]),
        Ok(number(-2.0))
    );
}

#[test]
fn move_top_inserts_beneath_the_given_number_of_values() {
    // [1, 2, 3] becomes [3, 1, 2], which folds to 3 - (1 - 2) = 4.
    let code = vec![
        Instruction::Constant(0),
        Instruction::Constant(1),
        Instruction::Constant(2),
        Instruction::MoveTop(2),
        Instruction::Binary(BinaryOperator::Subtract),
        Instruction::Binary(BinaryOperator::Subtract),
        Instruction::Return,
    ];
    assert_eq!(
        execute(code, vec![number(1.0), number(2.0), number(3.0)]),
        Ok(number(4.0))
    );
    // Moving past nothing keeps the order.
    let code = vec![
        Instruction::Constant(0),
        Instruction::Constant(1),
        Instruction::MoveTop(0),
        Instruction::Binary(BinaryOperator::Subtract),
        Instruction::Return,
    ];
    assert_eq!(
        execute(code, vec![number(5.0), number(2.0)]),
        Ok(number(3.0))
    );
}

#[test]
fn jump_if_nullish_keeps_the_value_and_only_jumps_for_null_or_undefined() {
    let code = |constant: Value| {
        execute(
            vec![
                Instruction::Constant(0),
                Instruction::JumpIfNullish(3),
                Instruction::Constant(1),
                Instruction::Return,
            ],
            vec![constant, string("fell through")],
        )
    };
    assert_eq!(code(Value::Null), Ok(Value::Null));
    assert_eq!(code(Value::Undefined), Ok(Value::Undefined));
    assert_eq!(code(number(0.0)), Ok(string("fell through")));
    assert_eq!(code(string("")), Ok(string("fell through")));
    assert_eq!(code(boolean(false)), Ok(string("fell through")));
}

#[test]
fn array_building_instructions_keep_the_array_on_the_stack() {
    let code = vec![
        Instruction::NewArray(0),
        Instruction::Constant(0),
        Instruction::ArrayPush,
        Instruction::Constant(1),
        Instruction::ArrayExtend,
        Instruction::Constant(2),
        Instruction::ArrayPush,
        Instruction::Return,
    ];
    let result = execute(code, vec![number(7.0), string("ab"), Value::Undefined]).unwrap();
    assert_eq!(result.to_string(), "7,a,b,");
    let Value::Array(array) = result else {
        panic!("expected an array");
    };
    assert_eq!(array.get("length"), Some(number(4.0)));
}

#[test]
fn spread_properties_copies_onto_the_object_beneath() {
    let code = vec![
        Instruction::NewObject,
        Instruction::Constant(0),
        Instruction::Constant(1),
        Instruction::NewArray(2),
        Instruction::SpreadProperties,
        Instruction::Constant(2),
        Instruction::LoadComputedProperty,
        Instruction::Return,
    ];
    let constants = vec![number(7.0), number(8.0), string("1")];
    assert_eq!(execute(code, constants), Ok(number(8.0)));
}

#[test]
fn delete_instructions_pop_their_operands_and_push_a_boolean() {
    let object = Object::new();
    object.set("k", number(1.0));
    let code = vec![
        Instruction::Constant(0),
        Instruction::DeleteProperty(1),
        Instruction::Return,
    ];
    assert_eq!(
        execute(code, vec![Value::Object(object.clone()), string("k")]),
        Ok(boolean(true))
    );
    assert!(!object.contains_key("k"));
    let code = vec![
        Instruction::Constant(0),
        Instruction::Constant(1),
        Instruction::DeleteComputedProperty,
        Instruction::Return,
    ];
    assert_eq!(
        execute(code, vec![Value::Object(Object::new()), string("missing")]),
        Ok(boolean(true))
    );
}

#[test]
fn malformed_expression_bytecode_is_an_error_not_a_panic() {
    type Case = (Vec<Instruction>, Vec<Value>, &'static str);
    let cases: Vec<Case> = vec![
        (vec![Instruction::Dup2], vec![], "underflow"),
        (
            vec![Instruction::Constant(0), Instruction::Dup2],
            vec![number(1.0)],
            "underflow",
        ),
        (vec![Instruction::MoveTop(1)], vec![], "underflow"),
        (
            vec![Instruction::Constant(0), Instruction::MoveTop(3)],
            vec![number(1.0)],
            "underflow",
        ),
        (vec![Instruction::JumpIfNullish(0)], vec![], "underflow"),
        (
            vec![Instruction::Constant(0), Instruction::JumpIfNullish(99)],
            vec![Value::Null],
            "jump target is invalid",
        ),
        (
            vec![Instruction::DeleteProperty(0)],
            vec![],
            "property name constant is invalid",
        ),
        (
            vec![Instruction::Constant(0), Instruction::DeleteProperty(5)],
            vec![Value::Object(Object::new())],
            "property name constant is invalid",
        ),
        (
            vec![Instruction::Constant(0), Instruction::DeleteProperty(1)],
            vec![Value::Null, string("x")],
            "Cannot delete property `x` of null",
        ),
        (
            vec![Instruction::DeleteComputedProperty],
            vec![],
            "underflow",
        ),
        (vec![Instruction::ArrayPush], vec![], "underflow"),
        (
            vec![
                Instruction::Constant(0),
                Instruction::Constant(1),
                Instruction::ArrayPush,
            ],
            vec![number(1.0), number(2.0)],
            "append target is not an array",
        ),
        (
            vec![Instruction::NewArray(0), Instruction::ArrayExtend],
            vec![],
            "underflow",
        ),
        (
            vec![
                Instruction::NewArray(0),
                Instruction::Constant(0),
                Instruction::ArrayExtend,
            ],
            vec![number(1.0)],
            "`1` is not iterable",
        ),
        (
            vec![
                Instruction::Constant(0),
                Instruction::Constant(1),
                Instruction::ArrayExtend,
            ],
            vec![number(1.0), string("ab")],
            "append target is not an array",
        ),
        (vec![Instruction::SpreadProperties], vec![], "underflow"),
        (
            vec![
                Instruction::Constant(0),
                Instruction::Constant(0),
                Instruction::SpreadProperties,
            ],
            vec![number(1.0)],
            "object spread target is not an object",
        ),
        (vec![Instruction::CallSpread], vec![], "underflow"),
        (
            vec![
                Instruction::Constant(0),
                Instruction::Constant(1),
                Instruction::CallSpread,
            ],
            vec![number(1.0), number(2.0)],
            "spread arguments are not an array",
        ),
        (
            vec![
                Instruction::Constant(0),
                Instruction::Constant(1),
                Instruction::CallMethodSpread,
            ],
            vec![number(1.0), number(2.0)],
            "spread arguments are not an array",
        ),
        (
            vec![
                Instruction::Constant(0),
                Instruction::NewArray(0),
                Instruction::CallSpread,
            ],
            vec![number(1.0)],
            "is not a function",
        ),
        (
            vec![
                Instruction::Constant(0),
                Instruction::NewArray(0),
                Instruction::NewSpread,
            ],
            vec![number(1.0)],
            "is not a constructor",
        ),
        (vec![Instruction::NewSpread], vec![], "underflow"),
        (
            vec![
                Instruction::Constant(0),
                Instruction::Constant(1),
                Instruction::Binary(BinaryOperator::In),
            ],
            vec![string("x"), number(1.0)],
            "Cannot use 'in' operator",
        ),
        (
            vec![
                Instruction::Constant(0),
                Instruction::Constant(1),
                Instruction::Binary(BinaryOperator::InstanceOf),
            ],
            vec![number(1.0), number(1.0)],
            "not callable",
        ),
    ];
    for (code, constants, expected) in cases {
        let error = execute(code.clone(), constants).expect_err(&format!("{code:?} should fail"));
        assert!(
            error.message.contains(expected),
            "{code:?}: expected `{expected}`, got {error:?}"
        );
    }
}

#[test]
fn hand_built_asts_with_misplaced_nodes_are_compile_errors() {
    use js::{Program, Statement};
    let compile_expression = |expression: Expr| {
        compile(&Program {
            body: vec![Statement::Expression(expression)],
        })
    };
    let error = compile_expression(Expr::Spread(Box::new(Expr::Number(1.0)))).unwrap_err();
    assert!(!error.is_unsupported());
    assert!(error.message.contains("spread"), "{error}");
    let error = compile_expression(Expr::OptionalMember {
        object: Box::new(Expr::Null),
        property: MemberProperty::Named("a".into()),
    })
    .unwrap_err();
    assert!(!error.is_unsupported());
    assert!(error.message.contains("optional"), "{error}");
    let error = compile_expression(Expr::Update {
        target: Box::new(Expr::Number(1.0)),
        operator: js::ast::UpdateOperator::Increment,
        prefix: true,
    })
    .unwrap_err();
    assert!(!error.is_unsupported());
    let error = compile_expression(Expr::CompoundAssign {
        target: Box::new(Expr::Number(1.0)),
        operator: AssignmentOperator::Binary(BinaryOperator::Add),
        value: Box::new(Expr::Number(1.0)),
    })
    .unwrap_err();
    assert!(!error.is_unsupported());
    // The evaluator reports the same misplacement as an error, not a panic.
    for expression in [
        Expr::Spread(Box::new(Expr::Number(1.0))),
        Expr::OptionalMember {
            object: Box::new(Expr::Null),
            property: MemberProperty::Named("a".into()),
        },
        Expr::Update {
            target: Box::new(Expr::Number(1.0)),
            operator: js::ast::UpdateOperator::Increment,
            prefix: false,
        },
    ] {
        let program = js::Program {
            body: vec![js::Statement::Expression(expression)],
        };
        assert!(evaluate_program(&program).is_err());
    }
}
