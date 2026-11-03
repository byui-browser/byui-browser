use js::{Realm, Value};
use webapis::register_window;

#[test]
fn window_self_returns_the_current_window_object() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.self() === window"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn global_self_aliases_window() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("self === window && self.self() === window"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_self_accepts_extra_arguments_without_changing_the_result() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.self(1, 'ignored') === window"),
        Ok(Value::Boolean(true))
    );
}
