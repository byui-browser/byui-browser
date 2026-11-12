use js::{Realm, Value};
use webapis::register_window;

#[test]
fn window_crypto_is_stable_and_has_a_global_alias() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.crypto === crypto && window.crypto === window.crypto && \
             window.crypto.subtle === window.crypto.subtle",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn get_random_values_fills_and_returns_the_same_array() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "let bytes = [0, 0, 0, 0]; \
             crypto.getRandomValues(bytes) === bytes && \
             bytes.length === 4 && bytes[0] >= 0 && bytes[0] <= 255 && \
             bytes[1] >= 0 && bytes[1] <= 255",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn random_uuid_has_version_four_and_rfc_variant_bits() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    let Ok(Value::String(uuid)) = realm.evaluate_script("crypto.randomUUID()") else {
        panic!("randomUUID() should return a string");
    };
    assert_eq!(uuid.len(), 36);
    assert_eq!(uuid.as_bytes()[14], b'4');
    assert!(matches!(uuid.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
}
