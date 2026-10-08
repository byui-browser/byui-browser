//! Behavioral tests for the tree-walk interpreter, checked against
//! JavaScript semantics.

use std::sync::{Arc, Mutex};

use js::{HostFunction, JsErrorCategory, JsResult, Realm, Value, eval, eval_with_limits};

fn number(value: f64) -> JsResult<Value> {
    Ok(Value::Number(value))
}

fn string(value: &str) -> JsResult<Value> {
    Ok(Value::String(value.into()))
}

fn boolean(value: bool) -> JsResult<Value> {
    Ok(Value::Boolean(value))
}

#[test]
fn addition_concatenates_when_either_side_is_a_string() {
    assert_eq!(eval("'a' + 'b'"), string("ab"));
    assert_eq!(eval("'1' + '2'"), string("12"));
    assert_eq!(eval("'1' + 2"), string("12"));
    assert_eq!(eval("1 + 2 + '3'"), string("33"));
    assert_eq!(
        eval("'' + 1.5 + true + null + undefined"),
        string("1.5truenullundefined")
    );
    assert_eq!(
        eval("'' + 1e21 + ' ' + 1e-7 + ' ' + 1 / 0"),
        string("1e+21 1e-7 Infinity")
    );
    assert_eq!(eval("true + 1"), number(2.0));
    assert_eq!(eval("null + 1"), number(1.0));
}

#[test]
fn strict_equality_compares_types_and_values() {
    for (source, expected) in [
        ("undefined === undefined", true),
        ("null === null", true),
        ("'a' === 'a'", true),
        ("1 === 1", true),
        ("1 === '1'", false),
        ("true === 1", false),
        ("null === 0", false),
        ("null === undefined", false),
        ("0 / 0 === 0 / 0", false),
        ("1 !== '1'", true),
    ] {
        assert_eq!(eval(source), boolean(expected), "{source}");
    }
}

#[test]
fn loose_equality_follows_javascript_coercions() {
    for (source, expected) in [
        ("undefined == null", true),
        ("null == undefined", true),
        ("undefined == undefined", true),
        ("null == 0", false),
        ("undefined == 0", false),
        ("null == false", false),
        ("1 == '1'", true),
        ("'' == 0", true),
        ("'0x10' == 16", true),
        ("true == 1", true),
        ("true == '1'", true),
        ("false == ''", true),
        ("'a' == 'a'", true),
        ("'a' == 'b'", false),
        ("'a' != 'b'", true),
    ] {
        assert_eq!(eval(source), boolean(expected), "{source}");
    }
}

#[test]
fn relational_operators_compare_strings_and_numbers() {
    for (source, expected) in [
        ("'a' < 'b'", true),
        ("'b' < 'a'", false),
        ("'10' < '9'", true),
        ("'10' < 9", false),
        ("1 <= 1", true),
        ("2 >= 3", false),
        ("undefined < 1", false),
        ("undefined >= 1", false),
        ("null >= 0", true),
    ] {
        assert_eq!(eval(source), boolean(expected), "{source}");
    }
}

#[test]
fn numeric_conversion_of_strings_matches_javascript() {
    assert_eq!(eval("'' * 1"), number(0.0));
    assert_eq!(eval("' 42 ' * 1"), number(42.0));
    assert_eq!(eval("'0x10' * 1"), number(16.0));
    assert_eq!(eval("'Infinity' * 1"), number(f64::INFINITY));
    for source in ["'abc' * 1", "'inf' * 1", "-'abc'", "undefined + 1"] {
        let Ok(Value::Number(result)) = eval(source) else {
            panic!("{source} should produce a number");
        };
        assert!(result.is_nan(), "{source}");
    }
}

#[test]
fn remainder_bitwise_and_shift_operators() {
    for (source, expected) in [
        ("7 % 3", 1.0),
        ("-7 % 3", -1.0),
        ("5.5 % 2", 1.5),
        ("6 & 3", 2.0),
        ("6 | 3", 7.0),
        ("6 ^ 3", 5.0),
        ("~5", -6.0),
        ("1 << 31", -2_147_483_648.0),
        ("1 << 32", 1.0),
        ("-16 >> 2", -4.0),
        ("-1 >>> 0", 4_294_967_295.0),
        ("-1 >>> 28", 15.0),
        ("4294967296 | 0", 0.0),
        ("'12' | 0", 12.0),
    ] {
        assert_eq!(eval(source), number(expected), "{source}");
    }
}

#[test]
fn var_is_function_scoped_and_hoisted() {
    assert_eq!(eval("var x = 1; var x = 2; x"), number(2.0));
    assert_eq!(eval("{ var y = 5; } y"), number(5.0));
    assert_eq!(
        eval("var before = hoisted; var hoisted = 1; before"),
        Ok(Value::Undefined)
    );
    assert_eq!(
        eval("function f() { if (true) { var inner = 3; } return inner; } f()"),
        number(3.0)
    );
    assert!(eval("function f() { var local = 1; } f(); local").is_err());
}

#[test]
fn let_and_const_are_block_scoped_with_a_dead_zone() {
    assert_eq!(eval("let x = 1; { let x = 2; } x"), number(1.0));
    assert!(eval("{ let y = 1; } y").is_err());
    assert!(eval("let x = 1; { x; let x = 2; }").is_err());
    assert!(eval("let x; var x;").is_err());
    assert!(eval("var x; let x;").is_err());
    assert!(eval("const c = 1; c = 2;").is_err());
    assert_eq!(eval("let u; u"), Ok(Value::Undefined));
}

#[test]
fn declarations_do_not_replace_the_completion_value() {
    assert_eq!(eval("1; let z = 2;"), number(1.0));
    assert_eq!(eval("1; var v = 2; function f() {}"), number(1.0));
    assert_eq!(eval("1; { let inner = 2; }"), number(1.0));
    assert_eq!(eval("1; if (false) 2;"), Ok(Value::Undefined));
    assert_eq!(eval("let i = 0; while (i < 3) { i = i + 1; }"), number(3.0));
}

#[test]
fn control_flow_statements_run() {
    assert_eq!(
        eval("let x = 0; if (x) { x = 1; } else { x = 2; } x"),
        number(2.0)
    );
    assert_eq!(
        eval("let total = 0; let i = 1; while (i <= 10) { total = total + i; i = i + 1; } total"),
        number(55.0)
    );
}

#[test]
fn functions_take_arguments_and_return_values() {
    assert_eq!(
        eval("function add(a, b) { return a + b; } add(2, 3)"),
        number(5.0)
    );
    assert_eq!(
        eval("function f(a, b) { return b; } f(1)"),
        Ok(Value::Undefined)
    );
    assert_eq!(eval("function f() { 1; } f()"), Ok(Value::Undefined));
    assert_eq!(eval("function f() { return; } f()"), Ok(Value::Undefined));
    assert_eq!(
        eval("f(); function f() { return 'hoisted'; }"),
        string("hoisted")
    );
    assert_eq!(
        eval("let r = later(); function later() { return 7; } r"),
        number(7.0)
    );
}

#[test]
fn return_exits_loops_and_nested_blocks() {
    assert_eq!(
        eval(
            "function find() { let i = 0; while (true) { if (i == 3) { return i; } i = i + 1; } }
             find()"
        ),
        number(3.0)
    );
}

#[test]
fn recursion_works() {
    assert_eq!(
        eval(
            "function fib(n) { if (n < 2) { return n; } return fib(n - 1) + fib(n - 2); }
             fib(15)"
        ),
        number(610.0)
    );
}

#[test]
fn closures_capture_their_defining_scope() {
    assert_eq!(
        eval("let x = 1; function f() { return x; } { let x = 2; f(); }"),
        number(1.0)
    );
    assert_eq!(
        eval(
            "function counter() { let count = 0; function next() { count = count + 1; return count; } return next; }
             let tick = counter(); tick(); tick(); tick()"
        ),
        number(3.0)
    );
    assert_eq!(
        eval(
            "function outer() { let a = 'A'; function inner() { return a; } return inner; } outer()()"
        ),
        string("A")
    );
}

#[test]
fn functions_are_values_compared_by_identity() {
    assert_eq!(eval("function f() {} let g = f; g === f"), boolean(true));
    assert_eq!(
        eval("function f() {} function g() {} f == g"),
        boolean(false)
    );
    assert_eq!(eval("function f() {} !f"), boolean(false));
    assert_eq!(
        eval("function f(a, b) {} '' + f"),
        string("function f(a, b) { [code] }")
    );
    assert!(
        matches!(eval("function f() {} f"), Ok(Value::Function(function)) if function.name() == "f")
    );
}

#[test]
fn calling_a_non_function_is_an_error() {
    let error = eval("let x = 1; x()").unwrap_err();
    assert!(error.message.contains("not a function"), "{error}");
    assert_eq!(error.category, JsErrorCategory::Runtime);
    assert_eq!(error.context.as_deref(), Some("function call"));
}

#[test]
fn unbounded_recursion_is_an_error_not_a_crash() {
    let error = eval("function f() { return f(); } f()").unwrap_err();
    assert_eq!(error.message, "Maximum call stack size exceeded");
}

#[test]
fn endless_loops_stop_at_the_step_limit() {
    let program = js::parse("let i = 0; while (true) { i = i + 1; }").unwrap();
    let error = js::runtime::evaluate_program_with_step_limit(&program, 1_000).unwrap_err();
    assert_eq!(error.message, "Script exceeded the evaluation step limit");

    // The default limit applies to every entry point.
    assert!(eval("while (true) {}").is_err());
    assert!(Realm::new().evaluate_script("while (true) {}").is_err());
}

#[test]
fn step_limit_counts_statements_and_expressions() {
    let program = js::parse("1 + 2").unwrap();
    // The statement, the addition, and its two operands.
    assert_eq!(
        js::runtime::evaluate_program_with_step_limit(&program, 4),
        number(3.0)
    );
    assert!(js::runtime::evaluate_program_with_step_limit(&program, 3).is_err());
}

#[test]
fn callers_can_configure_loop_limits_and_get_structured_diagnostics() {
    let error = eval_with_limits(
        "while (true) {}",
        js::runtime::ExecutionLimits {
            step_limit: 100_000,
            call_depth_limit: 100,
            loop_iteration_limit: 3,
        },
    )
    .unwrap_err();

    assert_eq!(error.category, JsErrorCategory::Limit);
    assert_eq!(error.message, "Script exceeded the loop iteration limit");
    assert_eq!(error.context.as_deref(), Some("while loop iteration"));
}

#[test]
fn loop_limit_allows_exactly_the_configured_number_of_iterations() {
    let limits = js::runtime::ExecutionLimits {
        loop_iteration_limit: 3,
        ..js::runtime::ExecutionLimits::default()
    };
    assert_eq!(
        eval_with_limits("let i = 0; while (i < 3) { i = i + 1; } i", limits),
        number(3.0)
    );
}

#[test]
fn syntax_errors_are_structured() {
    let error = eval("let = ;").unwrap_err();
    assert_eq!(error.category, JsErrorCategory::Syntax);
    assert_eq!(error.context.as_deref(), Some("parsing script"));
}

#[test]
fn callers_can_configure_call_depth_limits() {
    let error = eval_with_limits(
        "function recurse() { return recurse(); } recurse()",
        js::runtime::ExecutionLimits {
            step_limit: 100_000,
            call_depth_limit: 4,
            loop_iteration_limit: 100,
        },
    )
    .unwrap_err();

    assert_eq!(error.category, JsErrorCategory::Limit);
    assert_eq!(error.message, "Maximum call stack size exceeded");
    assert_eq!(error.context.as_deref(), Some("script function call depth"));
}

#[test]
fn realm_uses_configured_limits_for_each_script() {
    let mut realm = Realm::new();
    realm.set_execution_limits(js::runtime::ExecutionLimits {
        step_limit: 100_000,
        call_depth_limit: 100,
        loop_iteration_limit: 2,
    });

    let error = realm.evaluate_script("while (true) {}").unwrap_err();
    assert_eq!(error.category, JsErrorCategory::Limit);
    assert_eq!(error.context.as_deref(), Some("while loop iteration"));
}

#[test]
fn moderately_deep_recursion_succeeds() {
    assert_eq!(
        eval("function f(n) { if (n > 0) { return f(n - 1) + 1; } return 0; } f(1000)"),
        number(1000.0)
    );
}

#[test]
fn deep_programs_are_safe_on_a_small_caller_stack() {
    // Windows gives the main thread 1 MiB; the engine must not depend on it.
    let result = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            (
                eval(&vec!["1"; 500].join("+")),
                eval(&format!("{}1{}", "(".repeat(100), ")".repeat(100))),
                eval("function f() { return f(); } f()").is_err(),
            )
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(result, (number(500.0), number(1.0), true));
}

#[test]
fn realm_scripts_call_host_functions_with_arguments() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&calls);
    let log: HostFunction = Arc::new(move |arguments: &[Value]| {
        recorded.lock().unwrap().push(arguments.to_vec());
        Ok(Value::Undefined)
    });
    let double: HostFunction = Arc::new(|arguments: &[Value]| match arguments {
        [Value::Number(value)] => Ok(Value::Number(value * 2.0)),
        _ => Err(js::JsError::new("double expects one number")),
    });
    let mut realm = Realm::new();
    realm.register_global_function("log", log).unwrap();
    realm.register_global_function("double", double).unwrap();

    assert_eq!(
        realm.evaluate_script("log('total', double(21)); double(2) + 1"),
        number(5.0)
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [vec![Value::String("total".into()), Value::Number(42.0)]]
    );
    assert!(realm.evaluate_script("double('x')").is_err());
    assert!(realm.evaluate_script("missing()").is_err());
    assert!(realm.evaluate_script("double(").is_err());
}

#[test]
fn realm_scripts_may_shadow_host_globals() {
    let mut realm = Realm::new();
    realm
        .register_global_function("print", Arc::new(|_: &[Value]| Ok(Value::Null)))
        .unwrap();
    assert_eq!(realm.evaluate_script("let print = 3; print"), number(3.0));
    assert_eq!(realm.evaluate_script("print()"), Ok(Value::Null));
}

#[test]
fn values_display_as_javascript_strings() {
    for (value, expected) in [
        (Value::Undefined, "undefined"),
        (Value::Null, "null"),
        (Value::Boolean(true), "true"),
        (Value::Number(1.0), "1"),
        (Value::Number(-0.0), "0"),
        (Value::Number(1e21), "1e+21"),
        (Value::Number(f64::NAN), "NaN"),
        (Value::String("text".into()), "text"),
    ] {
        assert_eq!(value.to_string(), expected);
    }
}

#[test]
fn values_can_cross_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Value>();
}
