//! `Element.className` on script element wrappers.
//!
//! The getter reflects the `class` content attribute and returns `""` when
//! the attribute is absent. The setter replaces that attribute with the
//! argument converted by the engine's `ToString` (it does not call a
//! user-defined `toString`). `classList` reads the same attribute, so the two
//! stay in step.
//!
//! Limitation: `attributeChangedCallback` runs only when the stored value
//! changes, not when a setter writes the value the attribute already has.

use std::sync::{Arc, Mutex};

use html::NodeID;
use js::{JsResult, Object, Value, WeakObject};

use crate::document::Document;
use crate::element::Element;

use super::invoke_attribute_changed_callback;

/// Installs `Element.className` on the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let getter_document = Arc::clone(document);
    let setter_document = Arc::clone(document);
    let node = element.node;
    let owner = object.downgrade();
    object.define_accessor(
        "className",
        Arc::new(move || {
            Ok(Value::String(
                class_attribute(&getter_document, node).unwrap_or_default(),
            ))
        }),
        Some(Arc::new(move |value| {
            set_class_name(&setter_document, &owner, node, &value.to_string())
        })),
    );
}

/// Returns the `class` attribute value, or `None` when it is absent.
fn class_attribute(document: &Arc<Mutex<Document>>, node: NodeID) -> Option<String> {
    document
        .lock()
        .unwrap()
        .element_snapshot(node)
        .and_then(|element| element.get_attribute("class").map(str::to_owned))
}

/// Sets the `class` attribute and notifies a custom element when it changed.
fn set_class_name(
    document: &Arc<Mutex<Document>>,
    owner: &WeakObject,
    node: NodeID,
    value: &str,
) -> JsResult<()> {
    let old_value = class_attribute(document, node);
    document
        .lock()
        .unwrap()
        .set_attribute(node, "class", value)?;
    let new_value = class_attribute(document, node);
    if old_value != new_value
        && let Some(owner) = owner.upgrade()
    {
        invoke_attribute_changed_callback(&owner, "class", old_value, new_value);
    }
    log::debug!("Element.className set ({} bytes)", value.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use html::parse_raw_html;
    use js::{Realm, Value};

    use crate::document::{Document, register_document};

    fn realm_with(markup: &str) -> Realm {
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            Document::from_html_document(&parse_raw_html(markup.to_owned())),
        )
        .unwrap();
        realm
    }

    fn eval(realm: &mut Realm, script: &str) -> Value {
        realm
            .evaluate_script(script)
            .unwrap_or_else(|error| panic!("{script}: {error:?}"))
    }

    fn string(value: &str) -> Value {
        Value::String(value.to_owned())
    }

    #[test]
    fn class_name_reads_and_writes_the_class_attribute() {
        let mut realm = realm_with("<div id='a' class='one two'></div><p id='b'></p>");
        assert_eq!(
            eval(&mut realm, "document.getElementById('a').className"),
            string("one two")
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('b').className"),
            string("")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var el = document.getElementById('b'); el.className = 'x y'; \
                 el.className + '|' + el.getAttribute('class') + '|' + el.classList[1]"
            ),
            string("x y|x y|y")
        );
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('a').classList.add('three'); \
                 document.getElementById('a').className"
            ),
            string("one two three")
        );
    }

    #[test]
    fn class_name_stringifies_the_assigned_value() {
        let mut realm = realm_with("<div id='a'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var el = document.getElementById('a'); el.className = 12; el.getAttribute('class')"
            ),
            string("12")
        );
        assert_eq!(
            eval(&mut realm, "el.className = null; el.className"),
            string("null")
        );
    }
}
