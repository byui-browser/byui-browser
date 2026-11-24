use js::{ExecutionMode, JsErrorCategory, Realm, Value, eval_with_mode, parse};

fn assert_value(source: &str, expected: Value) {
    for mode in [
        ExecutionMode::Interpreter,
        ExecutionMode::Bytecode,
        ExecutionMode::Automatic,
    ] {
        assert_eq!(
            eval_with_mode(source, mode),
            Ok(expected.clone()),
            "{mode:?}: {source}"
        );
    }
    assert_eq!(
        Realm::new().evaluate_script(source),
        Ok(expected),
        "realm: {source}"
    );
}

#[test]
fn anonymous_expressions_work_as_initializers_callbacks_and_iifes() {
    for source in [
        "var f = function (x) { return x + 1; }; f(41)",
        "(function (x) { return x + 1; })(41)",
        "function apply(f, x) { return f(x); } apply(function (x) { return x + 1; }, 41)",
        "var o = {run: function (x) { return x + 1; }}; o.run(41)",
        "var f = function () {}; f() === undefined ? 42 : 0",
        "var n = 0; !function () { n = 42; }(); n",
    ] {
        assert_value(source, Value::Number(42.0));
    }
}

#[test]
fn closures_capture_live_bindings_and_keep_independent_instances() {
    assert_value(
        "var make = function (n) { return function () { n += 1; return n; }; }; \
         var a = make(0); var b = make(10); a(); a() + b()",
        Value::Number(13.0),
    );
    assert_value(
        "var make = function (x) { return function () { return () => x; }; }; make(42)()()",
        Value::Number(42.0),
    );
    assert_value(
        "var f; { let n = 41; f = function () { return n + 1; }; n = 42; } f()",
        Value::Number(43.0),
    );
}

#[test]
fn named_expressions_recurse_without_leaking_or_using_the_outer_binding() {
    assert_value(
        "var factorial = 7; var f = function factorial(n) { \
         return n < 2 ? 1 : n * factorial(n - 1); }; var saved = f; f = 0; saved(5) + factorial",
        Value::Number(127.0),
    );
    assert_value(
        "var f = function privateName() { return privateName; }; \
         f() === f && typeof privateName === 'undefined'",
        Value::Boolean(true),
    );
    assert_value(
        "var f = function self(self) { return self; }; f(42)",
        Value::Number(42.0),
    );
    assert_value(
        "var f = function self() { var self = 42; return self; }; f()",
        Value::Number(42.0),
    );
}

#[test]
fn expressions_bind_dynamic_this_and_can_construct_instances() {
    assert_value(
        "var o = {n: 42, get: function () { return this.n; }}; o.get()",
        Value::Number(42.0),
    );
    assert_value("(function () { return this; })()", Value::Undefined);
    assert_value(
        "var C = function (n) { this.n = n; }; new C(42).n",
        Value::Number(42.0),
    );
    assert_value(
        "var C = function () { return {n: 42}; }; new C().n",
        Value::Number(42.0),
    );
}

#[test]
fn expressions_create_fresh_functions_and_are_not_hoisted() {
    assert_value(
        "var make = function () { return function () {}; }; var f = make(); f === f && f !== make()",
        Value::Boolean(true),
    );
    assert_value(
        "var before = typeof f; var f = function () {}; before",
        Value::String("undefined".into()),
    );
}

#[test]
fn errors_preserve_binding_and_call_boundaries() {
    for (source, category, message) in [
        (
            "var f = function self() { self = 1; }; f()",
            JsErrorCategory::Runtime,
            "constant",
        ),
        (
            "var f = function () { return n; }; f(); let n = 1;",
            JsErrorCategory::Runtime,
            "initializ",
        ),
        (
            "var f = function () { throw 'boom'; }; f()",
            JsErrorCategory::Thrown,
            "Uncaught boom",
        ),
    ] {
        for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
            let error = eval_with_mode(source, mode).unwrap_err();
            assert_eq!(error.category, category, "{mode:?}: {source}");
            assert!(error.message.contains(message), "{mode:?}: {error}");
        }
    }
    assert_value(
        "var f = function () { throw 'boom'; }; var result = ''; \
         try { f(); } catch (e) { result = e; } result",
        Value::String("boom".into()),
    );
}

#[test]
fn unsupported_parameters_and_invalid_function_bodies_still_fail_to_parse() {
    for source in [
        "function () {}", // A statement still requires a declaration name.
        "var f = function (x, x) {};",
        "var f = function (x = 1) {};",
        "var f = function (...args) {};",
        "var f = function ({x}) {};",
        "var f = function* () {};",
        "var f = function () { return ;",
        "while (true) { var f = function () { break; }; }",
        "outer: while (true) { var f = function () { continue outer; }; }",
        "class C extends B { method() { return function () { return super.x; }; } }",
    ] {
        assert!(parse(source).is_err(), "{source}");
        assert_eq!(
            js::eval(source).unwrap_err().category,
            JsErrorCategory::Syntax
        );
    }
}

#[test]
fn named_recursion_obeys_execution_limits_in_both_engines() {
    let source = "(function recurse() { return recurse(); })()";
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        let error = js::eval_with_mode_and_limits(
            source,
            mode,
            js::runtime::ExecutionLimits {
                call_depth_limit: 8,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.category, JsErrorCategory::Limit);
    }
}
