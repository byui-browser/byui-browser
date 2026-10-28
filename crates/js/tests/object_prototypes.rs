//! Prototype chains on [`js::Object`].

use js::{Object, Value};

fn number(value: f64) -> Value {
    Value::Number(value)
}

#[test]
fn get_falls_back_to_the_prototype_chain() {
    let grandparent = Object::new();
    grandparent.set("a", number(1.0));
    let parent = Object::with_prototype(Some(grandparent.clone()));
    parent.set("b", number(2.0));
    let child = Object::with_prototype(Some(parent.clone()));
    child.set("c", number(3.0));

    assert_eq!(child.get("a"), Some(number(1.0)));
    assert_eq!(child.get("b"), Some(number(2.0)));
    assert_eq!(child.get("c"), Some(number(3.0)));
    assert_eq!(child.get("missing"), None);
    assert_eq!(child.prototype(), Some(parent));
}

#[test]
fn own_properties_shadow_and_writes_stay_on_the_object() {
    let prototype = Object::new();
    prototype.set("x", number(1.0));
    let child = Object::with_prototype(Some(prototype.clone()));

    child.set("x", number(2.0));

    assert_eq!(child.get("x"), Some(number(2.0)));
    assert_eq!(prototype.get("x"), Some(number(1.0)));
    assert_eq!(child.keys(), ["x"]);
}

#[test]
fn own_lookups_ignore_the_prototype() {
    let prototype = Object::new();
    prototype.set("x", number(1.0));
    let child = Object::with_prototype(Some(prototype));

    assert_eq!(child.get_own("x"), None);
    assert!(!child.contains_key("x"));
    assert_eq!(child.len(), 0);
    assert!(child.is_empty());
}

#[test]
fn changing_a_prototype_is_visible_through_existing_children() {
    let prototype = Object::new();
    let child = Object::with_prototype(Some(prototype.clone()));
    assert_eq!(child.get("late"), None);

    prototype.set("late", number(1.0));

    assert_eq!(child.get("late"), Some(number(1.0)));
}

#[test]
fn prototype_cycles_are_rejected() {
    let first = Object::new();
    let second = Object::with_prototype(Some(first.clone()));
    let third = Object::with_prototype(Some(second.clone()));

    assert!(!first.set_prototype(Some(third.clone())));
    assert!(!first.set_prototype(Some(first.clone())));
    assert_eq!(first.prototype(), None, "a rejected change leaves no link");
    assert!(first.set_prototype(None));
    assert!(third.set_prototype(None));
    assert_eq!(first.get("anything"), None, "lookups still terminate");
}

#[test]
fn plain_objects_have_no_prototype() {
    assert_eq!(Object::new().prototype(), None);
    assert_eq!(Object::default().prototype(), None);
}
