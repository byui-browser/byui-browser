//! The supported identity-handle subset of the well-known disposal symbol.

use js::{ExecutionMode, Realm, Value, eval_with_mode};

#[test]
fn dispose_has_stable_identity_description_and_read_only_binding() {
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        for source in [
            "Symbol.dispose === Symbol.dispose",
            "Symbol.dispose !== Symbol('Symbol.dispose')",
            "Symbol.dispose.description === 'Symbol.dispose'",
            "var original = Symbol.dispose; Symbol.dispose = 7; Symbol.dispose === original",
        ] {
            assert_eq!(
                eval_with_mode(source, mode).unwrap(),
                Value::Boolean(true),
                "{source}"
            );
        }
    }
    let first = Realm::new().evaluate_script("Symbol.dispose").unwrap();
    let second = Realm::new().evaluate_script("Symbol.dispose").unwrap();
    assert_eq!(first, second);
}
