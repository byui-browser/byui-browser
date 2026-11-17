use js::{Realm, Value, eval};

fn number(value: Value) -> f64 {
    match value {
        Value::Number(value) => value,
        other => panic!("expected number, got {other:?}"),
    }
}

#[test]
fn collections_are_available_in_a_persistent_realm() {
    let realm = Realm::new();
    assert_eq!(
        number(
            realm
                .evaluate_script("var map = new Map([[1, 'one']]); map.set(2, 'two'); map.size")
                .unwrap()
        ),
        2.0
    );
    assert_eq!(
        realm.evaluate_script("map.get(2)").unwrap(),
        Value::String("two".into())
    );
    assert_eq!(
        realm
            .evaluate_script("var set = new Set([1, 1, 2]); set.has(2)")
            .unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn standalone_vm_resolves_collection_globals() {
    assert_eq!(
        eval("var map = new Map(); map.set('key', 42); map.get('key')").unwrap(),
        Value::Number(42.0)
    );
    assert_eq!(
        eval("Reflect.get({ answer: 42 }, 'answer')").unwrap(),
        Value::Number(42.0)
    );
}

#[test]
fn symbols_have_identity() {
    assert_eq!(
        eval("var first = Symbol('x'); first === first").unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("Symbol('x') === Symbol('x')").unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn typed_arrays_store_bytes_and_reject_bad_lengths() {
    assert_eq!(
        eval("var bytes = new Uint8Array(2); bytes[0] = 257; bytes[0]").unwrap(),
        Value::Number(1.0)
    );
    assert!(eval("new Uint8Array(-1)").is_err());
    assert_eq!(
        eval("new ArrayBuffer(4).byteLength").unwrap(),
        Value::Number(4.0)
    );
}
