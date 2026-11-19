//! URI component encoding in both execution engines and persistent realms.

use js::{ExecutionMode, Realm, Value, eval_with_mode};

#[test]
fn encodes_utf8_and_reserved_characters_without_form_encoding() {
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        for (source, expected) in [
            ("encodeURIComponent(\"AZaz09-_.!~*'()\")", "AZaz09-_.!~*'()"),
            (
                "encodeURIComponent('a b+c/d?x=y&z=#%')",
                "a%20b%2Bc%2Fd%3Fx%3Dy%26z%3D%23%25",
            ),
            (
                "encodeURIComponent('café € 😀')",
                "caf%C3%A9%20%E2%82%AC%20%F0%9F%98%80",
            ),
            ("encodeURIComponent('')", ""),
            ("encodeURIComponent()", "undefined"),
            ("encodeURIComponent(null)", "null"),
            ("encodeURIComponent(42)", "42"),
            ("encodeURIComponent(true)", "true"),
            ("encodeURIComponent('%20')", "%2520"),
        ] {
            assert_eq!(
                eval_with_mode(source, mode).unwrap(),
                Value::String(expected.into()),
                "{mode:?}: {source}"
            );
        }
    }
}

#[test]
fn encoding_round_trips_and_is_available_in_persistent_realms() {
    let realm = Realm::new();
    realm
        .evaluate_script("var encoded = encodeURIComponent('a/b 😀')")
        .unwrap();
    assert_eq!(
        realm
            .evaluate_script("decodeURIComponent(encoded)")
            .unwrap(),
        Value::String("a/b 😀".into())
    );
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        assert_eq!(
            eval_with_mode("decodeURIComponent(encodeURIComponent('a/b 😀'))", mode).unwrap(),
            Value::String("a/b 😀".into())
        );
    }
}
