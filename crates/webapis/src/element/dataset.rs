//! `Element.dataset` on script element wrappers.
//!
//! The getter returns one [`DOMStringMap`](https://html.spec.whatwg.org/#domstringmap)
//! for the element. Each `data-*` attribute is a camel-cased property:
//! `data-foo-bar` is `dataset.fooBar`. Reading, writing, and `delete` update
//! the attribute. Setting a name that contains `-` followed by an ASCII
//! lowercase letter throws `SyntaxError`. A name that does not produce a
//! valid attribute throws `InvalidCharacterError`.
//!
//! The same map object is returned on every read. `ToString` uses the
//! engine's conversion and does not call a user-defined `toString`.
//! `attributeChangedCallback` runs when the stored attribute value changes.
//!
//! Attributes whose names contain ASCII uppercase are skipped. This document
//! stores HTML attribute names in lowercase, so that case does not come up
//! for parsed or `setAttribute` names.

use std::sync::{Arc, Mutex};

use html::NodeID;
use js::{JsResult, Object, Value, WeakObject};

use crate::document::Document;
use crate::element::Element;

use super::{invoke_attribute_changed_callback, thrown_exception};

/// Why a dataset property name cannot be stored.
#[derive(Debug)]
enum NameError {
    /// The name contains `-` followed by an ASCII lowercase letter.
    Syntax,
}

/// Installs `Element.dataset` on the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let map = dataset_object(element.node, document, &object.downgrade());
    object.define_accessor(
        "dataset",
        Arc::new(move || Ok(Value::Object(map.clone()))),
        None,
    );
}

/// Builds the live `DOMStringMap` for `node`.
fn dataset_object(node: NodeID, document: &Arc<Mutex<Document>>, owner: &WeakObject) -> Object {
    let map = Object::new();
    let read_document = Arc::clone(document);
    map.set_property_hooks(
        Arc::new(move |property| {
            Ok(pairs(&read_document, node)
                .into_iter()
                .find(|(name, _)| name == property)
                .map(|(_, value)| Value::String(value)))
        }),
        Arc::new({
            let document = Arc::clone(document);
            let owner = owner.clone();
            move |property, value| {
                set_pair(&document, &owner, node, property, &value.to_string())?;
                Ok(true)
            }
        }),
        Arc::new({
            let document = Arc::clone(document);
            move || {
                Ok(pairs(&document, node)
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect())
            }
        }),
    );
    let delete_document = Arc::clone(document);
    let delete_owner = owner.clone();
    map.set_property_deleter(Arc::new(move |property| {
        if pairs(&delete_document, node)
            .iter()
            .any(|(name, _)| name == property)
        {
            let attribute = attribute_name(property).expect("a listed dataset name converts");
            remove_pair(&delete_document, &delete_owner, node, &attribute)?;
        }
        Ok(Some(true))
    }));
    map
}

/// Returns the element's dataset name/value pairs in attribute order.
fn pairs(document: &Arc<Mutex<Document>>, node: NodeID) -> Vec<(String, String)> {
    let Some(element) = document.lock().unwrap().element_snapshot(node) else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    for attribute in &element.attributes {
        let Some(rest) = attribute.get_name().strip_prefix("data-") else {
            continue;
        };
        if rest.chars().any(|character| character.is_ascii_uppercase()) {
            continue;
        }
        let name = camel_case(rest);
        if pairs.iter().any(|(existing, _)| existing == &name) {
            continue;
        }
        pairs.push((name, attribute.get_value().clone()));
    }
    pairs
}

/// Sets `data-*` for `property` to `value` and notifies a custom element.
fn set_pair(
    document: &Arc<Mutex<Document>>,
    owner: &WeakObject,
    node: NodeID,
    property: &str,
    value: &str,
) -> JsResult<()> {
    let attribute = attribute_name(property).map_err(|NameError::Syntax| {
        thrown_exception(
            "SyntaxError",
            "dataset names must not contain '-' followed by a lowercase letter",
        )
    })?;
    let old_value = attribute_value(document, node, &attribute);
    match document
        .lock()
        .unwrap()
        .set_attribute(node, &attribute, value)
    {
        Ok(_) => {}
        Err(error) if error.to_string().contains("InvalidCharacterError") => {
            return Err(thrown_exception(
                "InvalidCharacterError",
                "the dataset name is not a valid attribute name",
            ));
        }
        Err(error) => return Err(error),
    }
    let new_value = attribute_value(document, node, &attribute);
    if old_value != new_value
        && let Some(owner) = owner.upgrade()
    {
        invoke_attribute_changed_callback(&owner, &attribute, old_value, new_value);
    }
    log::debug!("Element.dataset set {attribute}");
    Ok(())
}

/// Removes `attribute` and notifies a custom element when it was present.
fn remove_pair(
    document: &Arc<Mutex<Document>>,
    owner: &WeakObject,
    node: NodeID,
    attribute: &str,
) -> JsResult<()> {
    let old_value = attribute_value(document, node, attribute);
    document.lock().unwrap().remove_attribute(node, attribute)?;
    if old_value.is_some()
        && let Some(owner) = owner.upgrade()
    {
        invoke_attribute_changed_callback(&owner, attribute, old_value, None);
    }
    log::debug!("Element.dataset deleted {attribute}");
    Ok(())
}

/// Returns the current value of `attribute`, if the element has it.
fn attribute_value(
    document: &Arc<Mutex<Document>>,
    node: NodeID,
    attribute: &str,
) -> Option<String> {
    document
        .lock()
        .unwrap()
        .element_snapshot(node)
        .and_then(|element| element.get_attribute(attribute).map(str::to_owned))
}

/// Converts a `data-*` suffix to its dataset property name.
fn camel_case(name: &str) -> String {
    let characters: Vec<char> = name.chars().collect();
    let mut output = String::new();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == '-'
            && characters
                .get(index + 1)
                .is_some_and(|character| character.is_ascii_lowercase())
        {
            output.push(characters[index + 1].to_ascii_uppercase());
            index += 2;
        } else {
            output.push(characters[index]);
            index += 1;
        }
    }
    output
}

/// Converts a dataset property name to its `data-*` attribute name.
fn attribute_name(name: &str) -> Result<String, NameError> {
    let characters: Vec<char> = name.chars().collect();
    for (index, character) in characters.iter().enumerate() {
        if *character == '-'
            && characters
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_lowercase())
        {
            return Err(NameError::Syntax);
        }
    }
    let mut attribute = String::from("data-");
    for character in characters {
        if character.is_ascii_uppercase() {
            attribute.push('-');
            attribute.push(character.to_ascii_lowercase());
        } else {
            attribute.push(character);
        }
    }
    Ok(attribute)
}

#[cfg(test)]
mod tests {
    use html::parse_raw_html;
    use js::{Realm, Value};

    use super::attribute_name;
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
    fn dataset_reads_camel_cased_data_attributes() {
        let mut realm =
            realm_with("<div id='a' data-foo-bar='1' data-baz='2' data-foo-='tail'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var el = document.getElementById('a'); \
                 el.dataset === el.dataset && el.dataset.fooBar === '1' && el.dataset.baz === '2' \
                 && el.dataset['foo-'] === 'tail' && el.dataset.missing === undefined"
            ),
            Value::Boolean(true)
        );
        assert_eq!(eval(&mut realm, "({...el.dataset}).fooBar"), string("1"));
    }

    #[test]
    fn dataset_writes_and_deletes_attributes() {
        let mut realm = realm_with("<div id='a' data-foo-bar='1' data-baz='2'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var el = document.getElementById('a'); \
                 el.dataset.fooBar = '9'; el.dataset.newProp = 'x'; \
                 el.getAttribute('data-foo-bar') + '|' + el.getAttribute('data-new-prop') \
                 + '|' + (delete el.dataset.baz) + '|' + el.getAttribute('data-baz') \
                 + '|' + (delete el.dataset.missing)"
            ),
            string("9|x|true|null|true")
        );
    }

    #[test]
    fn dataset_rejects_ambiguous_and_invalid_names() {
        let mut realm = realm_with("<div id='a'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('a').dataset['foo-bar'] = 'x'; } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("SyntaxError")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('a').dataset['a b'] = 'x'; } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("InvalidCharacterError")
        );
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('a').getAttribute('data-foo-bar')"
            ),
            Value::Null
        );
    }

    #[test]
    fn attribute_name_round_trips_camel_case() {
        assert_eq!(attribute_name("fooBar").unwrap(), "data-foo-bar");
        assert_eq!(attribute_name("foo-").unwrap(), "data-foo-");
        assert!(attribute_name("foo-bar").is_err());
    }
}
