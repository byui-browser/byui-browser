//! Shared DOM Standard Node bindings over the existing HTML arena.
//!
//! Supported node kinds are Document, Element, Text, Comment, and DocumentType.
//! Node methods are inherited from one Node prototype per JavaScript realm;
//! getters are installed on wrappers because host getters lack a receiver.
//! Shadow trees, fragments, Attr/ProcessingInstruction/CDATA nodes, cross-document
//! adoption, mutation observers, and live Range adjustment are not yet supported.
use std::sync::{Arc, Mutex, Weak};

use html::{HTMLDocument, Namespace, NodeID, NodeKind};
use js::{Function, JsError, JsResult, Object, Realm, Value, WeakObject};

use crate::document::{Document, collect_elements};
use crate::element::{invoke_lifecycle_callback, wrap_element};
use crate::event::{add_event_listener_at, dispatch_event_at, remove_event_listener_at};

type SharedDocument = Arc<Mutex<Document>>;
type NodeEntry = (WeakObject, Weak<Mutex<Document>>, NodeID);
static NODES: Mutex<Vec<NodeEntry>> = Mutex::new(Vec::new());
type StandaloneTarget = (WeakObject, crate::event::EventTargetRef);
static EVENT_TARGETS: Mutex<Vec<StandaloneTarget>> = Mutex::new(Vec::new());

pub(crate) const TYPE_CONSTANTS: [(&str, u16); 12] = [
    ("ELEMENT_NODE", 1),
    ("ATTRIBUTE_NODE", 2),
    ("TEXT_NODE", 3),
    ("CDATA_SECTION_NODE", 4),
    ("ENTITY_REFERENCE_NODE", 5),
    ("ENTITY_NODE", 6),
    ("PROCESSING_INSTRUCTION_NODE", 7),
    ("COMMENT_NODE", 8),
    ("DOCUMENT_NODE", 9),
    ("DOCUMENT_TYPE_NODE", 10),
    ("DOCUMENT_FRAGMENT_NODE", 11),
    ("NOTATION_NODE", 12),
];
const POSITION_CONSTANTS: [(&str, u16); 6] = [
    ("DOCUMENT_POSITION_DISCONNECTED", 1),
    ("DOCUMENT_POSITION_PRECEDING", 2),
    ("DOCUMENT_POSITION_FOLLOWING", 4),
    ("DOCUMENT_POSITION_CONTAINS", 8),
    ("DOCUMENT_POSITION_CONTAINED_BY", 16),
    ("DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32),
];

fn exception(name: &str, message: &str) -> JsError {
    let object = Object::new();
    object.set("name", Value::String(name.into()));
    object.set("message", Value::String(message.into()));
    JsError::thrown(Value::Object(object))
}

#[derive(Clone)]
struct Handle {
    document: SharedDocument,
    id: NodeID,
    object: Object,
}

fn handle(value: &Value) -> JsResult<Handle> {
    let (Value::Object(object) | Value::Array(object)) = value else {
        return Err(exception("TypeError", "Expected a Node"));
    };
    {
        let entries = NODES.lock().unwrap();
        entries
            .iter()
            .find_map(|(weak, document, id)| {
                (weak.upgrade().as_ref() == Some(object))
                    .then(|| {
                        document.upgrade().map(|document| Handle {
                            document,
                            id: *id,
                            object: object.clone(),
                        })
                    })
                    .flatten()
            })
            .ok_or_else(|| exception("TypeError", "Expected a Node"))
    }
}

fn receiver() -> JsResult<Handle> {
    handle(&js::runtime::host_this())
}

fn nullable(value: Option<&Value>) -> JsResult<Option<Handle>> {
    match value {
        None | Some(Value::Null | Value::Undefined) => Ok(None),
        Some(value) => handle(value).map(Some),
    }
}

fn same_document(a: &Handle, b: &Handle) -> bool {
    Arc::ptr_eq(&a.document, &b.document)
}

fn children(document: &Document, id: NodeID) -> Vec<NodeID> {
    document
        .html_document()
        .and_then(|source| source.get_node(id))
        .map(|node| node.get_children().clone())
        .unwrap_or_default()
}

fn root(source: &HTMLDocument, mut node: NodeID) -> NodeID {
    while let Some(parent) = source.get_node(node).and_then(|node| node.get_parent()) {
        node = parent;
    }
    node
}

fn contains(source: &HTMLDocument, ancestor: NodeID, mut node: NodeID) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        match source.get_node(node).and_then(|node| node.get_parent()) {
            Some(parent) => node = parent,
            None => return false,
        }
    }
}

/// Registers or reuses the realm's Node interface and its wrapper prototypes.
/// Node is abstract: calling or constructing it throws TypeError.
pub(crate) fn register(realm: &mut Realm, document: &SharedDocument) -> JsResult<()> {
    let existing = ["Node", "NodeList", "Element", "Document"].map(|name| {
        match realm.registered_global_value(name) {
            Some(Value::Function(constructor)) => match constructor.get_own_property("prototype") {
                Some(Value::Object(prototype)) => Some(prototype),
                _ => None,
            },
            _ => None,
        }
    });
    if let [Some(node), Some(node_list), Some(element), Some(doc)] = existing {
        return finish_registration(document, node, node_list, element, doc);
    }
    let evaluator = realm.script_evaluator();
    let event_target = Object::new();
    for name in ["addEventListener", "removeEventListener", "dispatchEvent"] {
        event_target.set_method(
            name,
            Arc::new(move |arguments| {
                let this = js::runtime::host_this();
                if let Value::Object(object) = &this {
                    let target = EVENT_TARGETS
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(weak, _)| weak.upgrade().as_ref() == Some(object))
                        .map(|(_, target)| target.clone());
                    if let Some(target) = target {
                        return match name {
                            "addEventListener" => crate::event::add_event_listener_to_target(
                                &target,
                                arguments,
                                "EventTarget",
                            ),
                            "removeEventListener" => {
                                crate::event::remove_event_listener_from_target(&target, arguments)
                            }
                            _ => crate::event::dispatch_event_target(&target, object, arguments),
                        };
                    }
                }
                let node = handle(&this)?;
                if name == "dispatchEvent" {
                    let ancestors = {
                        let locked = node.document.lock().unwrap();
                        let source = locked.html_document().unwrap();
                        let mut ancestors = Vec::new();
                        let mut parent = source.get_nodes()[node.id.get()].get_parent();
                        while let Some(id) = parent {
                            ancestors.push(id);
                            parent = source.get_nodes()[id.get()].get_parent();
                        }
                        ancestors
                    };
                    for ancestor in ancestors {
                        wrap(&node.document, ancestor)?;
                    }
                }
                // Event dispatch executes callbacks; release the live document lock
                // before entering JavaScript. EventTarget state is shared by clones.
                let snapshot = node.document.lock().unwrap().clone();
                let target = snapshot
                    .html_document()
                    .and_then(|source| source.get_node(node.id))
                    .filter(|data| matches!(data.get_kind(), NodeKind::Document))
                    .map_or(Some(node.id), |_| None);
                match name {
                    "addEventListener" => add_event_listener_at(&snapshot, target, arguments),
                    "removeEventListener" => remove_event_listener_at(&snapshot, target, arguments),
                    _ => dispatch_event_at(&snapshot, target, &node.object, arguments),
                }
            }),
        );
    }
    let prototype = Object::with_prototype(Some(event_target.clone()));
    for (name, value) in TYPE_CONSTANTS.into_iter().chain(POSITION_CONSTANTS) {
        prototype.set_read_only(name, Value::Number(f64::from(value)));
    }
    for name in [
        "getRootNode",
        "hasChildNodes",
        "contains",
        "isSameNode",
        "isEqualNode",
        "compareDocumentPosition",
        "appendChild",
        "insertBefore",
        "removeChild",
        "replaceChild",
        "normalize",
        "cloneNode",
        "lookupPrefix",
        "lookupNamespaceURI",
        "isDefaultNamespace",
    ] {
        prototype.set_method(name, Arc::new(move |arguments| call(name, arguments)));
    }
    let element = Object::with_prototype(Some(prototype.clone()));
    let node_list = Object::new();
    let doc_prototype = Object::with_prototype(Some(prototype.clone()));
    for (name, object) in [
        ("EventTarget", event_target),
        ("Node", prototype.clone()),
        ("NodeList", node_list.clone()),
        ("Element", element.clone()),
        ("Document", doc_prototype.clone()),
    ] {
        let constructor_document = document.clone();
        let constructor_prototype = object.clone();
        let constructor_evaluator = evaluator.clone();
        let constructor = Function::from_host_constructor(
            name,
            Arc::new(|_| Err(exception("TypeError", "Illegal constructor"))),
            Arc::new(move |_| match name {
                "Document" => {
                    let bindings = constructor_document
                        .lock()
                        .unwrap()
                        .script_bindings()
                        .unwrap();
                    new_document(
                        &HTMLDocument::new(),
                        bindings,
                        Value::String("about:blank".into()),
                    )
                }
                "EventTarget" => {
                    let object = Object::with_prototype(Some(constructor_prototype.clone()));
                    let mut target = crate::event::EventTarget::default();
                    target.evaluator = Some(constructor_evaluator.clone());
                    let mut targets = EVENT_TARGETS.lock().unwrap();
                    targets.retain(|(weak, _)| weak.upgrade().is_some());
                    targets.push((object.downgrade(), Arc::new(Mutex::new(target))));
                    Ok(Value::Object(object))
                }
                _ => Err(exception("TypeError", "Illegal constructor")),
            }),
        );
        constructor.set_read_only_property("prototype", Value::Object(object.clone()));
        object.set_read_only("constructor", Value::Function(constructor.clone()));
        if name == "Node" {
            for (key, value) in TYPE_CONSTANTS.into_iter().chain(POSITION_CONSTANTS) {
                constructor.set_read_only_property(key, Value::Number(f64::from(value)));
            }
        }
        let value = Value::Function(constructor);
        realm.register_global_value(name, value.clone())?;
        if let Some(Value::Object(window)) = realm.registered_global_value("window") {
            window.set(name, value);
        }
    }
    finish_registration(document, prototype, node_list, element, doc_prototype)
}

fn finish_registration(
    document: &SharedDocument,
    prototype: Object,
    node_list: Object,
    element: Object,
    doc_prototype: Object,
) -> JsResult<()> {
    let root = *document.lock().unwrap().html_document().unwrap().get_root();
    let object = {
        let mut locked = document.lock().unwrap();
        let bindings = locked.script_bindings.as_mut().unwrap();
        bindings.node_prototype = prototype;
        bindings.node_list_prototype = node_list;
        bindings.element_prototype = element;
        bindings.document_prototype = doc_prototype;
        bindings.document.clone()
    };
    install(&object, root, document);
    // CharacterData creation is needed to exercise Node behavior on detached nodes.
    for (name, comment) in [("createTextNode", false), ("createComment", true)] {
        let document = document.clone();
        object.set_method(
            name,
            Arc::new(move |arguments| {
                let text = arguments
                    .first()
                    .ok_or_else(|| exception("TypeError", "Missing data"))?
                    .to_string();
                let id = {
                    let mut locked = document.lock().unwrap();
                    locked
                        .source_document
                        .as_mut()
                        .unwrap()
                        .push_node(if comment {
                            NodeKind::Comment(text)
                        } else {
                            NodeKind::Text(text)
                        })
                };
                wrap(&document, id)
            }),
        );
    }
    Ok(())
}

/// Installs the shared Node base on a wrapper without creating a second tree.
pub(crate) fn install(object: &Object, id: NodeID, document: &SharedDocument) {
    let (kind, bindings) = {
        let locked = document.lock().unwrap();
        (
            locked
                .html_document()
                .and_then(|source| source.get_node(id))
                .map(|node| node.get_kind().clone()),
            locked.script_bindings(),
        )
    };
    if let Some(bindings) = &bindings {
        let prototype = match kind {
            Some(NodeKind::Element(_)) => &bindings.element_prototype,
            Some(NodeKind::Document) => &bindings.document_prototype,
            _ => &bindings.node_prototype,
        };
        object.set_prototype(Some(prototype.clone()));
    }
    object.set_read_only("__nodeId", Value::Number(id.get() as f64));
    {
        let mut entries = NODES.lock().unwrap();
        entries.retain(|(object, _, _)| object.upgrade().is_some());
        if !entries
            .iter()
            .any(|(weak, _, _)| weak.upgrade().as_ref() == Some(object))
        {
            entries.push((object.downgrade(), Arc::downgrade(document), id));
        }
    }
    for name in [
        "nodeType",
        "nodeName",
        "baseURI",
        "isConnected",
        "ownerDocument",
        "parentNode",
        "parentElement",
        "firstChild",
        "lastChild",
        "previousSibling",
        "nextSibling",
        "nodeValue",
        "textContent",
    ] {
        let getter_document = document.clone();
        let setter_document = document.clone();
        let setter = if name == "nodeValue" || name == "textContent" {
            Some(
                Arc::new(move |value| set_text(&setter_document, id, name, value))
                    as js::HostSetter,
            )
        } else {
            None
        };
        object.define_accessor(
            name,
            Arc::new(move || property(&getter_document, id, name)),
            setter,
        );
    }
    let list = child_list(document, id);
    object.set_read_only("childNodes", Value::Object(list));
    if matches!(kind, Some(NodeKind::Text(_) | NodeKind::Comment(_))) {
        let get_document = document.clone();
        let set_document = document.clone();
        object.define_accessor(
            "data",
            Arc::new(move || property(&get_document, id, "nodeValue")),
            Some(Arc::new(move |value| {
                set_text(&set_document, id, "nodeValue", value)
            })),
        );
    }
}

pub(crate) fn wrap(document: &SharedDocument, id: NodeID) -> JsResult<Value> {
    let (kind, existing, bindings) = {
        let locked = document.lock().unwrap();
        (
            locked
                .html_document()
                .and_then(|source| source.get_node(id))
                .map(|node| node.get_kind().clone()),
            locked.cached_script_element(id),
            locked.script_bindings(),
        )
    };
    if let Some(existing) = existing {
        return Ok(Value::Object(existing));
    }
    if matches!(kind, Some(NodeKind::Document))
        && let Some(bindings) = &bindings
        && *document.lock().unwrap().html_document().unwrap().get_root() == id
    {
        return Ok(Value::Object(bindings.document.clone()));
    }
    if matches!(kind, Some(NodeKind::Element(_))) {
        return wrap_element(document, id);
    }
    if kind.is_none() || matches!(kind, Some(NodeKind::None)) {
        return Ok(Value::Null);
    }
    let object = Object::new();
    install(&object, id, document);
    Ok(Value::Object(
        document.lock().unwrap().remember_script_node(id, object),
    ))
}

fn property(document: &SharedDocument, id: NodeID, name: &str) -> JsResult<Value> {
    let (kind, bindings, relation, connected, text) = {
        let locked = document.lock().unwrap();
        let source = locked.html_document().unwrap();
        let node = source
            .get_node(id)
            .ok_or_else(|| exception("NotFoundError", "Unknown node"))?;
        let parent = node.get_parent();
        let siblings = parent.map(|parent| source.get_nodes()[parent.get()].get_children());
        let position = siblings.and_then(|siblings| siblings.iter().position(|child| *child == id));
        // `parentNode` is the DOM Standard's live parent relationship. It is
        // intentionally resolved from the arena on every read so that the
        // same wrapper observes moves and removals performed through any DOM
        // mutation method. Unlike `parentElement`, it also returns a
        // Document parent for the document element and applies to text,
        // comment, and doctype nodes.
        let relation = match name {
            "parentNode" => parent,
            "parentElement" => parent_element(source, parent),
            "firstChild" => node.get_children().first().copied(),
            "lastChild" => node.get_children().last().copied(),
            "previousSibling" => position
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| siblings?.get(i).copied()),
            "nextSibling" => position.and_then(|i| siblings?.get(i + 1).copied()),
            _ => None,
        };
        (
            node.get_kind().clone(),
            locked.script_bindings(),
            relation,
            matches!(
                source.get_nodes()[root(source, id).get()].get_kind(),
                NodeKind::Document
            ),
            if name == "textContent" {
                descendant_text(source, id)
            } else {
                String::new()
            },
        )
    };
    Ok(match name {
        "nodeType" => Value::Number(match kind {
            NodeKind::Element(_) => 1.,
            NodeKind::Text(_) => 3.,
            NodeKind::Comment(_) => 8.,
            NodeKind::Document => 9.,
            NodeKind::Doctype(_) => 10.,
            NodeKind::None => 0.,
        }),
        "nodeName" => Value::String(match kind {
            NodeKind::Element(element) => {
                if *element.get_namespace() == Namespace::Html {
                    element.get_name().to_ascii_uppercase()
                } else {
                    element.get_name().clone()
                }
            }
            NodeKind::Text(_) => "#text".into(),
            NodeKind::Comment(_) => "#comment".into(),
            NodeKind::Document => "#document".into(),
            NodeKind::Doctype(doctype) => doctype.get_name().clone().unwrap_or_default(),
            NodeKind::None => String::new(),
        }),
        "isConnected" => Value::Boolean(connected),
        "ownerDocument" => {
            if matches!(kind, NodeKind::Document) {
                Value::Null
            } else {
                bindings.map_or(Value::Null, |bindings| Value::Object(bindings.document))
            }
        }
        "baseURI" => Value::String(base_uri(document)),
        "nodeValue" => match kind {
            NodeKind::Text(data) | NodeKind::Comment(data) => Value::String(data),
            _ => Value::Null,
        },
        "textContent" => match kind {
            NodeKind::Text(data) | NodeKind::Comment(data) => Value::String(data),
            NodeKind::Element(_) => Value::String(text),
            _ => Value::Null,
        },
        _ => match relation {
            Some(id) => return wrap(document, id),
            None => Value::Null,
        },
    })
}

/// Returns the immediate parent when it is an Element.
///
/// This is the DOM `Node.parentElement` getter: it does not walk ancestors,
/// and a non-Element parent such as a Document produces `null`.
fn parent_element(source: &HTMLDocument, parent: Option<NodeID>) -> Option<NodeID> {
    parent.filter(|parent| {
        matches!(
            source.get_nodes()[parent.get()].get_kind(),
            NodeKind::Element(_)
        )
    })
}

fn descendant_text(source: &HTMLDocument, id: NodeID) -> String {
    let mut result = String::new();
    for child in source.get_nodes()[id.get()].get_children() {
        match source.get_nodes()[child.get()].get_kind() {
            NodeKind::Text(text) => result.push_str(text),
            NodeKind::Element(_) => result.push_str(&descendant_text(source, *child)),
            _ => {}
        }
    }
    result
}

fn base_uri(document: &SharedDocument) -> String {
    let (bindings, base) = {
        let locked = document.lock().unwrap();
        let base = locked.html_document().and_then(|source| {
            fn find(source: &HTMLDocument, id: NodeID) -> Option<String> {
                if let NodeKind::Element(element) = source.get_nodes()[id.get()].get_kind()
                    && *element.get_namespace() == Namespace::Html
                    && element.get_name() == "base"
                    && let Some(attr) = element
                        .get_attributes()
                        .iter()
                        .find(|a| a.get_name() == "href")
                {
                    return Some(attr.get_value().clone());
                }
                source.get_nodes()[id.get()]
                    .get_children()
                    .iter()
                    .find_map(|child| find(source, *child))
            }
            find(source, *source.get_root())
        });
        (locked.script_bindings(), base)
    };
    let href = bindings
        .and_then(|b| b.document.get("location"))
        .and_then(|location| match location {
            Value::Object(location) => location.get("href"),
            _ => None,
        })
        .map(|value| value.to_string())
        .unwrap_or_else(|| "about:blank".into());
    base.and_then(|base| net::Url::parse(&href).ok()?.join(&base).ok())
        .map_or(href, |url| url.to_string())
}

fn child_list(document: &SharedDocument, id: NodeID) -> Object {
    let list = Object::new();
    if let Some(bindings) = document.lock().unwrap().script_bindings() {
        list.set_prototype(Some(bindings.node_list_prototype));
    }
    let get_document = document.clone();
    list.set_property_hooks(
        Arc::new(move |key| {
            let ids = children(&get_document.lock().unwrap(), id);
            if key == "length" {
                return Ok(Some(Value::Number(ids.len() as f64)));
            }
            if let Ok(index) = key.parse::<usize>()
                && key == index.to_string()
            {
                return ids
                    .get(index)
                    .map(|id| wrap(&get_document, *id))
                    .transpose();
            }
            Ok(None)
        }),
        Arc::new(|property, _| {
            // NodeList's indexed properties are readonly. Other properties
            // are ordinary object properties and may be added by script.
            Ok(property == "length"
                || property
                    .parse::<usize>()
                    .is_ok_and(|index| index.to_string() == property))
        }),
        Arc::new({
            let document = document.clone();
            move || {
                Ok((0..children(&document.lock().unwrap(), id).len())
                    .map(|i| i.to_string())
                    .collect())
            }
        }),
    );
    let item_document = document.clone();
    list.set_method(
        "item",
        Arc::new(move |arguments| {
            if arguments.is_empty() {
                return Err(exception("TypeError", "NodeList.item requires an index"));
            }
            let number = match arguments.first() {
                Some(Value::Number(n)) => *n,
                Some(value) => value.to_string().parse().unwrap_or(f64::NAN),
                None => 0.,
            };
            let index = if number.is_finite() {
                number.trunc().rem_euclid(4294967296.) as usize
            } else {
                0
            };
            let ids = children(&item_document.lock().unwrap(), id);
            match ids.get(index) {
                Some(id) => wrap(&item_document, *id),
                None => Ok(Value::Null),
            }
        }),
    );
    let for_each_document = document.clone();
    let for_each_list = list.clone();
    list.set_method(
        "forEach",
        Arc::new(move |arguments| {
            let Some(Value::Function(callback)) = arguments.first() else {
                return Err(exception(
                    "TypeError",
                    "NodeList.forEach requires a callback",
                ));
            };
            // NodeList.forEach() is live, but the iteration list is fixed at
            // the start of the call, matching the DOM Standard's collection
            // iteration behavior while still resolving wrappers lazily.
            let ids = children(&for_each_document.lock().unwrap(), id);
            let evaluator = for_each_document.lock().unwrap().event_evaluator.clone();
            for (index, child) in ids.into_iter().enumerate() {
                let values = [
                    wrap(&for_each_document, child)?,
                    Value::Number(index as f64),
                    Value::Object(for_each_list.clone()),
                ];
                if let Some(evaluator) = &evaluator {
                    evaluator.call_with_this_reentrant(callback, Value::Undefined, &values)?;
                } else {
                    callback.call(&values)?;
                }
            }
            Ok(Value::Undefined)
        }),
    );
    for (name, kind) in [("keys", 0_u8), ("values", 1), ("entries", 2)] {
        let iterator_document = document.clone();
        list.set_method(
            name,
            Arc::new(move |_| {
                Ok(Value::Object(node_list_iterator(
                    &iterator_document,
                    id,
                    kind,
                )))
            }),
        );
    }
    let iterator_document = document.clone();
    list.set_method(
        "@@iterator",
        Arc::new(move |_| Ok(Value::Object(node_list_iterator(&iterator_document, id, 1)))),
    );
    list
}

fn node_list_iterator(document: &SharedDocument, id: NodeID, kind: u8) -> Object {
    let iterator = Object::new();
    let index = Arc::new(Mutex::new(0_usize));
    let next_document = document.clone();
    let next_index = index.clone();
    iterator.set_method(
        "next",
        Arc::new(move |_| {
            let mut index = next_index.lock().unwrap();
            let ids = children(&next_document.lock().unwrap(), id);
            let result = Object::new();
            if let Some(child) = ids.get(*index).copied() {
                let value = match kind {
                    0 => Value::Number(*index as f64),
                    1 => wrap(&next_document, child)?,
                    _ => {
                        let entry = Object::new();
                        entry.set("0", Value::Number(*index as f64));
                        entry.set("1", wrap(&next_document, child)?);
                        entry.set_read_only("length", Value::Number(2.0));
                        Value::Array(entry)
                    }
                };
                *index += 1;
                result.set("value", value);
                result.set("done", Value::Boolean(false));
            } else {
                result.set("value", Value::Undefined);
                result.set("done", Value::Boolean(true));
            }
            Ok(Value::Object(result))
        }),
    );
    iterator
}

fn set_text(document: &SharedDocument, id: NodeID, property: &str, value: Value) -> JsResult<()> {
    let value = if matches!(value, Value::Null | Value::Undefined) {
        String::new()
    } else {
        value.to_string()
    };
    let removed = {
        let mut locked = document.lock().unwrap();
        let kind = locked.source_document.as_ref().unwrap().get_nodes()[id.get()]
            .get_kind()
            .clone();
        match kind {
            NodeKind::Text(_) | NodeKind::Comment(_) => {
                locked
                    .source_document
                    .as_mut()
                    .unwrap()
                    .get_mut_node(id)
                    .unwrap()
                    .set_kind(if matches!(kind, NodeKind::Text(_)) {
                        NodeKind::Text(value)
                    } else {
                        NodeKind::Comment(value)
                    });
                Vec::new()
            }
            NodeKind::Element(_) if property == "textContent" => {
                let ids = children(&locked, id);
                let removed = connected_wrappers(&locked, &ids);
                let source = locked.source_document.as_mut().unwrap();
                for child in ids {
                    source.remove(child);
                }
                if !value.is_empty() {
                    let text = source.push_node(NodeKind::Text(value));
                    source.append_child(id, text);
                }
                locked.elements = collect_elements(source);
                removed
            }
            _ => Vec::new(),
        }
    };
    let evaluator = document.lock().unwrap().event_evaluator.clone();
    for object in removed {
        invoke_lifecycle_callback(&object, "disconnectedCallback", evaluator.as_ref());
    }
    Ok(())
}

fn connected_wrappers(document: &Document, ids: &[NodeID]) -> Vec<Object> {
    let mut objects = Vec::new();
    fn visit(document: &Document, id: NodeID, objects: &mut Vec<Object>) {
        if document.is_connected(id)
            && let Some(object) = document.cached_script_element(id)
        {
            objects.push(object);
        }
        for child in children(document, id) {
            visit(document, child, objects);
        }
    }
    for id in ids {
        visit(document, *id, &mut objects);
    }
    objects
}

fn call(name: &str, arguments: &[Value]) -> JsResult<Value> {
    let node = receiver()?;
    if arguments.is_empty()
        && matches!(
            name,
            "contains"
                | "isSameNode"
                | "isEqualNode"
                | "lookupPrefix"
                | "lookupNamespaceURI"
                | "isDefaultNamespace"
        )
    {
        return Err(exception("TypeError", "Missing required argument"));
    }
    match name {
        "hasChildNodes" => Ok(Value::Boolean(
            !children(&node.document.lock().unwrap(), node.id).is_empty(),
        )),
        "getRootNode" => {
            let id = {
                let locked = node.document.lock().unwrap();
                root(locked.html_document().unwrap(), node.id)
            };
            wrap(&node.document, id)
        }
        "contains" | "isSameNode" | "isEqualNode" => {
            let other = nullable(arguments.first())?;
            let result = if let Some(other) = other {
                match name {
                    "isSameNode" => node.object == other.object,
                    "isEqualNode" => equal(&node, &other),
                    _ if same_document(&node, &other) => contains(
                        node.document.lock().unwrap().html_document().unwrap(),
                        node.id,
                        other.id,
                    ),
                    _ => false,
                }
            } else {
                false
            };
            Ok(Value::Boolean(result))
        }
        "compareDocumentPosition" => {
            let other = handle(
                arguments
                    .first()
                    .ok_or_else(|| exception("TypeError", "Missing Node"))?,
            )?;
            let position = if !same_document(&node, &other) {
                1 | 32
                    | if Arc::as_ptr(&node.document) < Arc::as_ptr(&other.document) {
                        4
                    } else {
                        2
                    }
            } else {
                let locked = node.document.lock().unwrap();
                let source = locked.html_document().unwrap();
                if node.id == other.id {
                    0
                } else if root(source, node.id) != root(source, other.id) {
                    1 | 32 | if node.id.get() < other.id.get() { 4 } else { 2 }
                } else if contains(source, node.id, other.id) {
                    16 | 4
                } else if contains(source, other.id, node.id) {
                    8 | 2
                } else {
                    fn order(source: &HTMLDocument, id: NodeID, output: &mut Vec<NodeID>) {
                        output.push(id);
                        for child in source.get_nodes()[id.get()].get_children() {
                            order(source, *child, output);
                        }
                    }
                    let mut ids = Vec::new();
                    order(source, root(source, node.id), &mut ids);
                    if ids.iter().position(|id| *id == node.id)
                        < ids.iter().position(|id| *id == other.id)
                    {
                        4
                    } else {
                        2
                    }
                }
            };
            Ok(Value::Number(f64::from(position)))
        }
        "appendChild" | "insertBefore" | "removeChild" | "replaceChild" => {
            mutate(&node, name, arguments)
        }
        "normalize" => {
            normalize(
                node.document
                    .lock()
                    .unwrap()
                    .source_document
                    .as_mut()
                    .unwrap(),
                node.id,
            );
            Ok(Value::Undefined)
        }
        "cloneNode" => clone_node(&node, arguments.first().is_some_and(truthy)),
        "lookupPrefix" | "lookupNamespaceURI" | "isDefaultNamespace" => {
            let argument = arguments
                .first()
                .filter(|value| !matches!(value, Value::Null | Value::Undefined))
                .map(ToString::to_string)
                .filter(|text| !text.is_empty());
            let locked = node.document.lock().unwrap();
            let source = locked.html_document().unwrap();
            let result = if name == "lookupPrefix" {
                lookup_prefix(source, node.id, argument.as_deref())
            } else {
                lookup_namespace(
                    source,
                    node.id,
                    if name == "isDefaultNamespace" {
                        None
                    } else {
                        argument.as_deref()
                    },
                )
            };
            Ok(if name == "isDefaultNamespace" {
                Value::Boolean(result == argument)
            } else {
                result.map_or(Value::Null, Value::String)
            })
        }
        _ => unreachable!(),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Undefined => false,
        Value::Boolean(b) => *b,
        Value::Number(n) => *n != 0. && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn mutate(parent: &Handle, method: &str, arguments: &[Value]) -> JsResult<Value> {
    let inserted = handle(
        arguments
            .first()
            .ok_or_else(|| exception("TypeError", "Missing Node"))?,
    )?;
    if !same_document(parent, &inserted) {
        return Err(exception(
            "NotSupportedError",
            "Cross-document adoption is not implemented",
        ));
    }
    let reference = if method == "insertBefore" || method == "replaceChild" {
        if arguments.len() < 2 {
            return Err(exception("TypeError", "Missing child argument"));
        }
        let reference = nullable(arguments.get(1))?;
        if method == "replaceChild" && reference.is_none() {
            return Err(exception("TypeError", "Expected child Node"));
        }
        if let Some(reference) = &reference
            && !same_document(parent, reference)
        {
            return Err(exception(
                "NotFoundError",
                "Child belongs to a different document",
            ));
        }
        reference
    } else {
        None
    };
    let (removed, moved, changed, result) = {
        let mut locked = parent.document.lock().unwrap();
        let source = locked.source_document.as_mut().unwrap();
        let mut ids = source.get_nodes()[parent.id.get()].get_children().clone();
        let child = if method == "removeChild" {
            Some(inserted.id)
        } else {
            reference.as_ref().map(|h| h.id)
        };
        if method != "removeChild"
            && (!matches!(
                source.get_nodes()[parent.id.get()].get_kind(),
                NodeKind::Element(_) | NodeKind::Document
            ) || contains(source, inserted.id, parent.id))
        {
            return Err(exception("HierarchyRequestError", "Invalid node hierarchy"));
        }
        if let Some(child) = child
            && source.get_nodes()[child.get()].get_parent() != Some(parent.id)
        {
            return Err(exception("NotFoundError", "Not a child of this node"));
        }
        if method != "removeChild" {
            if matches!(
                source.get_nodes()[inserted.id.get()].get_kind(),
                NodeKind::Document | NodeKind::None
            ) {
                return Err(exception("HierarchyRequestError", "Invalid node hierarchy"));
            }
            if child == Some(inserted.id) {
                return Ok(Value::Object(inserted.object.clone()));
            }
            ids.retain(|id| *id != inserted.id);
            let position = child
                .and_then(|child| ids.iter().position(|id| *id == child))
                .unwrap_or(ids.len());
            if method == "replaceChild" {
                ids.remove(position);
            }
            ids.insert(position, inserted.id);
            if matches!(
                source.get_nodes()[parent.id.get()].get_kind(),
                NodeKind::Document
            ) {
                let mut element = None;
                let mut doctype = None;
                for (index, id) in ids.iter().enumerate() {
                    match source.get_nodes()[id.get()].get_kind() {
                        NodeKind::Element(_) if element.is_none() => element = Some(index),
                        NodeKind::Doctype(_) if doctype.is_none() => doctype = Some(index),
                        NodeKind::Comment(_) => {}
                        _ => {
                            return Err(exception(
                                "HierarchyRequestError",
                                "Invalid document children",
                            ));
                        }
                    }
                }
                if element
                    .zip(doctype)
                    .is_some_and(|(element, doctype)| doctype > element)
                {
                    return Err(exception(
                        "HierarchyRequestError",
                        "Doctype must precede document element",
                    ));
                }
            } else if matches!(
                source.get_nodes()[inserted.id.get()].get_kind(),
                NodeKind::Doctype(_)
            ) {
                return Err(exception(
                    "HierarchyRequestError",
                    "Doctype requires a document parent",
                ));
            }
        }
        let result = if method == "replaceChild" {
            reference.as_ref().unwrap().object.clone()
        } else {
            inserted.object.clone()
        };
        let unchanged = child == Some(inserted.id) && method != "removeChild";
        let removed_ids = if method == "removeChild" {
            vec![inserted.id]
        } else if method == "replaceChild" && !unchanged {
            vec![reference.as_ref().unwrap().id]
        } else {
            vec![]
        };
        let removed = connected_wrappers(&locked, &removed_ids);
        let moved = if method != "removeChild" && !unchanged {
            connected_wrappers(&locked, &[inserted.id])
        } else {
            vec![]
        };
        if !unchanged {
            let source = locked.source_document.as_mut().unwrap();
            source.remove(inserted.id);
            if method == "removeChild" {
                ids.retain(|id| *id != inserted.id);
            } else {
                if method == "replaceChild" {
                    source.remove(reference.as_ref().unwrap().id);
                }
                source
                    .get_mut_node(NodeID::new(inserted.id.get()))
                    .unwrap()
                    .set_parent(Some(parent.id));
            }
            source
                .get_mut_node(NodeID::new(parent.id.get()))
                .unwrap()
                .set_children(ids);
            locked.elements = collect_elements(source);
        }
        (removed, moved, !unchanged, result)
    };
    let evaluator = parent.document.lock().unwrap().event_evaluator.clone();
    for object in removed.into_iter().chain(moved) {
        invoke_lifecycle_callback(&object, "disconnectedCallback", evaluator.as_ref());
    }
    if changed && method != "removeChild" {
        let connected = connected_wrappers(&parent.document.lock().unwrap(), &[inserted.id]);
        for object in connected {
            invoke_lifecycle_callback(&object, "connectedCallback", evaluator.as_ref());
        }
    }
    Ok(Value::Object(result))
}

fn equal(a: &Handle, b: &Handle) -> bool {
    let a_source = a.document.lock().unwrap().html_document().unwrap().clone();
    let b_source = b.document.lock().unwrap().html_document().unwrap().clone();
    fn compare(a: &HTMLDocument, ai: NodeID, b: &HTMLDocument, bi: NodeID) -> bool {
        let an = &a.get_nodes()[ai.get()];
        let bn = &b.get_nodes()[bi.get()];
        let kinds_equal = match (an.get_kind(), bn.get_kind()) {
            (NodeKind::Element(a), NodeKind::Element(b)) => {
                a.get_name() == b.get_name()
                    && a.get_namespace() == b.get_namespace()
                    && a.get_attributes().len() == b.get_attributes().len()
                    && a.get_attributes()
                        .iter()
                        .all(|attr| b.get_attributes().contains(attr))
            }
            (a, b) => a == b,
        };
        kinds_equal
            && an.get_children().len() == bn.get_children().len()
            && an
                .get_children()
                .iter()
                .zip(bn.get_children())
                .all(|(ai, bi)| compare(a, *ai, b, *bi))
    }
    compare(&a_source, a.id, &b_source, b.id)
}

fn normalize(source: &mut HTMLDocument, id: NodeID) {
    let ids = source.get_nodes()[id.get()].get_children().clone();
    let mut previous: Option<NodeID> = None;
    for child in ids {
        if let NodeKind::Text(text) = source.get_nodes()[child.get()].get_kind().clone() {
            if text.is_empty() {
                source.remove(child);
            } else if let Some(previous) = previous {
                let NodeKind::Text(mut data) =
                    source.get_nodes()[previous.get()].get_kind().clone()
                else {
                    unreachable!()
                };
                data.push_str(&text);
                source
                    .get_mut_node(NodeID::new(previous.get()))
                    .unwrap()
                    .set_kind(NodeKind::Text(data));
                source.remove(child);
            } else {
                previous = Some(child);
            }
        } else {
            previous = None;
            normalize(source, child);
        }
    }
}

fn clone_node(node: &Handle, deep: bool) -> JsResult<Value> {
    fn copy(source: &mut HTMLDocument, id: NodeID, deep: bool) -> NodeID {
        let data = source.get_nodes()[id.get()].clone();
        let cloned = source.push_node(data.get_kind().clone());
        if deep {
            for child in data.get_children() {
                let child = copy(source, *child, true);
                source.append_child(cloned, child);
            }
        }
        cloned
    }
    let document_node = matches!(
        node.document
            .lock()
            .unwrap()
            .source_document
            .as_ref()
            .unwrap()
            .get_nodes()[node.id.get()]
        .get_kind(),
        NodeKind::Document
    );
    if document_node {
        let (source, original_bindings) = {
            let document = node.document.lock().unwrap();
            (
                document.html_document().unwrap().clone(),
                document.script_bindings().unwrap(),
            )
        };
        fn copy_into(old: &HTMLDocument, new: &mut HTMLDocument, id: NodeID, parent: NodeID) {
            let new_id = new.push_node(old.get_nodes()[id.get()].get_kind().clone());
            new.append_child(parent, new_id);
            for child in old.get_nodes()[id.get()].get_children() {
                copy_into(old, new, *child, new_id);
            }
        }
        let mut target = HTMLDocument::new();
        if deep {
            let root = *target.get_root();
            for child in source.get_nodes()[node.id.get()].get_children() {
                copy_into(&source, &mut target, *child, root);
            }
        }
        let href = original_bindings
            .document
            .get("location")
            .and_then(|location| match location {
                Value::Object(location) => location.get("href"),
                _ => None,
            })
            .unwrap_or(Value::String("about:blank".into()));
        return new_document(&target, original_bindings, href);
    }
    let id = {
        let mut document = node.document.lock().unwrap();
        let source = document.source_document.as_mut().unwrap();
        let id = copy(source, node.id, deep);
        document.elements = collect_elements(source);
        id
    };
    wrap(&node.document, id)
}

fn namespace_element(source: &HTMLDocument, id: NodeID) -> Option<NodeID> {
    match source.get_nodes()[id.get()].get_kind() {
        NodeKind::Element(_) => Some(id),
        NodeKind::Document => source.get_nodes()[id.get()]
            .get_children()
            .iter()
            .find(|child| {
                matches!(
                    source.get_nodes()[child.get()].get_kind(),
                    NodeKind::Element(_)
                )
            })
            .copied(),
        NodeKind::Doctype(_) => None,
        _ => source.get_nodes()[id.get()].get_parent().filter(|parent| {
            matches!(
                source.get_nodes()[parent.get()].get_kind(),
                NodeKind::Element(_)
            )
        }),
    }
}

fn namespace_uri(namespace: Namespace) -> &'static str {
    match namespace {
        Namespace::Html => "http://www.w3.org/1999/xhtml",
        Namespace::Svg => "http://www.w3.org/2000/svg",
        Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
    }
}

fn lookup_namespace(source: &HTMLDocument, id: NodeID, prefix: Option<&str>) -> Option<String> {
    let mut current = namespace_element(source, id);
    current?;
    if prefix == Some("xml") {
        return Some("http://www.w3.org/XML/1998/namespace".into());
    }
    if prefix == Some("xmlns") {
        return Some("http://www.w3.org/2000/xmlns/".into());
    }
    while let Some(id) = current {
        let NodeKind::Element(element) = source.get_nodes()[id.get()].get_kind() else {
            return None;
        };
        if prefix.is_none() {
            return Some(namespace_uri(*element.get_namespace()).into());
        }
        let attribute = format!("xmlns:{}", prefix.unwrap());
        if let Some(attr) = element
            .get_attributes()
            .iter()
            .find(|attr| attr.get_name() == &attribute)
        {
            return (!attr.get_value().is_empty()).then(|| attr.get_value().clone());
        }
        current = source.get_nodes()[id.get()].get_parent().filter(|parent| {
            matches!(
                source.get_nodes()[parent.get()].get_kind(),
                NodeKind::Element(_)
            )
        });
    }
    None
}

fn lookup_prefix(source: &HTMLDocument, id: NodeID, namespace: Option<&str>) -> Option<String> {
    let namespace = namespace?;
    let original = id;
    let mut current = namespace_element(source, id);
    while let Some(id) = current {
        let NodeKind::Element(element) = source.get_nodes()[id.get()].get_kind() else {
            return None;
        };
        for attr in element.get_attributes() {
            if attr.get_value() == namespace
                && let Some(prefix) = attr.get_name().strip_prefix("xmlns:")
                && lookup_namespace(source, original, Some(prefix)).as_deref() == Some(namespace)
            {
                return Some(prefix.into());
            }
        }
        current = source.get_nodes()[id.get()].get_parent().filter(|parent| {
            matches!(
                source.get_nodes()[parent.get()].get_kind(),
                NodeKind::Element(_)
            )
        });
    }
    None
}

fn new_document(
    source: &HTMLDocument,
    original: crate::document::ScriptBindings,
    href: Value,
) -> JsResult<Value> {
    let document = Arc::new(Mutex::new(Document::from_html_document(source)));
    let mut realm = Realm::new();
    let location = Object::new();
    location.set("href", href);
    crate::document::register_shared_document(&mut realm, document.clone(), Some(location))?;
    let mut locked = document.lock().unwrap();
    let bindings = locked.script_bindings.as_mut().unwrap();
    bindings.node_prototype = original.node_prototype;
    bindings.node_list_prototype = original.node_list_prototype;
    bindings.element_prototype = original.element_prototype;
    bindings.document_prototype = original.document_prototype;
    bindings
        .document
        .set_prototype(Some(bindings.document_prototype.clone()));
    Ok(Value::Object(bindings.document.clone()))
}
