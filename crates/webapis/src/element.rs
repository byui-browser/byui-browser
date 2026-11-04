//! Script-visible DOM elements and their JavaScript object wrappers.

use std::sync::{Arc, Mutex};

use html::{Attribute, Namespace, NodeID};
use js::{JsError, JsResult, Object, Value};

use crate::document::Document;
use crate::event::{add_event_listener_at, dispatch_event_at, remove_event_listener_at};

/// Script-visible view of a DOM element.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Index of this element in its document's DOM arena.
    pub node: NodeID,
    /// The element's HTML local name, normalized to ASCII lowercase.
    pub local_name: String,
    /// Namespace of this element, used to apply the DOM's HTML name matching
    /// rules when attributes are read.
    pub namespace: Namespace,
    /// Value of the `id` attribute, if any.
    pub id_attr: Option<String>,
    /// Attributes in document order, including their original values.
    pub attributes: Vec<Attribute>,
}

impl Element {
    /// Returns the value of the first attribute matching `qualified_name`.
    ///
    /// This follows the DOM Standard's `Element.getAttribute()` algorithm:
    /// HTML elements in an HTML document match attribute names after ASCII
    /// lowercasing, while other namespaces use the name as provided. The
    /// returned value borrows this element's immutable attribute snapshot and
    /// is `None` when no matching attribute exists.
    pub fn get_attribute(&self, qualified_name: &str) -> Option<&str> {
        let name = match self.namespace {
            Namespace::Html => qualified_name.to_ascii_lowercase(),
            Namespace::Svg | Namespace::MathMl => qualified_name.to_owned(),
        };

        self.attributes
            .iter()
            .find(|attribute| attribute.get_name() == &name)
            .map(|attribute| attribute.get_value().as_str())
    }

    /// Sets an attribute according to the DOM Standard's name validation and
    /// namespace normalization rules.
    pub fn set_attribute(&mut self, qualified_name: &str, value: &str) -> JsResult<()> {
        let name = normalize_attribute_name(self.namespace, qualified_name)?;
        if let Some(attribute) = self
            .attributes
            .iter_mut()
            .find(|attribute| attribute.get_name() == &name)
        {
            attribute.set_value(value.to_owned());
        } else {
            self.attributes.push(Attribute::new(name, value.to_owned()));
        }
        self.id_attr = self
            .attributes
            .iter()
            .find(|attribute| attribute.get_name() == "id")
            .map(|attribute| attribute.get_value().clone());
        Ok(())
    }
}

pub(crate) fn element_object(element: Element, document: Arc<Mutex<Document>>) -> Object {
    let object = Object::new();
    let node = element.node;
    object.set("__nodeId", Value::Number(node.get() as f64));
    object.set("localName", Value::String(element.local_name.clone()));
    let style_value = element
        .get_attribute("style")
        .map_or_else(String::new, str::to_owned);
    let style = Object::new();
    style.set("cssText", Value::String(style_value.clone()));
    for (name, value) in parse_inline_style(&style_value) {
        style.set(css_property_to_js(&name), Value::String(value));
    }
    object.set("style", Value::Object(style));
    let get_attribute_document = document.clone();
    object.set_method(
        "getAttribute",
        Arc::new(move |arguments| {
            let document = get_attribute_document.lock().unwrap();
            let Some(element) = document.element_snapshot(node) else {
                return Ok(Value::Null);
            };
            get_attribute(&element, arguments)
        }),
    );
    let set_attribute_document = document.clone();
    let set_attribute_target = object.clone();
    object.set_method(
        "setAttribute",
        Arc::new(move |arguments| {
            let [Value::String(qualified_name), Value::String(value)] = arguments else {
                return Err(JsError::new(
                    "Element.setAttribute() expects a qualified name and value string",
                ));
            };
            let old_value = {
                let document = set_attribute_document.lock().unwrap();
                document
                    .element_snapshot(node)
                    .and_then(|element| element.get_attribute(qualified_name).map(str::to_owned))
            };
            set_attribute_document
                .lock()
                .unwrap()
                .set_attribute(node, qualified_name, value)?;
            let new_value = set_attribute_document
                .lock()
                .unwrap()
                .element_snapshot(node)
                .and_then(|element| element.get_attribute(qualified_name).map(str::to_owned));
            invoke_attribute_changed_callback(
                &set_attribute_target,
                qualified_name,
                old_value,
                new_value,
            );
            Ok(Value::Undefined)
        }),
    );
    let append_document = document.clone();
    object.set_method(
        "appendChild",
        Arc::new(move |arguments| {
            let ([Value::Object(child)] | [Value::Array(child)]) = arguments else {
                return Err(JsError::new("Element.appendChild() expects an element"));
            };
            let Some(Value::Number(child_id)) = child.get("__nodeId") else {
                return Err(JsError::new("Element.appendChild() expects an element"));
            };
            let child_id = html::NodeID::new(child_id as usize);
            let was_connected = append_document.lock().unwrap().is_connected(child_id);
            if was_connected {
                invoke_lifecycle_callback(child, "disconnectedCallback");
            }
            append_document
                .lock()
                .unwrap()
                .append_child_to(node, child_id)?;
            invoke_lifecycle_callback(child, "connectedCallback");
            Ok(Value::Object(child.clone()))
        }),
    );
    let remove_document = document.clone();
    let remove_target = object.clone();
    object.set_method(
        "remove",
        Arc::new(move |_| {
            let was_connected = remove_document.lock().unwrap().is_connected(node);
            remove_document.lock().unwrap().remove(node);
            if was_connected {
                invoke_lifecycle_callback(&remove_target, "disconnectedCallback");
            }
            Ok(Value::Undefined)
        }),
    );
    let add_document = document.clone();
    let add_node = element.node;
    object.set_method(
        "addEventListener",
        Arc::new(move |arguments| {
            let document = add_document.lock().unwrap();
            add_event_listener_at(&document, Some(add_node), arguments)
        }),
    );
    let remove_document = document.clone();
    let remove_node = element.node;
    object.set_method(
        "removeEventListener",
        Arc::new(move |arguments| {
            let document = remove_document.lock().unwrap();
            remove_event_listener_at(&document, Some(remove_node), arguments)
        }),
    );
    let dispatch_document = document.clone();
    let dispatch_node = element.node;
    let dispatch_target = object.clone();
    object.set_method(
        "dispatchEvent",
        Arc::new(move |arguments| {
            let document = dispatch_document.lock().unwrap();
            dispatch_event_at(&document, Some(dispatch_node), &dispatch_target, arguments)
        }),
    );
    object
}

pub(crate) fn invoke_lifecycle_callback(element: &Object, name: &str) {
    if let Some(Value::Function(callback)) = element.get(name) {
        let _ = callback.call_with_this(Value::Object(element.clone()), &[]);
    }
}

pub(crate) fn invoke_attribute_changed_callback(
    element: &Object,
    name: &str,
    old_value: Option<String>,
    new_value: Option<String>,
) {
    let Some(prototype) = element.prototype() else {
        return;
    };
    let Some(Value::Function(constructor)) = prototype.get("constructor") else {
        return;
    };
    let Some(Value::Array(observed)) = constructor.get_own_property("observedAttributes") else {
        return;
    };
    let length = match observed.get("length") {
        Some(Value::Number(length)) if length >= 0.0 => length as usize,
        _ => return,
    };
    let observes_name = (0..length).any(|index| {
        matches!(observed.get(&index.to_string()), Some(Value::String(value)) if value == name)
    });
    if !observes_name {
        return;
    }
    let Some(Value::Function(callback)) = element.get("attributeChangedCallback") else {
        return;
    };
    let _ = callback.call_with_this(
        Value::Object(element.clone()),
        &[
            Value::String(name.to_owned()),
            old_value.map_or(Value::Null, Value::String),
            new_value.map_or(Value::Null, Value::String),
        ],
    );
}

/// Reads declarations from an HTML `style` attribute.
fn parse_inline_style(style: &str) -> Vec<(String, String)> {
    let mut declarations = Vec::new();
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        if property.is_empty() || value.is_empty() {
            continue;
        }
        if let Some(existing) = declarations.iter_mut().find(|(name, _)| name == &property) {
            existing.1 = value.to_owned();
        } else {
            declarations.push((property, value.to_owned()));
        }
    }
    declarations
}

fn css_property_to_js(property: &str) -> String {
    let mut result = String::with_capacity(property.len());
    let mut uppercase_next = false;
    for character in property.chars() {
        if character == '-' {
            uppercase_next = true;
        } else if uppercase_next {
            result.extend(character.to_uppercase());
            uppercase_next = false;
        } else {
            result.push(character);
        }
    }
    result
}

fn normalize_attribute_name(namespace: Namespace, qualified_name: &str) -> JsResult<String> {
    if qualified_name.is_empty()
        || qualified_name.chars().any(|character| {
            matches!(
                character,
                '\t' | '\n' | '\x0c' | '\r' | ' ' | '\0' | '/' | '=' | '>'
            )
        })
    {
        return Err(JsError::new("InvalidCharacterError"));
    }

    Ok(match namespace {
        Namespace::Html => qualified_name.to_ascii_lowercase(),
        Namespace::Svg | Namespace::MathMl => qualified_name.to_owned(),
    })
}

/// Implements JavaScript `Element.getAttribute(qualifiedName)`.
pub fn get_attribute(element: &Element, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(qualified_name)] = arguments else {
        return Err(JsError::new(
            "Element.getAttribute() expects one qualified name string",
        ));
    };

    Ok(match element.get_attribute(qualified_name) {
        Some(value) => Value::String(value.to_owned()),
        None => Value::Null,
    })
}

#[cfg(test)]
mod tests {
    use html::{Namespace, NodeID, parse_raw_html};
    use js::{Realm, Value};

    use super::*;
    use crate::document::{Document, register_document};

    #[test]
    fn registration_exposes_element_get_attribute_to_javascript() {
        let html_document = parse_raw_html("<main id='app' data-state='ready'></main>".to_owned());
        let mut realm = Realm::new();
        register_document(&mut realm, Document::from_html_document(&html_document)).unwrap();

        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').getAttribute('DATA-STATE')")
                .unwrap(),
            Value::String("ready".to_owned())
        );
        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').getAttribute('missing')")
                .unwrap(),
            Value::Null
        );
    }

    #[test]
    fn element_style_exposes_inline_css2_declarations_as_camel_case_properties() {
        let html_document = parse_raw_html(
            "<main id='app' style='color: red; background-color: blue; color: green; broken'></main>"
                .to_owned(),
        );
        let mut realm = Realm::new();
        register_document(&mut realm, Document::from_html_document(&html_document)).unwrap();

        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').style.color")
                .unwrap(),
            Value::String("green".to_owned())
        );
        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').style.backgroundColor")
                .unwrap(),
            Value::String("blue".to_owned())
        );
        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').style.cssText")
                .unwrap(),
            Value::String("color: red; background-color: blue; color: green; broken".to_owned())
        );
    }

    #[test]
    fn registration_exposes_element_set_attribute_to_javascript() {
        let html_document = parse_raw_html("<main id='app' data-state='ready'></main>".to_owned());
        let mut realm = Realm::new();
        register_document(&mut realm, Document::from_html_document(&html_document)).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "document.getElementById('app').setAttribute('DATA-STATE', 'updated'); document.getElementById('app').getAttribute('data-state')",
            ),
            Ok(Value::String("updated".to_owned()))
        );
        assert_eq!(
            realm.evaluate_script(
                "document.getElementById('app').setAttribute('data-added', 'yes')",
            ),
            Ok(Value::Undefined)
        );
        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').getAttribute('DATA-ADDED')")
                .unwrap(),
            Value::String("yes".to_owned())
        );
    }

    #[test]
    fn javascript_can_create_and_append_an_element_to_the_document_tree() {
        let html_document = parse_raw_html("<main id='root'></main>".to_owned());
        let mut realm = Realm::new();
        register_document(&mut realm, Document::from_html_document(&html_document)).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "var child = document.createElement('x-panel'); child.setAttribute('id', 'created'); document.getElementById('root').appendChild(child); document.getElementById('created') !== null"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn element_get_attribute_matches_html_names_case_insensitively() {
        let document = Document::from_html_document(&parse_raw_html(
            "<main id='app' data-state='ready' data-state='first'></main>".to_owned(),
        ));
        let element = document.get_element_by_id("app").expect("app element");

        assert_eq!(element.get_attribute("DATA-STATE"), Some("ready"));
        assert_eq!(element.get_attribute("missing"), None);
    }

    #[test]
    fn element_set_attribute_rejects_invalid_names() {
        let mut element = Element {
            node: NodeID::new(1),
            local_name: "main".into(),
            namespace: Namespace::Html,
            id_attr: None,
            attributes: Vec::new(),
        };
        assert_eq!(
            element
                .set_attribute("bad name", "value")
                .unwrap_err()
                .to_string(),
            "InvalidCharacterError"
        );
        assert!(element.attributes.is_empty());
    }

    #[test]
    fn created_element_get_attribute_returns_none_without_attributes() {
        let mut document = Document::new();
        let element = document.create_element("custom-widget").unwrap();

        assert_eq!(element.get_attribute("data-state"), None);
    }
}
