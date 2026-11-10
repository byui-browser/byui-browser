//! The script-visible custom element registry.

use std::sync::{Arc, Mutex};

use js::{Function, JsError, Object, Value};

type PromiseResult = Arc<Mutex<Option<Value>>>;

struct PendingDefinition {
    name: String,
    promise: Object,
    result: PromiseResult,
    reactions: Vec<Function>,
}

/// Registry of custom element names and their JavaScript constructors.
///
/// This implements registry identity and the `define`, `get`, `getName`,
/// `whenDefined`, and subtree `upgrade` operations. The returned
/// `whenDefined` value is a thenable because the JavaScript runtime does not
/// yet provide native promises. Constructors that need realm globals cannot
/// run through the host construction path. Attribute changes made with
/// `setAttribute()` are reported for observed names, but parser-time
/// attributes and attribute removals are not yet reported. Customized
/// built-ins, scoped registries, and form-associated custom elements are not
/// supported.
#[derive(Clone)]
pub struct CustomElementRegistry {
    object: Object,
}

impl CustomElementRegistry {
    /// Creates an empty registry and its JavaScript object wrapper.
    pub fn new() -> Self {
        let object = Object::new();
        let definitions: Arc<Mutex<Vec<(String, Function)>>> = Arc::default();
        let waiting: Arc<Mutex<Vec<PendingDefinition>>> = Arc::default();

        let define_definitions = definitions.clone();
        let define_waiting = waiting.clone();
        object.set_method(
            "define",
            Arc::new(move |arguments| {
                let [Value::String(name), Value::Function(constructor), ..] = arguments else {
                    return Err(JsError::new(
                        "CustomElementRegistry.define() expects a name and constructor",
                    ));
                };
                if !is_valid_custom_element_name(name) {
                    return Err(JsError::new("SyntaxError: invalid custom element name"));
                }
                let mut definitions = define_definitions.lock().unwrap();
                if definitions.iter().any(|(defined, _)| defined == name) {
                    return Err(JsError::new("NotSupportedError: name is already defined"));
                }
                if definitions
                    .iter()
                    .any(|(_, defined)| defined == constructor)
                {
                    return Err(JsError::new(
                        "NotSupportedError: constructor is already registered",
                    ));
                }
                if matches!(
                    arguments.get(2),
                    Some(Value::Object(options) | Value::Array(options))
                        if options.get("extends").is_some_and(|value| !matches!(value, Value::Undefined))
                ) {
                    return Err(JsError::new(
                        "NotSupportedError: customized built-in elements are not supported",
                    ));
                }
                definitions.push((name.clone(), constructor.clone()));
                drop(definitions);
                let pending = {
                    let mut waiting = define_waiting.lock().unwrap();
                    waiting
                        .iter()
                        .position(|pending| pending.name == *name)
                        .map(|position| waiting.remove(position))
                };
                if let Some(pending) = pending {
                    *pending.result.lock().unwrap() = Some(Value::Function(constructor.clone()));
                    for reaction in pending.reactions {
                        let _ = reaction.call(&[Value::Function(constructor.clone())]);
                    }
                }
                Ok(Value::Undefined)
            }),
        );

        let get_definitions = definitions.clone();
        object.set_method(
            "get",
            Arc::new(move |arguments| {
                let [Value::String(name)] = arguments else {
                    return Err(JsError::new("CustomElementRegistry.get() expects a name"));
                };
                Ok(get_definitions
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(defined, _)| defined == name)
                    .map_or(Value::Undefined, |(_, constructor)| {
                        Value::Function(constructor.clone())
                    }))
            }),
        );

        let when_defined_definitions = definitions.clone();
        object.set_method(
            "whenDefined",
            Arc::new(move |arguments| {
                let [Value::String(name)] = arguments else {
                    return Err(JsError::new(
                        "CustomElementRegistry.whenDefined() expects a name",
                    ));
                };
                if !is_valid_custom_element_name(name) {
                    return Err(JsError::new("SyntaxError: invalid custom element name"));
                }
                let definition = when_defined_definitions
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(defined, _)| defined == name)
                    .map(|(_, constructor)| Value::Function(constructor.clone()));
                if let Some(pending) = waiting
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|pending| pending.name == *name)
                {
                    return Ok(Value::Object(pending.promise.clone()));
                }
                let settled = Arc::new(Mutex::new(definition));
                let promise_like = Object::new();
                let then_settled = settled.clone();
                let then_waiting = waiting.clone();
                let then_name = name.clone();
                if settled.lock().unwrap().is_none() {
                    waiting.lock().unwrap().push(PendingDefinition {
                        name: name.clone(),
                        promise: promise_like.clone(),
                        result: settled.clone(),
                        reactions: Vec::new(),
                    });
                }
                promise_like.set_method(
                    "then",
                    Arc::new(move |arguments| {
                        let Some(Value::Function(callback)) = arguments.first() else {
                            return Ok(Value::Undefined);
                        };
                        let callback = callback.clone();
                        let settled_value = then_settled.lock().unwrap().clone();
                        if let Some(value) = settled_value {
                            let _ = callback.call(&[value]);
                        } else {
                            let mut waiting = then_waiting.lock().unwrap();
                            if let Some(pending) =
                                waiting.iter_mut().find(|pending| pending.name == then_name)
                            {
                                let reaction_state = then_settled.clone();
                                pending.reactions.push(Function::from_host(
                                    "whenDefined reaction",
                                    Arc::new(move |values| {
                                        let Some(constructor) = values.first() else {
                                            return Ok(Value::Undefined);
                                        };
                                        *reaction_state.lock().unwrap() = Some(constructor.clone());
                                        let _ = callback.call(values);
                                        Ok(Value::Undefined)
                                    }),
                                ));
                            } else {
                                let reaction_state = then_settled.clone();
                                let reaction = Function::from_host(
                                    "whenDefined reaction",
                                    Arc::new(move |values| {
                                        let Some(constructor) = values.first() else {
                                            return Ok(Value::Undefined);
                                        };
                                        *reaction_state.lock().unwrap() = Some(constructor.clone());
                                        let _ = callback.call(values);
                                        Ok(Value::Undefined)
                                    }),
                                );
                                waiting.push(PendingDefinition {
                                    name: then_name.clone(),
                                    promise: Object::new(),
                                    result: then_settled.clone(),
                                    reactions: vec![reaction],
                                });
                            }
                        }
                        Ok(Value::Undefined)
                    }),
                );
                promise_like.set_method("catch", Arc::new(|_| Ok(Value::Undefined)));
                Ok(Value::Object(promise_like))
            }),
        );

        let upgrade_definitions = definitions.clone();
        object.set_method(
            "upgrade",
            Arc::new(move |arguments| {
                let [Value::Object(root)] = arguments else {
                    return Err(JsError::new(
                        "TypeError: CustomElementRegistry.upgrade() expects a Node",
                    ));
                };
                let Some(Value::String(local_name)) = root.get("localName") else {
                    return Err(JsError::new(
                        "TypeError: CustomElementRegistry.upgrade() expects a Node",
                    ));
                };
                if !matches!(root.get("__nodeId"), Some(Value::Number(_))) {
                    return Err(JsError::new(
                        "TypeError: CustomElementRegistry.upgrade() expects a Node",
                    ));
                }
                let constructor = upgrade_definitions
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(name, _)| name == &local_name)
                    .map(|(_, constructor)| constructor.clone());
                if let Some(constructor) = constructor {
                    if matches!(
                        root.get("__customElementUpgraded"),
                        Some(Value::Boolean(true))
                    ) {
                        return Ok(Value::Undefined);
                    }
                    root.set("__customElementUpgraded", Value::Boolean(true));
                    if let Some(Value::Object(prototype)) =
                        constructor.get_own_property("prototype")
                    {
                        root.set_prototype(Some(prototype));
                    }
                    if constructor.is_bytecode_function()
                        && let Value::Object(instance) | Value::Array(instance) =
                            constructor.construct(&[])?
                    {
                        for key in instance.keys() {
                            if let Some(value) = instance.get_own(&key) {
                                root.set(key, value);
                            }
                        }
                    }
                }
                Ok(Value::Undefined)
            }),
        );

        object.set_method(
            "initialize",
            Arc::new(|arguments| {
                if !matches!(arguments, [Value::Object(_) | Value::Array(_)]) {
                    return Err(JsError::new(
                        "CustomElementRegistry.initialize() expects a Node object",
                    ));
                }
                Err(JsError::new(
                    "NotSupportedError: scoped custom element registries are not supported",
                ))
            }),
        );

        object.set_method(
            "getName",
            Arc::new(move |arguments| {
                let [Value::Function(constructor)] = arguments else {
                    return Err(JsError::new(
                        "CustomElementRegistry.getName() expects a constructor",
                    ));
                };
                Ok(definitions
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(_, defined)| defined == constructor)
                    .map_or(Value::Null, |(name, _)| Value::String(name.clone())))
            }),
        );
        Self { object }
    }

    /// Returns the JavaScript object associated with this registry.
    pub fn object(&self) -> Object {
        self.object.clone()
    }
}

impl Default for CustomElementRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn is_valid_custom_element_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "annotation-xml",
        "color-profile",
        "font-face",
        "font-face-src",
        "font-face-uri",
        "font-face-format",
        "font-face-name",
        "missing-glyph",
    ];
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() || !name.contains('-') || RESERVED.contains(&name) {
        return false;
    }
    chars.all(|character| {
        if character.is_ascii() {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '.' | '_')
        } else {
            !character.is_control() && !character.is_whitespace()
        }
    })
}

#[cfg(test)]
mod tests {
    use js::{Object, Realm, Value, Vm, bytecode};

    use super::CustomElementRegistry;
    use crate::register_window;

    #[test]
    fn window_custom_elements_defines_and_looks_up_a_constructor() {
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "function XPanel() {} window.customElements.define('x-panel', XPanel); window.customElements.get('x-panel') === XPanel && window.customElements.getName(XPanel) === 'x-panel'"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn window_and_global_custom_elements_share_the_same_registry() {
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();

        assert_eq!(
            realm.evaluate_script("window.customElements === customElements"),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn when_defined_notifies_a_waiter_after_registration() {
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "var resolved = false; customElements.whenDefined('x-panel').then(constructor => { resolved = constructor === XPanel; }); function XPanel() {} customElements.define('x-panel', XPanel); resolved"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn when_defined_returns_the_registered_constructor_for_defined_names() {
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "function XPanel() {} customElements.define('x-panel', XPanel); var resolved; customElements.whenDefined('x-panel').then(constructor => { resolved = constructor === XPanel; }); resolved"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn repeated_when_defined_calls_share_the_pending_promise() {
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "var first = customElements.whenDefined('x-panel'); first === customElements.whenDefined('x-panel')"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn registering_window_after_document_preserves_the_document_registry() {
        let mut realm = Realm::new();
        crate::register_document(&mut realm, crate::Document::new()).unwrap();
        register_window(&mut realm).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "window.customElements === customElements && window.document === document"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn upgrade_accepts_a_dom_element_wrapper() {
        let mut realm = Realm::new();
        crate::register_document(&mut realm, crate::Document::new()).unwrap();
        register_window(&mut realm).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "var element = document.createElement('x-panel'); customElements.upgrade(element); true"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn upgrade_sets_the_registered_constructor_prototype_on_the_element() {
        let program =
            js::parse("class XPanel { constructor() { this.ready = true; } } XPanel").unwrap();
        let chunk = bytecode::compile(&program).unwrap();
        let constructor = match Vm::new().execute(&chunk).unwrap() {
            Value::Function(constructor) => constructor,
            value => panic!("expected class constructor, got {value}"),
        };
        let registry = CustomElementRegistry::new().object();
        let Some(Value::Function(define)) = registry.get("define") else {
            panic!("registry define method is missing");
        };
        define
            .call(&[
                Value::String("x-panel".into()),
                Value::Function(constructor.clone()),
            ])
            .unwrap();

        let element = Object::new();
        element.set("__nodeId", Value::Number(1.0));
        element.set("localName", Value::String("x-panel".into()));
        let Some(Value::Function(upgrade)) = registry.get("upgrade") else {
            panic!("registry upgrade method is missing");
        };
        upgrade.call(&[Value::Object(element.clone())]).unwrap();

        assert_eq!(
            element.prototype(),
            match constructor.get_own_property("prototype") {
                Some(Value::Object(prototype)) => Some(prototype),
                _ => None,
            }
        );
        assert_eq!(element.get("ready"), Some(Value::Boolean(true)));
    }

    #[test]
    fn create_element_runs_a_registered_bytecode_constructor_once() {
        let program = js::parse(
            "class XPanel { constructor() { this.ready = true; } connectedCallback() { this.connected = true; } disconnectedCallback() { this.connected = false; } } XPanel",
        )
        .unwrap();
        let chunk = bytecode::compile(&program).unwrap();
        let constructor = Vm::new().execute(&chunk).unwrap();
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();
        let html_document = html::parse_raw_html("<main id='root'></main>".into());
        crate::register_document(
            &mut realm,
            crate::Document::from_html_document(&html_document),
        )
        .unwrap();
        realm.register_global_value("XPanel", constructor).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "customElements.define('x-panel', XPanel); var element = document.createElement('x-panel'); element.setAttribute('id', 'created'); document.getElementById('root').appendChild(element); var sameElement = document.getElementById('created'); var connected = element === sameElement && sameElement.ready === true && sameElement.connected === true; sameElement.remove(); connected && sameElement.connected === false"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn define_upgrades_matching_elements_already_in_the_document() {
        let program =
            js::parse("class XPanel { constructor() { this.ready = true; } } XPanel").unwrap();
        let constructor = Vm::new()
            .execute(&bytecode::compile(&program).unwrap())
            .unwrap();
        let html_document = html::parse_raw_html("<x-panel id='existing'></x-panel>".into());
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();
        crate::register_document(
            &mut realm,
            crate::Document::from_html_document(&html_document),
        )
        .unwrap();
        realm.register_global_value("XPanel", constructor).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "customElements.define('x-panel', XPanel); document.getElementById('existing').ready"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn attribute_changed_callback_runs_only_for_observed_attributes() {
        let program = js::parse(
            "class XPanel { attributeChangedCallback(name, oldValue, newValue) { this.changed = newValue; } } XPanel",
        )
        .unwrap();
        let constructor = Vm::new()
            .execute(&bytecode::compile(&program).unwrap())
            .unwrap();
        let Value::Function(constructor_function) = &constructor else {
            panic!("expected class constructor");
        };
        let observed = Object::new();
        observed.set("0", Value::String("data-state".into()));
        observed.set("length", Value::Number(1.0));
        assert!(
            constructor_function.set_own_property("observedAttributes", Value::Array(observed))
        );

        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();
        crate::register_document(&mut realm, crate::Document::new()).unwrap();
        realm.register_global_value("XPanel", constructor).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "customElements.define('x-panel', XPanel); var element = document.createElement('x-panel'); element.setAttribute('title', 'ignored'); var before = element.changed; element.setAttribute('data-state', 'ready'); before === undefined && element.changed === 'ready'"
            ),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn upgrade_reports_existing_observed_attributes() {
        let program = js::parse(
            "class XPanel { attributeChangedCallback(name, oldValue, newValue) { this.changed = newValue; } } XPanel",
        )
        .unwrap();
        let constructor = Vm::new()
            .execute(&bytecode::compile(&program).unwrap())
            .unwrap();
        let Value::Function(constructor_function) = &constructor else {
            panic!("expected class constructor");
        };
        let observed = Object::new();
        observed.set("0", Value::String("data-state".into()));
        observed.set("length", Value::Number(1.0));
        assert!(
            constructor_function.set_own_property("observedAttributes", Value::Array(observed))
        );

        let html_document =
            html::parse_raw_html("<x-panel id='panel' data-state='from-parser'></x-panel>".into());
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();
        crate::register_document(
            &mut realm,
            crate::Document::from_html_document(&html_document),
        )
        .unwrap();
        realm.register_global_value("XPanel", constructor).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "customElements.define('x-panel', XPanel); document.getElementById('panel').changed === 'from-parser'"
            ),
            Ok(Value::Boolean(true))
        );
    }
}
