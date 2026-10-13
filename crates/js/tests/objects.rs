//! Object values and member access: `object.name`, `object[key]`, member
//! assignment, and calling host-provided methods.
//!
//! Scripts cannot create objects yet (no object literals), so every object
//! here is built by the host and installed with `Realm::register_global_value`.

use std::sync::{Arc, Mutex};

use js::ast::{Expr, MemberProperty};
use js::lexer::{Token, tokenize};
use js::{HostFunction, JsError, JsErrorCategory, Object, Realm, Value, eval, parse};

fn number(value: f64) -> Result<Value, JsError> {
    Ok(Value::Number(value))
}

fn string(value: &str) -> Result<Value, JsError> {
    Ok(Value::String(value.into()))
}

fn expression(source: &str) -> Expr {
    match parse(source).unwrap().body.as_slice() {
        [js::Statement::Expression(expression)] => expression.clone(),
        other => panic!("expected one expression statement, got {other:?}"),
    }
}

fn named(object: Expr, name: &str) -> Expr {
    Expr::Member {
        object: Box::new(object),
        property: MemberProperty::Named(name.into()),
    }
}

fn id(name: &str) -> Expr {
    Expr::Identifier(name.into())
}

/// A realm with a global `object` whose `value` property is `1`.
fn realm_with_object() -> (Realm, Object) {
    let object = Object::new();
    object.set("value", Value::Number(1.0));
    let mut realm = Realm::new();
    realm
        .register_global_value("object", Value::Object(object.clone()))
        .unwrap();
    (realm, object)
}

/// A realm with a `log` global that records each evaluated argument list, so
/// tests can observe evaluation order.
fn recording_realm() -> (Realm, Object, Arc<Mutex<Vec<String>>>) {
    let (mut realm, object) = realm_with_object();
    let log = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&log);
    let function: HostFunction = Arc::new(move |arguments: &[Value]| {
        let first = arguments.first().cloned().unwrap_or(Value::Undefined);
        recorded.lock().unwrap().push(first.to_string());
        Ok(first)
    });
    realm.register_global_function("log", function).unwrap();
    (realm, object, log)
}

fn runtime_error(realm: &Realm, source: &str) -> JsError {
    let error = realm.evaluate_script(source).expect_err(source);
    assert_eq!(
        error.category,
        JsErrorCategory::Runtime,
        "{source}: {error}"
    );
    error
}

// Lexing and parsing --------------------------------------------------------------

#[test]
fn lexes_member_access_punctuation() {
    assert_eq!(
        tokenize("a.b[c]"),
        vec![
            Token::Identifier("a".into()),
            Token::Dot,
            Token::Identifier("b".into()),
            Token::LeftBracket,
            Token::Identifier("c".into()),
            Token::RightBracket,
        ]
    );
    // A dot followed by a digit still starts a number.
    assert_eq!(
        tokenize("a .5"),
        [Token::Identifier("a".into()), Token::Number(0.5)]
    );
}

#[test]
fn parses_dot_and_computed_member_chains_left_to_right() {
    assert_eq!(
        expression("a.b[c].d"),
        named(
            Expr::Member {
                object: Box::new(named(id("a"), "b")),
                property: MemberProperty::Computed(Box::new(id("c"))),
            },
            "d"
        )
    );
}

#[test]
fn parses_member_calls_and_calls_on_members() {
    assert_eq!(
        expression("console.log(1).x"),
        named(
            Expr::Call {
                callee: Box::new(named(id("console"), "log")),
                arguments: vec![Expr::Number(1.0)],
            },
            "x"
        )
    );
}

#[test]
fn parses_member_assignment_as_right_associative() {
    assert_eq!(
        expression("a.b = c[d] = 3"),
        Expr::MemberAssign {
            object: Box::new(id("a")),
            property: MemberProperty::Named("b".into()),
            value: Box::new(Expr::MemberAssign {
                object: Box::new(id("c")),
                property: MemberProperty::Computed(Box::new(id("d"))),
                value: Box::new(Expr::Number(3.0)),
            }),
        }
    );
}

#[test]
fn member_binds_tighter_than_unary_and_binary_operators() {
    assert_eq!(
        expression("-a.b + c[0]"),
        Expr::Binary {
            left: Box::new(Expr::Unary {
                operator: js::ast::UnaryOperator::Negate,
                operand: Box::new(named(id("a"), "b")),
            }),
            operator: js::ast::BinaryOperator::Add,
            right: Box::new(Expr::Member {
                object: Box::new(id("c")),
                property: MemberProperty::Computed(Box::new(Expr::Number(0.0))),
            }),
        }
    );
}

#[test]
fn keywords_are_allowed_as_dot_property_names() {
    for keyword in [
        "if",
        "else",
        "while",
        "function",
        "return",
        "let",
        "var",
        "const",
        "true",
        "false",
        "null",
        "undefined",
    ] {
        assert_eq!(
            expression(&format!("o.{keyword}")),
            named(id("o"), keyword),
            "o.{keyword}"
        );
    }
}

// Reading properties -----------------------------------------------------------------

#[test]
fn reads_properties_with_dot_and_computed_access() {
    let (realm, object) = realm_with_object();
    object.set("name", Value::String("box".into()));
    object.set("1", Value::Boolean(true));

    assert_eq!(realm.evaluate_script("object.value"), number(1.0));
    assert_eq!(realm.evaluate_script("object['name']"), string("box"));
    assert_eq!(
        realm.evaluate_script("let key = 'na' + 'me'; object[key]"),
        string("box")
    );
    // Computed keys are converted to strings.
    assert_eq!(realm.evaluate_script("object[1]"), Ok(Value::Boolean(true)));
    assert_eq!(
        realm.evaluate_script("object[0 + 1]"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn reads_nested_objects() {
    let (realm, object) = realm_with_object();
    let inner = Object::new();
    inner.set("deep", Value::String("found".into()));
    object.set("inner", Value::Object(inner));

    assert_eq!(realm.evaluate_script("object.inner.deep"), string("found"));
    assert_eq!(
        realm.evaluate_script("object['inner']['deep']"),
        string("found")
    );
    assert_eq!(
        realm.evaluate_script("(object).inner.deep"),
        string("found")
    );
}

#[test]
fn member_values_work_in_expressions_and_control_flow() {
    let (realm, _) = realm_with_object();
    assert_eq!(
        realm.evaluate_script(
            "let total = 0;
             while (object.value < 5) { total = total + object.value; object.value = object.value + 1; }
             total"
        ),
        number(10.0)
    );
}

// Writing properties ------------------------------------------------------------------

#[test]
fn member_assignment_updates_and_creates_properties() {
    let (realm, object) = realm_with_object();

    assert_eq!(realm.evaluate_script("object.value = 3"), number(3.0));
    assert_eq!(object.get("value"), Some(Value::Number(3.0)));
    assert_eq!(
        realm.evaluate_script("object['fresh'] = 'new'"),
        string("new")
    );
    assert_eq!(object.get("fresh"), Some(Value::String("new".into())));
    assert_eq!(
        realm.evaluate_script("object[2] = 0; object['2']"),
        number(0.0)
    );
    assert_eq!(object.keys(), ["value", "fresh", "2"]);
}

#[test]
fn chained_assignment_assigns_the_same_value() {
    let (realm, object) = realm_with_object();
    assert_eq!(
        realm.evaluate_script("let x; x = object.a = object['b'] = 7; x"),
        number(7.0)
    );
    assert_eq!(object.get("a"), Some(Value::Number(7.0)));
    assert_eq!(object.get("b"), Some(Value::Number(7.0)));
}

#[test]
fn objects_are_shared_references() {
    let (realm, object) = realm_with_object();
    assert_eq!(
        realm.evaluate_script("let alias = object; alias.value = 9; object.value"),
        number(9.0)
    );
    assert_eq!(
        realm.evaluate_script("let alias = object; alias === object"),
        Ok(Value::Boolean(true))
    );
    // Property changes persist across scripts and are visible to the host.
    realm.evaluate_script("object.count = 1").unwrap();
    assert_eq!(realm.evaluate_script("object.count"), number(1.0));
    assert_eq!(object.get("count"), Some(Value::Number(1.0)));
}

#[test]
fn objects_can_store_and_return_themselves() {
    let (realm, object) = realm_with_object();
    assert_eq!(
        realm.evaluate_script("object.self = object; object.self.self.value"),
        number(1.0)
    );
    // Debug output lists keys only, so a cycle does not recurse forever.
    assert_eq!(
        format!("{object:?}"),
        r#"Object { keys: ["value", "self"] }"#
    );
}

// Evaluation order ---------------------------------------------------------------------

#[test]
fn computed_reads_evaluate_object_then_key() {
    let (realm, _, log) = recording_realm();
    realm.evaluate_script("log(object)[log('value')]").unwrap();
    assert_eq!(*log.lock().unwrap(), ["[object Object]", "value"]);
}

#[test]
fn member_assignment_evaluates_object_then_key_then_value() {
    let (realm, object, log) = recording_realm();
    realm
        .evaluate_script("log(object)[log('k')] = log(5)")
        .unwrap();
    assert_eq!(*log.lock().unwrap(), ["[object Object]", "k", "5"]);
    assert_eq!(object.get("k"), Some(Value::Number(5.0)));
}

#[test]
fn method_is_read_before_arguments_are_evaluated() {
    let (realm, _, log) = recording_realm();
    let error = runtime_error(&realm, "object.missing(log('argument'))");
    assert_eq!(error.message, "`object.missing` is not defined");
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn assigning_to_a_non_object_fails_after_evaluating_the_value() {
    let (realm, _, log) = recording_realm();
    let error = runtime_error(&realm, "let n = null; n[log('k')] = log('v')");
    assert_eq!(error.message, "Cannot set property `k` of null");
    assert_eq!(*log.lock().unwrap(), ["k", "v"]);
}

// Host methods -----------------------------------------------------------------------------

#[test]
fn host_objects_expose_callable_methods() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&messages);
    let console = Object::new();
    console.set_method(
        "log",
        Arc::new(move |arguments: &[Value]| {
            let line: Vec<_> = arguments.iter().map(Value::to_string).collect();
            recorded.lock().unwrap().push(line.join(" "));
            Ok(Value::Undefined)
        }),
    );
    let mut realm = Realm::new();
    realm
        .register_global_value("console", Value::Object(console))
        .unwrap();

    assert_eq!(
        realm.evaluate_script(
            "function greet(name) { console.log('hello', name); return name; }
             console.log(1 + 1);
             console['log']('computed');
             let log = console.log; log('detached');
             greet('world')"
        ),
        string("world")
    );
    assert_eq!(
        *messages.lock().unwrap(),
        ["2", "computed", "detached", "hello world"]
    );
    assert_eq!(
        realm.evaluate_script("'' + console.log"),
        string("function log() { [native code] }")
    );
}

#[test]
fn nested_namespaces_expose_methods() {
    let storage = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let local_storage = Object::new();
    let writes = Arc::clone(&storage);
    local_storage.set_method(
        "setItem",
        Arc::new(move |arguments: &[Value]| {
            let [Value::String(key), value] = arguments else {
                return Err(JsError::new("setItem expects a key and a value"));
            };
            writes.lock().unwrap().push((key.clone(), value.clone()));
            Ok(Value::Undefined)
        }),
    );
    let window = Object::new();
    window.set("localStorage", Value::Object(local_storage));
    let mut realm = Realm::new();
    realm
        .register_global_value("window", Value::Object(window))
        .unwrap();

    realm
        .evaluate_script("window.localStorage.setItem('theme', 'dark')")
        .unwrap();
    assert_eq!(
        *storage.lock().unwrap(),
        [("theme".to_owned(), Value::String("dark".into()))]
    );
    // Host errors from methods propagate unchanged.
    let error = runtime_error(&realm, "window.localStorage.setItem(1)");
    assert_eq!(error.message, "setItem expects a key and a value");
}

#[test]
fn script_functions_stored_on_objects_are_callable() {
    let (realm, object) = realm_with_object();
    assert_eq!(
        realm.evaluate_script(
            "function double(n) { return n * 2; } object.double = double; object.double(21)"
        ),
        number(42.0)
    );
    assert!(matches!(object.get("double"), Some(Value::Function(_))));
}

#[test]
fn methods_do_not_receive_this() {
    let (realm, _) = realm_with_object();
    let error = runtime_error(
        &realm,
        "function read() { return this.value; } object.read = read; object.read()",
    );
    assert_eq!(error.message, "`this` is not defined");
}

// Errors -------------------------------------------------------------------------------------

#[test]
fn missing_properties_are_runtime_errors() {
    let (realm, _) = realm_with_object();
    for (source, message) in [
        ("object.missing", "`object.missing` is not defined"),
        (
            "object.value.missing",
            "Cannot read property `missing` of a number",
        ),
        ("object['missing']", "Property `missing` is not defined"),
        ("object.nested.deep", "`object.nested` is not defined"),
    ] {
        let error = runtime_error(&realm, source);
        assert_eq!(error.message, message, "{source}");
    }
    let error = runtime_error(&realm, "object.missing");
    assert_eq!(error.context.as_deref(), Some("property access"));
}

#[test]
fn property_access_on_non_objects_is_a_runtime_error() {
    for (source, message) in [
        ("undefined.x", "Cannot read property `x` of undefined"),
        ("null.x", "Cannot read property `x` of null"),
        ("true.x", "Cannot read property `x` of a boolean"),
        ("(1).x", "Cannot read property `x` of a number"),
        ("'text'.length", "Cannot read property `length` of a string"),
        ("'text'[0]", "Cannot read property `0` of a string"),
        (
            "function f() {} f.name",
            "Cannot read property `name` of a function",
        ),
        ("let n = null; n.x = 1", "Cannot set property `x` of null"),
        (
            "let s = 's'; s['x'] = 1",
            "Cannot set property `x` of a string",
        ),
    ] {
        let error = eval(source).expect_err(source);
        assert_eq!(error.category, JsErrorCategory::Runtime, "{source}");
        assert_eq!(error.message, message, "{source}");
    }
    let error = eval("null.x = 1").unwrap_err();
    assert_eq!(error.context.as_deref(), Some("property assignment"));
}

#[test]
fn calling_a_non_function_member_is_a_runtime_error() {
    let (realm, _) = realm_with_object();
    let error = runtime_error(&realm, "object.value()");
    assert_eq!(error.message, "`object.value` is not a function");
    assert_eq!(error.context.as_deref(), Some("function call"));
}

#[test]
fn errors_stop_the_script_without_partial_writes() {
    let (realm, object) = realm_with_object();
    runtime_error(&realm, "object.a = 1; object.missing.b = 2; object.c = 3;");
    assert_eq!(object.keys(), ["value", "a"]);
}

// Values ---------------------------------------------------------------------------------------

#[test]
fn objects_follow_javascript_conversions() {
    let (realm, _) = realm_with_object();
    for (source, expected) in [
        ("'' + object", Value::String("[object Object]".into())),
        ("!!object", Value::Boolean(true)),
        ("object == '[object Object]'", Value::Boolean(true)),
        ("object === '[object Object]'", Value::Boolean(false)),
        ("object == null", Value::Boolean(false)),
    ] {
        assert_eq!(realm.evaluate_script(source), Ok(expected), "{source}");
    }
    assert!(matches!(
        realm.evaluate_script("object - 1"),
        Ok(Value::Number(n)) if n.is_nan()
    ));
    // Using an object as a key uses its string form.
    assert_eq!(
        realm.evaluate_script("object[object] = 5; object['[object Object]']"),
        number(5.0)
    );
}

#[test]
fn separately_created_objects_are_never_equal() {
    let (first, second) = (Object::new(), Object::new());
    assert_ne!(first, second);
    assert_eq!(first, first.clone());

    let mut realm = Realm::new();
    realm
        .register_global_value("a", Value::Object(first))
        .unwrap();
    realm
        .register_global_value("b", Value::Object(second))
        .unwrap();
    assert_eq!(realm.evaluate_script("a == b"), Ok(Value::Boolean(false)));
}

#[test]
fn object_api_reports_its_properties() {
    let object = Object::new();
    assert!(object.is_empty());
    object.set("a", Value::Number(1.0));
    object.set("b", Value::Null);
    object.set("a", Value::Number(2.0));
    assert_eq!(object.len(), 2);
    assert_eq!(object.keys(), ["a", "b"]);
    assert_eq!(object.get("a"), Some(Value::Number(2.0)));
    assert!(object.contains_key("b"));
    assert_eq!(object.get("c"), None);
    assert_eq!(Value::Object(object).to_string(), "[object Object]");
}

// Realm registration -------------------------------------------------------------------------------

#[test]
fn global_values_replace_functions_with_the_same_name_and_vice_versa() {
    let mut realm = Realm::new();
    realm
        .register_global_function("thing", Arc::new(|_: &[Value]| Ok(Value::Null)))
        .unwrap();
    realm
        .register_global_value("thing", Value::Number(1.0))
        .unwrap();
    assert_eq!(realm.evaluate_script("thing"), number(1.0));
    assert!(realm.call_global("thing", &[]).is_err());

    realm
        .register_global_function("thing", Arc::new(|_: &[Value]| Ok(Value::Null)))
        .unwrap();
    assert_eq!(realm.evaluate_script("thing()"), Ok(Value::Null));
}

#[test]
fn global_value_names_cannot_be_empty() {
    let error = Realm::new()
        .register_global_value("", Value::Null)
        .unwrap_err();
    assert_eq!(error.message, "global value name cannot be empty");
}

#[test]
fn scripts_can_shadow_global_objects() {
    let (realm, object) = realm_with_object();
    assert_eq!(realm.evaluate_script("let object = 5; object"), number(5.0));
    // Reassigning the global name affects only that script.
    realm.evaluate_script("object = 1").unwrap();
    assert_eq!(realm.evaluate_script("object.value"), number(1.0));
    assert_eq!(object.get("value"), Some(Value::Number(1.0)));
}
