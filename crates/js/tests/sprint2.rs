//! Regression coverage for the Sprint 2 vertical slice.
//!
//! Every test drives the engine through the public source-level APIs
//! ([`js::eval`], [`js::parse`], and [`js::Realm::evaluate_script`]) so the
//! lexer, parser, and interpreter are exercised together. The final section
//! pins down how intentionally unsupported JavaScript fails; see
//! `docs/js/sprint2-coverage.md` for the full list.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use js::{HostFunction, JsError, JsErrorCategory, JsResult, Realm, Value, eval, parse};

fn number(value: f64) -> JsResult<Value> {
    Ok(Value::Number(value))
}

fn string(value: &str) -> JsResult<Value> {
    Ok(Value::String(value.into()))
}

fn boolean(value: bool) -> JsResult<Value> {
    Ok(Value::Boolean(value))
}

/// Evaluates `source`, asserting it fails while parsing.
fn syntax_error(source: &str) -> JsError {
    let error = eval(source).expect_err(source);
    assert_eq!(error.category, JsErrorCategory::Syntax, "{source}: {error}");
    assert_eq!(error.context.as_deref(), Some("parsing script"), "{source}");
    error
}

/// Evaluates `source`, asserting it parses but fails while running.
fn runtime_error(source: &str) -> JsError {
    assert!(parse(source).is_ok(), "{source} should parse");
    let error = eval(source).expect_err(source);
    assert_eq!(
        error.category,
        JsErrorCategory::Runtime,
        "{source}: {error}"
    );
    error
}

/// A realm with a `touch` global that counts its calls and returns its first
/// argument (or `true` when called without one).
fn counting_realm() -> (Realm, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let touch: HostFunction = Arc::new(move |arguments: &[Value]| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(arguments.first().cloned().unwrap_or(Value::Boolean(true)))
    });
    let mut realm = Realm::new();
    realm.register_global_function("touch", touch).unwrap();
    (realm, calls)
}

// Arithmetic precedence ------------------------------------------------------

#[test]
fn arithmetic_follows_javascript_precedence_and_associativity() {
    for (source, expected) in [
        ("2 + 3 * 4", 14.0),
        ("(2 + 3) * 4", 20.0),
        ("1 + 2 * 3 - 4 / 2", 5.0),
        ("(1 + 2) * (3 - 1)", 6.0),
        ("10 - 4 - 3", 3.0),
        ("8 / 4 / 2", 1.0),
        ("2 * 3 % 4", 2.0),
        ("-2 * 3", -6.0),
        ("-(2 + 3)", -5.0),
        ("- -2", 2.0),
        ("!0 + 1", 2.0),
        ("1 + 2 << 1", 6.0),
        ("5 & 3 == 1", 0.0),
        ("((((7))))", 7.0),
    ] {
        assert_eq!(eval(source), number(expected), "{source}");
    }
}

#[test]
fn logical_operators_bind_looser_than_comparisons() {
    assert_eq!(eval("1 + 2 < 4 && 3 == 3"), boolean(true));
    assert_eq!(eval("1 || 0 && 0"), number(1.0));
    assert_eq!(eval("(1 || 0) && 0"), number(0.0));
    assert_eq!(eval("0 && 1 || 2"), number(2.0));
}

// Literals --------------------------------------------------------------------

#[test]
fn numeric_literals_evaluate_to_numbers() {
    for (source, expected) in [
        ("0", 0.0),
        ("42", 42.0),
        ("3.25", 3.25),
        (".5", 0.5),
        ("5.", 5.0),
        ("1e3", 1000.0),
        ("2E2", 200.0),
        ("1.5e-3", 0.0015),
        ("1e+2", 100.0),
    ] {
        assert_eq!(eval(source), number(expected), "{source}");
    }
}

#[test]
fn string_literals_support_both_quotes_and_escapes() {
    for (source, expected) in [
        ("'single'", "single"),
        ("\"double\"", "double"),
        ("''", ""),
        ("'it\\'s'", "it's"),
        ("\"say \\\"hi\\\"\"", "say \"hi\""),
        ("'a\\nb\\tc'", "a\nb\tc"),
        ("'\\x41\\u0042\\u{43}'", "ABC"),
        ("'\\uD83D\\uDE00'", "\u{1F600}"),
        ("'caf\u{e9}'", "caf\u{e9}"),
    ] {
        assert_eq!(eval(source), string(expected), "{source}");
    }
}

#[test]
fn keyword_literals_evaluate_to_primitives() {
    assert_eq!(eval("true"), boolean(true));
    assert_eq!(eval("false"), boolean(false));
    assert_eq!(eval("null"), Ok(Value::Null));
    assert_eq!(eval("undefined"), Ok(Value::Undefined));
}

// Truthiness -------------------------------------------------------------------

#[test]
fn truthiness_matches_javascript_to_boolean() {
    for (source, expected) in [
        ("0", false),
        ("-0", false),
        ("0 / 0", false),
        ("''", false),
        ("null", false),
        ("undefined", false),
        ("false", false),
        ("1", true),
        ("-1", true),
        ("1 / 0", true),
        ("'0'", true),
        ("' '", true),
        ("'false'", true),
        ("true", true),
    ] {
        assert_eq!(
            eval(&format!("!!({source})")),
            boolean(expected),
            "!!{source}"
        );
        let branch = if expected { "then" } else { "else" };
        assert_eq!(
            eval(&format!("if ({source}) {{ 'then'; }} else {{ 'else'; }}")),
            string(branch),
            "if ({source})"
        );
    }
    assert_eq!(eval("function f() {} !!f"), boolean(true));
}

#[test]
fn while_conditions_use_truthiness() {
    assert_eq!(
        eval("let n = 3; let runs = 0; while (n) { n = n - 1; runs = runs + 1; } runs"),
        number(3.0)
    );
    assert_eq!(
        eval("let s = 'x'; let runs = 0; while (s) { s = ''; runs = runs + 1; } runs"),
        number(1.0)
    );
}

// Comparisons ------------------------------------------------------------------

#[test]
fn comparisons_produce_booleans() {
    for (source, expected) in [
        ("1 < 2", true),
        ("2 < 1", false),
        ("2 <= 2", true),
        ("3 > 2", true),
        ("2 >= 3", false),
        ("1 == 1", true),
        ("1 != 1", false),
        ("1 === 1", true),
        ("1 !== 1", false),
        ("1 < 2 < 3", true),
        ("3 > 2 > 1", false),
        ("'apple' < 'banana'", true),
        ("0 / 0 == 0 / 0", false),
        ("0 / 0 != 0 / 0", true),
        ("0 === -0", true),
    ] {
        assert_eq!(eval(source), boolean(expected), "{source}");
    }
}

// Logical short-circuiting -------------------------------------------------------

#[test]
fn logical_operators_return_an_operand_not_a_boolean() {
    assert_eq!(eval("'a' && 'b'"), string("b"));
    assert_eq!(eval("'' && 'b'"), string(""));
    assert_eq!(eval("null || 'fallback'"), string("fallback"));
    assert_eq!(eval("0 || null"), Ok(Value::Null));
    assert_eq!(eval("'first' || 'second'"), string("first"));
}

#[test]
fn short_circuited_operands_are_never_evaluated() {
    for (source, expected_calls) in [
        ("false && touch()", 0),
        ("true || touch()", 0),
        ("0 && touch() && touch()", 0),
        ("true && touch()", 1),
        ("false || touch()", 1),
        ("touch(0) && touch()", 1),
        ("touch(1) || touch()", 1),
        ("touch(1) && touch(0) && touch()", 2),
    ] {
        let (realm, calls) = counting_realm();
        realm.evaluate_script(source).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), expected_calls, "{source}");
    }
}

#[test]
fn short_circuiting_skips_assignments_on_the_right() {
    assert_eq!(eval("let x = 1; false && (x = 2); x"), number(1.0));
    assert_eq!(eval("let x = 1; true || (x = 2); x"), number(1.0));
    assert_eq!(eval("let x = 1; true && (x = 2); x"), number(2.0));
}

// Declarations and assignment ------------------------------------------------------

#[test]
fn declarations_bind_initial_values() {
    assert_eq!(eval("let a = 1; a"), number(1.0));
    assert_eq!(eval("const b = 'two'; b"), string("two"));
    assert_eq!(eval("var c = true; c"), boolean(true));
    assert_eq!(eval("var d; d"), Ok(Value::Undefined));
    assert_eq!(eval("let e; e"), Ok(Value::Undefined));
    assert_eq!(eval("let f = 1 + 2 * 3; f"), number(7.0));
}

#[test]
fn assignment_is_a_right_associative_expression() {
    assert_eq!(eval("let a; a = 5"), number(5.0));
    assert_eq!(eval("let a; let b; a = b = 5; a + b"), number(10.0));
    assert_eq!(
        eval("let a = 1; let b = (a = a + 1) * 10; a + b"),
        number(22.0)
    );
    assert_eq!(
        eval("let s = 'a'; s = s + 'b'; s = s + 'c'; s"),
        string("abc")
    );
}

#[test]
fn assignment_updates_the_nearest_enclosing_binding() {
    assert_eq!(eval("let x = 1; { x = 2; } x"), number(2.0));
    assert_eq!(eval("let x = 1; { let x = 5; x = 2; } x"), number(1.0));
    assert_eq!(
        eval("let x = 1; function set() { x = 9; } set(); x"),
        number(9.0)
    );
}

// Control flow ------------------------------------------------------------------

#[test]
fn if_else_chains_select_one_branch() {
    let classify = "function classify(n) {
            if (n < 0) { return 'negative'; }
            else if (n == 0) { return 'zero'; }
            else if (n < 10) { return 'small'; }
            else { return 'large'; }
        }";
    for (argument, expected) in [
        ("-5", "negative"),
        ("0", "zero"),
        ("7", "small"),
        ("100", "large"),
    ] {
        assert_eq!(
            eval(&format!("{classify} classify({argument})")),
            string(expected),
            "classify({argument})"
        );
    }
    assert_eq!(eval("let x = 0; if (true) x = 1; x"), number(1.0));
    assert_eq!(eval("let x = 0; if (false) x = 1; x"), number(0.0));
}

#[test]
fn while_loops_iterate_and_nest() {
    assert_eq!(eval("while (false) { 1; }"), Ok(Value::Undefined));
    assert_eq!(
        eval(
            "let out = ''; let i = 1;
             while (i <= 15) {
                 if (i % 15 == 0) { out = out + 'FB'; }
                 else if (i % 3 == 0) { out = out + 'F'; }
                 else if (i % 5 == 0) { out = out + 'B'; }
                 else { out = out + i; }
                 i = i + 1;
             }
             out"
        ),
        string("12F4BF78FB11F1314FB")
    );
    assert_eq!(
        eval(
            "let total = 0; let i = 0;
             while (i < 3) { let j = 0; while (j < 4) { total = total + 1; j = j + 1; } i = i + 1; }
             total"
        ),
        number(12.0)
    );
}

// Functions, returns, and recursion -----------------------------------------------

#[test]
fn function_calls_bind_parameters_positionally() {
    let source = "function describe(a, b, c) { return '' + a + ',' + b + ',' + c; }";
    assert_eq!(
        eval(&format!("{source} describe(1, 'two', true)")),
        string("1,two,true")
    );
    assert_eq!(
        eval(&format!("{source} describe(1)")),
        string("1,undefined,undefined")
    );
    assert_eq!(
        eval(&format!("{source} describe(1, 2, 3, 4)")),
        string("1,2,3")
    );
}

#[test]
fn functions_are_first_class_values() {
    assert_eq!(
        eval(
            "function apply(f, x) { return f(x); } function square(n) { return n * n; }
             apply(square, 7)"
        ),
        number(49.0)
    );
    assert_eq!(
        eval(
            "function adder(a) { function add(b) { return a + b; } return add; }
             adder(2)(3)"
        ),
        number(5.0)
    );
}

#[test]
fn return_stops_execution_of_the_function_body() {
    assert_eq!(eval("function f() { return 1; 2; } f()"), number(1.0));
    let (realm, calls) = counting_realm();
    assert_eq!(
        realm.evaluate_script("function f() { return 'done'; touch(); } f()"),
        string("done")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        eval("function f(n) { while (true) { if (n > 2) { return n; } n = n + 1; } } f(0)"),
        number(3.0)
    );
    assert_eq!(eval("function f() { return; } f()"), Ok(Value::Undefined));
}

#[test]
fn recursion_computes_results() {
    assert_eq!(
        eval("function fact(n) { if (n <= 1) { return 1; } return n * fact(n - 1); } fact(10)"),
        number(3_628_800.0)
    );
    assert_eq!(
        eval(
            "function isEven(n) { if (n == 0) { return true; } return isOdd(n - 1); }
             function isOdd(n) { if (n == 0) { return false; } return isEven(n - 1); }
             isEven(10) && !isEven(7)"
        ),
        boolean(true)
    );
    assert_eq!(
        eval(
            "function gcd(a, b) { if (b == 0) { return a; } return gcd(b, a % b); } gcd(1071, 462)"
        ),
        number(21.0)
    );
}

// Syntax errors ------------------------------------------------------------------

#[test]
fn syntax_errors_report_location_and_offending_token() {
    let error = parse("let = ;").unwrap_err();
    assert_eq!(error.token_index, 1);
    assert_eq!(error.context, "variable declaration");
    assert_eq!(error.found.as_deref(), Some("Assign"));
    assert_eq!(
        (error.offset, error.line, error.column),
        (Some(4), Some(1), Some(5))
    );

    let error = parse("let a = 1;\nlet b = ;").unwrap_err();
    assert_eq!((error.line, error.column), (Some(2), Some(9)));
    assert_eq!(error.found.as_deref(), Some("Semicolon"));

    let error = parse("function f() {").unwrap_err();
    assert_eq!(error.found, None);
    assert_eq!((error.line, error.column), (Some(1), Some(15)));
}

#[test]
fn syntax_errors_surface_through_eval_with_their_location() {
    let error = syntax_error("let a = 1;\nlet b = ;");
    assert!(error.message.contains("line 2, column 9"), "{error}");
}

#[test]
fn representative_malformed_programs_are_syntax_errors() {
    for source in [
        "1 +",
        "(1 + 2",
        "1 + 2)",
        "let 5 = 1;",
        "const c;",
        "if 1 { }",
        "if (true) let x = 1;",
        "while (true) const y = 1;",
        "function (a) { }",
        "function f(a, a) { }",
        "function f(a,) { }",
        "f(1,)",
        "{ 1;",
        "1 2",
        "1 = 2",
        "'unterminated",
        "/* unterminated",
        "1..2",
        "@",
    ] {
        syntax_error(source);
    }
}

#[test]
fn syntax_errors_prevent_any_execution() {
    let (realm, calls) = counting_realm();
    let error = realm.evaluate_script("touch(); let = ;").unwrap_err();
    assert_eq!(error.category, JsErrorCategory::Syntax);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

// Runtime errors -----------------------------------------------------------------

#[test]
fn runtime_errors_describe_the_failure() {
    for (source, message) in [
        ("missing", "`missing` is not defined"),
        ("missing + 1", "`missing` is not defined"),
        (
            "{ x; let x = 1; }",
            "Cannot access `x` before initialization",
        ),
        (
            "const answer = 42; answer = 7",
            "Assignment to constant `answer`",
        ),
        ("let a = 1; let a = 2;", "`a` has already been declared"),
        ("let a = 1; var a;", "`a` has already been declared"),
        ("let x = 1; x()", "`x` is not a function"),
        ("'text'()", "expression is not a function"),
    ] {
        assert_eq!(runtime_error(source).message, message, "{source}");
    }
}

#[test]
fn runtime_errors_stop_the_script_at_the_failing_statement() {
    let (realm, calls) = counting_realm();
    let error = realm
        .evaluate_script("touch(); missing; touch();")
        .unwrap_err();
    assert_eq!(error.message, "`missing` is not defined");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn runtime_errors_propagate_out_of_nested_calls() {
    let error = runtime_error(
        "function inner() { return missing; } function outer() { return inner(); } outer()",
    );
    assert_eq!(error.message, "`missing` is not defined");
}

// Host-function invocation ----------------------------------------------------------

#[test]
fn host_functions_receive_evaluated_arguments_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&calls);
    let record: HostFunction = Arc::new(move |arguments: &[Value]| {
        recorded.lock().unwrap().push(arguments.to_vec());
        Ok(Value::Number(arguments.len() as f64))
    });
    let mut realm = Realm::new();
    realm.register_global_function("record", record).unwrap();

    assert_eq!(
        realm.evaluate_script("let x = 2; record(x * 3, 'a' + 'b', x > 1, null, undefined)"),
        number(5.0)
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [vec![
            Value::Number(6.0),
            Value::String("ab".into()),
            Value::Boolean(true),
            Value::Null,
            Value::Undefined,
        ]]
    );
}

#[test]
fn host_functions_are_callable_from_script_functions_and_loops() {
    let (realm, calls) = counting_realm();
    assert_eq!(
        realm.evaluate_script(
            "function ping(n) { return touch(n) + 1; }
             let i = 0; let sum = 0;
             while (i < 4) { sum = sum + ping(i); i = i + 1; }
             sum"
        ),
        number(10.0)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[test]
fn host_functions_are_values() {
    let (realm, calls) = counting_realm();
    assert_eq!(
        realm.evaluate_script("let alias = touch; alias('via alias')"),
        string("via alias")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(realm.evaluate_script("touch === touch"), boolean(true));
    assert_eq!(
        realm.evaluate_script("'' + touch"),
        string("function touch() { [native code] }")
    );
}

/// Host failures are currently returned unchanged, so a host that reports
/// `JsError::new` surfaces as a `Runtime` error without call context. The
/// `Host` category is reserved but not yet applied by the interpreter.
#[test]
fn host_errors_propagate_unchanged_and_stop_the_script() {
    let fail: HostFunction = Arc::new(|_: &[Value]| Err(JsError::new("host refused")));
    let (mut realm, calls) = counting_realm();
    realm.register_global_function("fail", fail).unwrap();

    let error = realm
        .evaluate_script("touch(); fail(); touch();")
        .unwrap_err();
    assert_eq!(error, JsError::new("host refused"));
    assert_eq!(error.category, JsErrorCategory::Runtime);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn each_realm_script_starts_with_fresh_script_bindings() {
    let (realm, _) = counting_realm();
    realm.evaluate_script("let kept = 1;").unwrap();
    let error = realm.evaluate_script("kept").unwrap_err();
    assert_eq!(error.message, "`kept` is not defined");
    assert_eq!(realm.evaluate_script("let kept = 2; kept"), number(2.0));
}

// Intentionally unsupported JavaScript --------------------------------------------

/// Syntax outside the Sprint 2 subset is rejected while parsing rather than
/// misinterpreted. Remove entries as features land.
#[test]
fn unsupported_syntax_is_rejected_at_parse_time() {
    for source in [
        // Member access, objects, and arrays.
        "let o = 1; o.x",
        "let o = {};",
        "let a = [1, 2];",
        // Update and compound assignment.
        "let i = 0; i++;",
        "let i = 0; i += 1;",
        // Unary plus and `typeof`.
        "+'3'",
        "typeof 1",
        // Function expressions and arrow functions.
        "let f = function () {};",
        "let f = () => 1;",
        // Template literals.
        "`text`",
        // `for` loops.
        "for (let i = 0; i < 3; i = i + 1) {}",
        // Multiple declarators.
        "let a = 1, b = 2;",
        // Empty statements.
        ";",
        "1;;",
        // `return` outside a function.
        "return 1;",
        // No automatic semicolon insertion between statements.
        "let a = 1\nlet b = 2",
        // `undefined` is a keyword here, so it cannot be a binding name.
        "let undefined = 1;",
        // Hexadecimal numeric literals in source.
        "0x10",
    ] {
        syntax_error(source);
    }
}

/// Some unsupported keywords and globals lex as plain identifiers, so they
/// parse but fail at run time as undefined bindings.
#[test]
fn unsupported_keywords_and_globals_fail_at_run_time() {
    for (source, message) in [
        ("while (true) { break; }", "`break` is not defined"),
        ("NaN", "`NaN` is not defined"),
        ("Infinity", "`Infinity` is not defined"),
        ("this", "`this` is not defined"),
        ("undeclared = 1", "`undeclared` is not defined"),
    ] {
        assert_eq!(runtime_error(source).message, message, "{source}");
    }
}
