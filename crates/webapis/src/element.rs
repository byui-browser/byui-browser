//! Script-visible DOM elements and their JavaScript object wrappers.

use std::sync::{Arc, Mutex};

use html::{Attribute, Namespace, NodeID, NodeKind};
use js::{JsError, JsResult, Object, ScriptEvaluator, Value};

use crate::document::{Document, script_element_object};
use crate::node_tree::NodeInput;

// One submodule per Element API; each installs its members on the wrapper.
mod append;
mod children;
mod class_list;
mod class_name;
mod closest;
#[cfg(test)]
mod compare_document_position;
#[cfg(test)]
mod contains;
mod dataset;
#[cfg(test)]
mod first_child;
mod focus;
mod get_elements_by_class_name;
pub(crate) mod get_elements_by_tag_name;
mod has_attribute;
#[cfg(test)]
mod last_element_child;
#[cfg(test)]
mod node_type;
#[cfg(test)]
mod parent_element;
#[cfg(test)]
mod parent_node;
mod query_selector;

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
                log::debug!("getAttribute on a node that is no longer in the document");
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
                log::debug!("Element.setAttribute() called with invalid arguments; ignored");
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
            let evaluator = set_attribute_document
                .lock()
                .unwrap()
                .event_evaluator
                .clone();
            invoke_attribute_changed_callback(
                &set_attribute_target,
                qualified_name,
                old_value,
                new_value,
                evaluator.as_ref(),
            );
            Ok(Value::Undefined)
        }),
    );
    let remove_document = document.clone();
    let remove_target = object.clone();
    object.set_method(
        "remove",
        Arc::new(move |_| {
            let was_connected = remove_document.lock().unwrap().is_connected(node);
            let evaluator = remove_document.lock().unwrap().event_evaluator.clone();
            remove_document.lock().unwrap().remove(node);
            if was_connected {
                invoke_lifecycle_callback(
                    &remove_target,
                    "disconnectedCallback",
                    evaluator.as_ref(),
                );
            }
            Ok(Value::Undefined)
        }),
    );
    define_element_properties(&object, &element, &document);
    define_tree_methods(&object, node, &document);
    append::install(&object, &element, &document);
    children::install(&object, &element, &document);
    class_list::install(&object, &element, &document);
    class_name::install(&object, &element, &document);
    closest::install(&object, &element, &document);
    dataset::install(&object, &element, &document);
    focus::install(&object, &element, &document);
    get_elements_by_class_name::install(&object, &element, &document);
    get_elements_by_tag_name::install(&object, &element, &document);
    has_attribute::install(&object, &element, &document);
    query_selector::install(&object, &element, &document);
    crate::node::install(&object, node, &document);
    object
}

/// Defines `tagName`, `attributes`, `outerHTML`, `clientWidth`,
/// `clientHeight`, `ownerDocument`, and `querySelectorAll` on an element
/// wrapper.
fn define_element_properties(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    // An element's qualified name never changes, so `tagName` is a constant:
    // the HTML-uppercased qualified name for HTML elements, as-is otherwise.
    let tag_name = match element.namespace {
        Namespace::Html => element.local_name.to_ascii_uppercase(),
        Namespace::Svg | Namespace::MathMl => element.local_name.clone(),
    };
    object.set_read_only("tagName", Value::String(tag_name));
    let offset_parent_document = document.clone();
    object.define_accessor(
        "offsetParent",
        Arc::new(move || {
            let parent = {
                let document = offset_parent_document.lock().unwrap();
                let Some(source) = document.html_document() else {
                    return Ok(Value::Null);
                };
                let styles = document.computed_styles();
                if styles.value_for(node, "position") == Some("fixed") {
                    return Ok(Value::Null);
                }
                let mut current = source.get_node(node).and_then(|entry| entry.get_parent());
                let mut found = None;
                while let Some(id) = current {
                    let Some(entry) = source.get_node(id) else {
                        break;
                    };
                    if let NodeKind::Element(data) = entry.get_kind() {
                        let positioned = styles
                            .value_for(id, "position")
                            .is_some_and(|value| !value.eq_ignore_ascii_case("static"));
                        if positioned {
                            found = Some(id);
                            break;
                        }
                        if data.get_name() == "body" {
                            found = Some(id);
                            break;
                        }
                    }
                    current = entry.get_parent();
                }
                found
            };
            match parent {
                Some(parent) => wrap_element(&offset_parent_document, parent),
                None => Ok(Value::Null),
            }
        }),
        None,
    );
    let naturally_focusable = matches!(
        element.local_name.as_str(),
        "button" | "input" | "select" | "textarea" | "a"
    );
    let tab_document = document.clone();
    object.define_accessor(
        "tabIndex",
        Arc::new(move || {
            let value = tab_document
                .lock()
                .unwrap()
                .element_snapshot(node)
                .and_then(|element| element.get_attribute("tabindex").map(str::to_owned));
            let index = value
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or({
                    // The supported DOM subset treats common interactive controls as focusable.
                    if naturally_focusable { 0 } else { -1 }
                });
            Ok(Value::Number(index as f64))
        }),
        None,
    );

    let attributes_document = document.clone();
    let attributes_owner = object.clone();
    object.define_accessor(
        "attributes",
        Arc::new(move || {
            let element = attributes_document.lock().unwrap().element_snapshot(node);
            let attributes = element
                .map(|element| element.attributes)
                .unwrap_or_default();
            Ok(Value::Array(named_node_map(&attributes, &attributes_owner)))
        }),
        None,
    );

    let outer_html_document = document.clone();
    object.define_accessor(
        "outerHTML",
        Arc::new(move || {
            let html = outer_html_document.lock().unwrap().outer_html(node);
            Ok(Value::String(html.unwrap_or_default()))
        }),
        Some(Arc::new(|_| {
            log::debug!("Element.outerHTML setter is not supported");
            Err(JsError::new(
                "NotSupportedError: setting Element.outerHTML is not supported yet",
            ))
        })),
    );

    for (property, height) in [
        ("clientWidth", false),
        ("clientHeight", true),
        ("offsetWidth", false),
    ] {
        let client_document = document.clone();
        object.define_accessor(
            property,
            Arc::new(move || {
                let size = client_document.lock().unwrap().client_size(node);
                Ok(Value::Number(if height { size.height } else { size.width }))
            }),
            None,
        );
    }

    let query_document = document.clone();
    object.set_method(
        "querySelectorAll",
        Arc::new(move |arguments| {
            let Some(selector) = arguments.first() else {
                return Err(JsError::new(
                    "TypeError: Element.querySelectorAll() expects a selector string",
                ));
            };
            let selector = selector.to_string();
            let matches = query_document
                .lock()
                .unwrap()
                .query_selector_all_within(node, &selector)
                .map_err(|error| JsError::new(format!("SyntaxError: {error}")))?;
            // A static array stands in for the static `NodeList`.
            let list = Object::new();
            for (index, matched) in matches.iter().enumerate() {
                list.set(index.to_string(), wrap_element(&query_document, *matched)?);
            }
            list.set("length", Value::Number(matches.len() as f64));
            Ok(Value::Array(list))
        }),
    );
}

/// Defines `removeChild`, `replaceChild`, `replaceChildren`, and `before`.
fn define_tree_methods(object: &Object, node: NodeID, document: &Arc<Mutex<Document>>) {
    let replace_children_document = document.clone();
    object.set_method(
        "replaceChildren",
        Arc::new(move |arguments| {
            let (inputs, inserted) = node_inputs(arguments);
            let removed: Vec<Object> = {
                let document = replace_children_document.lock().unwrap();
                let children = document
                    .html_document()
                    .and_then(|source| source.get_node(node))
                    .map(|data| data.get_children().clone())
                    .unwrap_or_default();
                children
                    .into_iter()
                    .filter_map(|child| document.cached_script_element(child))
                    .collect()
            };
            mutate_tree(
                &replace_children_document,
                &inserted,
                &removed,
                |document| document.replace_children(node, &inputs),
            )?;
            Ok(Value::Undefined)
        }),
    );

    let before_document = document.clone();
    object.set_method(
        "before",
        Arc::new(move |arguments| {
            let (inputs, inserted) = node_inputs(arguments);
            mutate_tree(&before_document, &inserted, &[], |document| {
                document.before(node, &inputs)
            })?;
            Ok(Value::Undefined)
        }),
    );

    let after_document = document.clone();
    object.set_method(
        "after",
        Arc::new(move |arguments| {
            let (inputs, inserted) = node_inputs(arguments);
            {
                let document = after_document.lock().unwrap();
                let Some(source) = document.html_document() else {
                    return Ok(Value::Undefined);
                };
                let Some(_parent) = source.get_node(node).and_then(|entry| entry.get_parent())
                else {
                    return Ok(Value::Undefined);
                };
            }
            mutate_tree(&after_document, &inserted, &[], |document| {
                document.after(node, &inputs)
            })?;
            Ok(Value::Undefined)
        }),
    );
}

/// Builds a static `NamedNodeMap` snapshot: indexed `Attr`-like objects with
/// `length`, `item(index)`, and `getNamedItem(name)`.
///
/// Limitation: the DOM's map is live and its `Attr` nodes are shared objects;
/// this snapshot does not reflect later attribute changes, and writing an
/// `Attr`'s `value` does not change the element.
fn named_node_map(attributes: &[Attribute], owner: &Object) -> Object {
    let map = Object::new();
    let attrs: Vec<Object> = attributes
        .iter()
        .map(|attribute| {
            let attr = Object::new();
            attr.set("name", Value::String(attribute.get_name().clone()));
            attr.set("localName", Value::String(attribute.get_name().clone()));
            attr.set("value", Value::String(attribute.get_value().clone()));
            attr.set("namespaceURI", Value::Null);
            attr.set("prefix", Value::Null);
            attr.set("specified", Value::Boolean(true));
            attr.set("ownerElement", Value::Object(owner.clone()));
            attr
        })
        .collect();
    for (index, attr) in attrs.iter().enumerate() {
        map.set(index.to_string(), Value::Object(attr.clone()));
    }
    map.set("length", Value::Number(attrs.len() as f64));
    let item_attrs = attrs.clone();
    map.set_method(
        "item",
        Arc::new(move |arguments| {
            let index = match arguments.first() {
                Some(Value::Number(index)) if *index >= 0.0 => *index as usize,
                _ => return Ok(Value::Null),
            };
            Ok(item_attrs
                .get(index)
                .map_or(Value::Null, |attr| Value::Object(attr.clone())))
        }),
    );
    map.set_method(
        "getNamedItem",
        Arc::new(move |arguments| {
            let Some(name) = arguments.first() else {
                return Ok(Value::Null);
            };
            // HTML attribute names are stored lowercased, so lookups are
            // ASCII case-insensitive like getAttribute().
            let name = name.to_string().to_ascii_lowercase();
            Ok(attrs
                .iter()
                .find(|attr| matches!(attr.get("name"), Some(Value::String(n)) if n == name))
                .map_or(Value::Null, |attr| Value::Object(attr.clone())))
        }),
    );
    map
}

/// Returns the node ID and wrapper of a script `Node` argument.
pub(crate) fn node_argument(value: &Value) -> Option<(NodeID, Object)> {
    let (Value::Object(object) | Value::Array(object)) = value else {
        return None;
    };
    let Some(Value::Number(id)) = object.get("__nodeId") else {
        return None;
    };
    Some((NodeID::new(id as usize), object.clone()))
}

/// Converts `(Node or DOMString)...` arguments: element wrappers become node
/// inputs, and every other value is converted to a string for a new text node.
fn node_inputs(arguments: &[Value]) -> (Vec<NodeInput>, Vec<Object>) {
    let mut inputs = Vec::new();
    let mut objects = Vec::new();
    for argument in arguments {
        match node_argument(argument) {
            Some((id, object)) => {
                inputs.push(NodeInput::Node(id));
                objects.push(object);
            }
            None => inputs.push(NodeInput::Text(argument.to_string())),
        }
    }
    (inputs, objects)
}

/// Runs a tree mutation and then reports connection changes to custom
/// element callbacks, with the document unlocked while callbacks run.
///
/// Each `inserted` wrapper gets `disconnectedCallback` if it was connected
/// before the mutation and `connectedCallback` if it is connected after it,
/// so a node moved within the document sees both, as with `appendChild()`.
/// Each `removed` wrapper that is not also inserted gets
/// `disconnectedCallback` only when it left the document.
fn mutate_tree(
    document: &Arc<Mutex<Document>>,
    inserted: &[Object],
    removed: &[Object],
    mutation: impl FnOnce(&mut Document) -> Result<(), crate::DomException>,
) -> JsResult<()> {
    let mut tracked: Vec<(&Object, bool)> = Vec::new();
    for (object, is_inserted) in inserted
        .iter()
        .map(|object| (object, true))
        .chain(removed.iter().map(|object| (object, false)))
    {
        if !tracked.iter().any(|(seen, _)| *seen == object) {
            tracked.push((object, is_inserted));
        }
    }
    let node_of = |object: &Object| node_argument(&Value::Object(object.clone())).map(|(id, _)| id);
    let connections = {
        let mut document = document.lock().unwrap();
        let connected = |document: &Document, object: &Object| {
            node_of(object).is_some_and(|id| document.is_connected(id))
        };
        let before: Vec<bool> = tracked
            .iter()
            .map(|(object, _)| connected(&document, object))
            .collect();
        mutation(&mut document).map_err(|error| JsError::new(error.to_string()))?;
        tracked
            .iter()
            .zip(before)
            .map(|((object, _), was)| (was, connected(&document, object)))
            .collect::<Vec<_>>()
    };
    let evaluator = document.lock().unwrap().event_evaluator.clone();
    for ((object, is_inserted), (was_connected, is_connected)) in
        tracked.into_iter().zip(connections)
    {
        if is_inserted {
            if was_connected {
                invoke_lifecycle_callback(object, "disconnectedCallback", evaluator.as_ref());
            }
            if is_connected {
                invoke_lifecycle_callback(object, "connectedCallback", evaluator.as_ref());
            }
        } else if was_connected && !is_connected {
            invoke_lifecycle_callback(object, "disconnectedCallback", evaluator.as_ref());
        }
    }
    Ok(())
}

/// Returns the script wrapper for the element `node`, reusing the wrapper
/// already handed to scripts so identity comparisons hold.
pub(crate) fn wrap_element(document: &Arc<Mutex<Document>>, node: NodeID) -> JsResult<Value> {
    let (element, bindings) = {
        let document = document.lock().unwrap();
        (document.element_snapshot(node), document.script_bindings())
    };
    let Some(element) = element else {
        return Ok(Value::Null);
    };
    Ok(Value::Object(match bindings {
        Some(bindings) => script_element_object(document, &bindings.registry, element)?,
        None => element_object(element, document.clone()),
    }))
}

/// Builds a thrown exception object whose `name` and `message` a `catch`
/// clause can read.
pub(super) fn thrown_exception(name: &str, message: &str) -> JsError {
    let exception = Object::new();
    exception.set("name", Value::String(name.to_owned()));
    exception.set("message", Value::String(message.to_owned()));
    JsError::thrown(Value::Object(exception))
}

pub(crate) fn invoke_lifecycle_callback(
    element: &Object,
    name: &str,
    evaluator: Option<&ScriptEvaluator>,
) {
    if let Some(Value::Function(callback)) = element.get(name) {
        let _ = evaluator.map_or_else(
            || callback.call_with_this(Value::Object(element.clone()), &[]),
            |evaluator| {
                evaluator.call_with_this_reentrant(&callback, Value::Object(element.clone()), &[])
            },
        );
    }
}

pub(crate) fn invoke_attribute_changed_callback(
    element: &Object,
    name: &str,
    old_value: Option<String>,
    new_value: Option<String>,
    evaluator: Option<&ScriptEvaluator>,
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
    let arguments = [
        Value::String(name.to_owned()),
        old_value.map_or(Value::Null, Value::String),
        new_value.map_or(Value::Null, Value::String),
    ];
    let _ = evaluator.map_or_else(
        || callback.call_with_this(Value::Object(element.clone()), &arguments),
        |evaluator| {
            evaluator.call_with_this_reentrant(
                &callback,
                Value::Object(element.clone()),
                &arguments,
            )
        },
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
        log::debug!("rejected invalid attribute name {qualified_name:?}");
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
        log::debug!("Element.getAttribute() called with invalid arguments");
        return Err(JsError::new(
            "Element.getAttribute() expects one qualified name string",
        ));
    };

    log::trace!("Element.getAttribute({qualified_name:?})");
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
    fn element_query_selector_all_returns_shared_wrappers_for_descendants() {
        let mut realm = realm_with(
            "<div id='root' class='x'><p class='x' id='a'></p><span><b class='x' id='b'></b></span></div>\
             <p class='x' id='outside'></p>",
        );
        assert_eq!(
            eval(
                &mut realm,
                "var found = document.getElementById('root').querySelectorAll('.x, i'); \
                 found.length + ':' + found[0].getAttribute('id') + ':' + found[1].getAttribute('id')"
            ),
            string("2:a:b")
        );
        assert_eq!(
            eval(&mut realm, "found[0] === document.getElementById('a')"),
            Value::Boolean(true)
        );
        assert!(
            realm
                .evaluate_script("document.getElementById('root').querySelectorAll('div > p')")
                .unwrap_err()
                .to_string()
                .contains("SyntaxError")
        );
    }

    #[test]
    fn element_tag_name_uppercases_html_names() {
        let mut realm = realm_with("<main id='app'></main>");
        assert_eq!(
            eval(&mut realm, "document.getElementById('app').tagName"),
            string("MAIN")
        );
        assert_eq!(
            eval(&mut realm, "document.createElement('x-panel').tagName"),
            string("X-PANEL")
        );
    }

    #[test]
    fn element_attributes_lists_name_value_pairs_in_order() {
        let mut realm = realm_with("<a id='link' href='/x' data-kind='nav'></a>");
        assert_eq!(
            eval(
                &mut realm,
                "var attrs = document.getElementById('link').attributes; \
                 attrs.length + ':' + attrs[1].name + '=' + attrs[1].value + ':' + \
                 attrs.getNamedItem('DATA-KIND').value + ':' + attrs.item(0).name + ':' + \
                 (attrs.getNamedItem('missing') === null)"
            ),
            string("3:href=/x:nav:id:true")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var link = document.getElementById('link'); link.setAttribute('title', 't'); \
                 link.attributes.length + ':' + (link.attributes[0].ownerElement === link)"
            ),
            string("4:true")
        );
    }

    #[test]
    fn element_outer_html_serializes_the_current_subtree() {
        let mut realm = realm_with("<ul id='list'><li>a &amp; b</li></ul>");
        assert_eq!(
            eval(&mut realm, "document.getElementById('list').outerHTML"),
            string("<ul id=\"list\"><li>a &amp; b</li></ul>")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var list = document.getElementById('list'); \
                 list.appendChild(document.createElement('li')); list.outerHTML"
            ),
            string("<ul id=\"list\"><li>a &amp; b</li><li></li></ul>")
        );
        assert!(
            realm
                .evaluate_script("document.getElementById('list').outerHTML = '<p></p>'")
                .is_err()
        );
    }

    #[test]
    fn element_client_size_reports_px_padding_box_or_zero() {
        let mut realm = realm_with(
            "<div id='box' style='width: 100px; height: 40px; padding: 5px'></div><div id='auto'></div>",
        );
        assert_eq!(
            eval(
                &mut realm,
                "var box = document.getElementById('box'); box.clientWidth + 'x' + box.clientHeight"
            ),
            string("110x50")
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('auto').clientWidth"),
            Value::Number(0.0)
        );
        assert_eq!(
            eval(
                &mut realm,
                "var created = document.createElement('div'); \
                 created.setAttribute('style', 'width: 10px'); created.clientWidth"
            ),
            Value::Number(0.0)
        );
    }

    #[test]
    fn element_owner_document_is_the_document_object() {
        let mut realm = realm_with("<main id='app'></main>");
        assert_eq!(
            eval(
                &mut realm,
                "(document.getElementById('app').ownerDocument === document) + ':' + \
                 (document.createElement('p').ownerDocument === document)"
            ),
            string("true:true")
        );
    }

    #[test]
    fn element_remove_child_detaches_only_direct_children() {
        let mut realm = realm_with("<div id='p'><span id='c'><i id='g'></i></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); var child = document.getElementById('c'); \
                 (parent.removeChild(child) === child) + ':' + (document.getElementById('c') === null) + \
                 ':' + parent.outerHTML"
            ),
            string("true:true:<div id=\"p\"></div>")
        );
        assert!(
            realm
                .evaluate_script("parent.removeChild(child)")
                .unwrap_err()
                .to_string()
                .contains("NotFoundError")
        );
    }

    #[test]
    fn element_replace_child_swaps_nodes_and_returns_the_old_child() {
        let mut realm = realm_with("<div id='p'><a id='one'></a><a id='two'></a></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); var one = document.getElementById('one'); \
                 var fresh = document.createElement('b'); \
                 (parent.replaceChild(fresh, one) === one) + ':' + parent.outerHTML"
            ),
            string("true:<div id=\"p\"><b></b><a id=\"two\"></a></div>")
        );
        assert!(
            realm
                .evaluate_script("fresh.replaceChild(parent, fresh)")
                .unwrap_err()
                .to_string()
                .contains("HierarchyRequestError")
        );
    }

    #[test]
    fn element_replace_children_accepts_nodes_and_strings() {
        let mut realm = realm_with("<div id='p'><a id='one'></a><a id='two'></a></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); \
                 parent.replaceChildren('x<', document.getElementById('two'), 7); parent.outerHTML"
            ),
            string("<div id=\"p\">x&lt;<a id=\"two\"></a>7</div>")
        );
        assert_eq!(
            eval(&mut realm, "parent.replaceChildren(); parent.outerHTML"),
            string("<div id=\"p\"></div>")
        );
    }

    #[test]
    fn element_before_inserts_preceding_siblings() {
        let mut realm = realm_with("<div id='p'><a id='one'></a><a id='two'></a></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var two = document.getElementById('two'); \
                 two.before(document.createElement('hr'), 'text'); \
                 document.getElementById('p').outerHTML"
            ),
            string("<div id=\"p\"><a id=\"one\"></a><hr>text<a id=\"two\"></a></div>")
        );
        assert_eq!(
            eval(
                &mut realm,
                "document.createElement('p').before('ignored'); 'ok'"
            ),
            string("ok")
        );
    }

    #[test]
    fn tree_mutations_run_custom_element_connection_callbacks() {
        let mut realm = realm_with("<div id='p'><a id='one'></a></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var log = ''; var one = document.getElementById('one'); \
                 one.disconnectedCallback = () => { log = log + 'one-out;'; }; \
                 var fresh = document.createElement('b'); \
                 fresh.connectedCallback = () => { log = log + 'fresh-in,'; }; \
                 one.before(fresh); document.getElementById('p').removeChild(one); log"
            ),
            string("fresh-in,one-out;")
        );
    }

    #[test]
    fn created_element_get_attribute_returns_none_without_attributes() {
        let mut document = Document::new();
        let element = document.create_element("custom-widget").unwrap();

        assert_eq!(element.get_attribute("data-state"), None);
    }
}
