//! Persistent global state across `Realm::evaluate_script` calls: bindings,
//! redeclaration errors, host globals, isolation between realms, and reset.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use js::{JsError, Object, Realm, Value};

fn number(value: f64) -> Result<Value, JsError> {
    Ok(Value::Number(value))
}

fn error(realm: &Realm, source: &str) -> String {
    realm.evaluate_script(source).unwrap_err().message
}

#[test]
fn var_and_function_bindings_persist_and_are_callable() {
    let realm = Realm::new();
    realm
        .evaluate_script("var total = 40; function add(a, b) { return a + b; }")
        .unwrap();
    assert_eq!(realm.evaluate_script("add(total, 2)"), number(42.0));
    realm.evaluate_script("total = 1").unwrap();
    assert_eq!(realm.evaluate_script("total"), number(1.0));
}

#[test]
fn persisted_functions_see_globals_declared_later() {
    let realm = Realm::new();
    realm
        .evaluate_script("function read() { return later; }")
        .unwrap();
    realm.evaluate_script("let later = 7;").unwrap();
    assert_eq!(realm.evaluate_script("read()"), number(7.0));
}

#[test]
fn function_and_block_scopes_stay_isolated() {
    let realm = Realm::new();
    realm
        .evaluate_script("function f() { var inner = 1; return inner; } { let scoped = 2; }")
        .unwrap();
    assert_eq!(realm.evaluate_script("f()"), number(1.0));
    assert_eq!(error(&realm, "inner"), "`inner` is not defined");
    assert_eq!(error(&realm, "scoped"), "`scoped` is not defined");
}

#[test]
fn let_and_const_persist_and_const_stays_constant() {
    let realm = Realm::new();
    realm.evaluate_script("let a = 1; const b = 2;").unwrap();
    realm.evaluate_script("a = a + 1").unwrap();
    assert_eq!(realm.evaluate_script("a + b"), number(4.0));
    assert_eq!(error(&realm, "b = 3"), "Assignment to constant `b`");
}

#[test]
fn redeclaring_var_and_function_is_allowed() {
    let realm = Realm::new();
    realm
        .evaluate_script("var x = 1; function f() { return 1; }")
        .unwrap();
    // A bare `var` keeps the existing value.
    realm.evaluate_script("var x;").unwrap();
    assert_eq!(realm.evaluate_script("x"), number(1.0));
    realm
        .evaluate_script("var x = 2; function f() { return 2; }")
        .unwrap();
    assert_eq!(realm.evaluate_script("x + f()"), number(4.0));
}

#[test]
fn lexical_redeclaration_is_reported_consistently() {
    let realm = Realm::new();
    realm
        .evaluate_script("let l = 1; const c = 2; var v = 3; function fn1() {}")
        .unwrap();
    for source in [
        "let l = 9;",
        "const l = 9;",
        "let c = 9;",
        "let v = 9;",
        "const v = 9;",
        "let fn1 = 9;",
        "var l = 9;",
        "var c = 9;",
        "function l() {}",
        "{ var l = 9; }",
    ] {
        let name = source
            .split(|ch: char| !ch.is_alphanumeric())
            .find(|word| ["l", "c", "v", "fn1"].contains(word))
            .unwrap();
        assert_eq!(
            error(&realm, source),
            format!("`{name}` has already been declared"),
            "{source}"
        );
    }
    // Original bindings are untouched.
    assert_eq!(realm.evaluate_script("l + c + v"), number(6.0));
}

#[test]
fn duplicate_lexical_names_within_one_script_are_rejected() {
    let realm = Realm::new();
    assert_eq!(
        error(&realm, "let d = 1; let d = 2;"),
        "`d` has already been declared"
    );
    assert_eq!(
        error(&realm, "let e = 1; var e = 2;"),
        "`e` has already been declared"
    );
}

#[test]
fn failed_redeclaration_declares_nothing_and_runs_nothing() {
    let realm = Realm::new();
    realm.evaluate_script("let taken = 1;").unwrap();
    assert!(
        realm
            .evaluate_script("var fresh = 1; let other = 2; let taken = 3;")
            .is_err()
    );
    assert_eq!(error(&realm, "fresh"), "`fresh` is not defined");
    assert_eq!(error(&realm, "other"), "`other` is not defined");
    assert_eq!(realm.evaluate_script("taken"), number(1.0));
}

#[test]
fn runtime_error_keeps_earlier_effects_and_leaves_let_uninitialized() {
    let realm = Realm::new();
    assert!(
        realm
            .evaluate_script("var before = 1; missing(); let after = 2;")
            .is_err()
    );
    assert_eq!(realm.evaluate_script("before"), number(1.0));
    assert_eq!(
        error(&realm, "after"),
        "Cannot access `after` before initialization"
    );
}

#[test]
fn assigning_an_undeclared_name_still_fails() {
    let realm = Realm::new();
    assert_eq!(error(&realm, "ghost = 1"), "`ghost` is not defined");
    assert_eq!(error(&realm, "ghost"), "`ghost` is not defined");
}

#[test]
fn realms_do_not_share_script_state() {
    let first = Realm::new();
    let second = Realm::new();
    first
        .evaluate_script("var shared = 1; let only = 2;")
        .unwrap();
    assert_eq!(error(&second, "shared"), "`shared` is not defined");
    second
        .evaluate_script("let only = 3; var shared = 4;")
        .unwrap();
    assert_eq!(first.evaluate_script("shared + only"), number(3.0));
    assert_eq!(second.evaluate_script("shared + only"), number(7.0));
}

#[test]
fn host_and_script_globals_resolve_through_one_environment() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let mut realm = Realm::new();
    realm
        .register_global_function(
            "ping",
            Arc::new(move |_: &[Value]| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Undefined)
            }),
        )
        .unwrap();
    realm
        .evaluate_script("function twice() { ping(); ping(); }")
        .unwrap();
    realm.evaluate_script("twice()").unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(realm.call_global("ping", &[]), Ok(Value::Undefined));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn host_registration_after_scripts_is_visible_to_existing_functions() {
    let mut realm = Realm::new();
    realm
        .evaluate_script("function read() { return limit; }")
        .unwrap();
    realm
        .register_global_value("limit", Value::Number(5.0))
        .unwrap();
    assert_eq!(realm.evaluate_script("read()"), number(5.0));
    realm
        .register_global_value("limit", Value::Number(6.0))
        .unwrap();
    assert_eq!(realm.evaluate_script("read()"), number(6.0));
}

#[test]
fn host_global_assignment_and_var_redeclaration_persist() {
    let mut realm = Realm::new();
    realm
        .register_global_value("mode", Value::String("host".into()))
        .unwrap();
    realm.evaluate_script("var mode;").unwrap();
    assert_eq!(
        realm.evaluate_script("mode"),
        Ok(Value::String("host".into()))
    );
    realm.evaluate_script("mode = 'script'").unwrap();
    assert_eq!(
        realm.evaluate_script("mode"),
        Ok(Value::String("script".into()))
    );
    // Once a script has declared the name, `let` can no longer shadow it.
    assert_eq!(
        error(&realm, "let mode = 1;"),
        "`mode` has already been declared"
    );
}

#[test]
fn let_may_shadow_an_untouched_host_global() {
    let mut realm = Realm::new();
    realm
        .register_global_value("host", Value::Number(1.0))
        .unwrap();
    realm.evaluate_script("let host = 2;").unwrap();
    assert_eq!(realm.evaluate_script("host"), number(2.0));
}

#[test]
fn reset_script_state_drops_script_bindings_but_keeps_host_globals() {
    let console = Object::new();
    let mut realm = Realm::new();
    realm
        .register_global_value("console", Value::Object(console.clone()))
        .unwrap();
    realm
        .evaluate_script("var kept = 1; let lexical = 2; function f() {} console.seen = 1;")
        .unwrap();
    realm.evaluate_script("console = 5").unwrap();

    realm.reset_script_state();

    assert_eq!(error(&realm, "kept"), "`kept` is not defined");
    assert_eq!(error(&realm, "f"), "`f` is not defined");
    assert_eq!(
        realm.evaluate_script("let lexical = 3; lexical"),
        number(3.0)
    );
    // The host value is restored, and the shared object kept its property.
    assert_eq!(realm.evaluate_script("console.seen"), number(1.0));
}
