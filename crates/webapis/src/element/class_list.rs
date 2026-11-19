//! `Element.classList` on script element wrappers.
//!
//! The getter returns a `DOMTokenList`-like host object whose state lives in
//! the element's `class` attribute: every read (`length`, `value`, indexed
//! tokens, `item`, `contains`) consults the document, and every mutation
//! (`add`, `remove`, `toggle`, `replace`, the `value` setter) writes through
//! [`Document::class_list`], so `getAttribute('class')` reflects it.
//!
//! Limitations:
//! - `attributeChangedCallback` runs only when a mutation changes the
//!   `class` attribute's value (or creates it), whereas the DOM Standard also
//!   fires it when an update rewrites the attribute with an equal value.
//! - The list has no iterator protocol (`for...of`, spread) and no
//!   `forEach`/`keys`/`values`/`entries`; scripts index it with `length`.

use std::sync::{Arc, Mutex};

use html::NodeID;
use js::{JsError, JsResult, Object, Value, WeakObject};

use crate::document::Document;
use crate::element::{Element, invoke_attribute_changed_callback};

/// Installs `Element.classList` on the wrapper `object` for `element`.
///
/// The getter always returns the same token-list object for one wrapper,
/// matching the `[SameObject]` annotation. Assigning to `classList` forwards
/// to the list's `value` setter (`[PutForwards=value]`).
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let list = ClassList {
        node: element.node,
        document: document.clone(),
        owner: object.downgrade(),
    };
    let token_list = token_list_object(&list);
    let getter_list = token_list.clone();
    object.define_accessor(
        "classList",
        Arc::new(move || Ok(Value::Object(getter_list.clone()))),
        Some(Arc::new(move |value: Value| {
            list.set_value(&value.to_string())
        })),
    );
}

/// The shared state behind one element's token-list object.
#[derive(Clone)]
struct ClassList {
    /// The element whose `class` attribute backs the list.
    node: NodeID,
    /// The document holding the element.
    document: Arc<Mutex<Document>>,
    /// The element's wrapper, held weakly so the list does not keep it alive,
    /// used to run `attributeChangedCallback`.
    owner: WeakObject,
}

impl ClassList {
    /// Returns the current tokens, in order, without duplicates.
    fn tokens(&self) -> JsResult<Vec<String>> {
        Ok(self
            .document
            .lock()
            .unwrap()
            .class_list(self.node)?
            .tokens())
    }

    /// Returns the raw `class` attribute value, or `""` when absent.
    fn value(&self) -> JsResult<String> {
        Ok(self.document.lock().unwrap().class_list(self.node)?.value())
    }

    /// Returns the raw `class` attribute value, or `None` when absent.
    fn attribute(&self) -> Option<String> {
        self.document
            .lock()
            .unwrap()
            .element_snapshot(self.node)
            .and_then(|element| element.get_attribute("class").map(str::to_owned))
    }

    /// Runs `operation` on the document's token list, then fires
    /// `attributeChangedCallback` (without the document lock held) when the
    /// `class` attribute changed.
    fn mutate<T>(
        &self,
        operation: impl FnOnce(&mut html::DOMTokenList<'_>) -> Result<T, html::DomError>,
    ) -> JsResult<T> {
        let old_value = self.attribute();
        let result = {
            let mut document = self.document.lock().unwrap();
            let mut list = document.class_list(self.node)?;
            operation(&mut list).map_err(dom_exception)?
        };
        let new_value = self.attribute();
        if old_value != new_value
            && let Some(owner) = self.owner.upgrade()
        {
            invoke_attribute_changed_callback(&owner, "class", old_value, new_value);
        }
        Ok(result)
    }

    /// Implements the `value` setter: replaces the raw attribute value.
    fn set_value(&self, value: &str) -> JsResult<()> {
        self.mutate(|list| list.set_value(value))
    }
}

/// Builds the script-visible token-list object backed by `list`.
fn token_list_object(list: &ClassList) -> Object {
    let object = Object::new();

    let length_list = list.clone();
    object.define_accessor(
        "length",
        Arc::new(move || Ok(Value::Number(length_list.tokens()?.len() as f64))),
        None,
    );

    let value_list = list.clone();
    let value_setter_list = list.clone();
    object.define_accessor(
        "value",
        Arc::new(move || Ok(Value::String(value_list.value()?))),
        Some(Arc::new(move |value: Value| {
            value_setter_list.set_value(&value.to_string())
        })),
    );

    // Indexed tokens are resolved on every read through the host property
    // hooks, so `list[0]` always reflects the current attribute value.
    let index_list = list.clone();
    let keys_list = list.clone();
    object.set_property_hooks(
        Arc::new(move |key| {
            let Some(index) = array_index(key) else {
                return Ok(None);
            };
            Ok(index_list.tokens()?.get(index).cloned().map(Value::String))
        }),
        Arc::new(|key, _| {
            // Indexed properties are read-only; assignments to them are
            // silently ignored, as in non-strict code.
            Ok(array_index(key).is_some())
        }),
        Arc::new(move || {
            Ok((0..keys_list.tokens()?.len())
                .map(|index| index.to_string())
                .collect())
        }),
    );

    let item_list = list.clone();
    object.set_method(
        "item",
        Arc::new(move |arguments| {
            let index = to_index(arguments.first().unwrap_or(&Value::Undefined));
            let tokens = item_list.tokens()?;
            Ok(index
                .and_then(|index| tokens.get(index).cloned())
                .map_or(Value::Null, Value::String))
        }),
    );

    let contains_list = list.clone();
    object.set_method(
        "contains",
        Arc::new(move |arguments| {
            let token = string_argument(arguments, 0);
            Ok(Value::Boolean(contains_list.tokens()?.contains(&token)))
        }),
    );

    let add_list = list.clone();
    object.set_method(
        "add",
        Arc::new(move |arguments| {
            let tokens = string_arguments(arguments);
            let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();
            add_list.mutate(|list| list.add(&tokens))?;
            Ok(Value::Undefined)
        }),
    );

    let remove_list = list.clone();
    object.set_method(
        "remove",
        Arc::new(move |arguments| {
            let tokens = string_arguments(arguments);
            let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();
            remove_list.mutate(|list| list.remove(&tokens))?;
            Ok(Value::Undefined)
        }),
    );

    let toggle_list = list.clone();
    object.set_method(
        "toggle",
        Arc::new(move |arguments| {
            let token = string_argument(arguments, 0);
            let force = match arguments.get(1) {
                None | Some(Value::Undefined) => None,
                Some(value) => Some(truthy(value)),
            };
            let present = toggle_list.mutate(|list| list.toggle(&token, force))?;
            Ok(Value::Boolean(present))
        }),
    );

    let replace_list = list.clone();
    object.set_method(
        "replace",
        Arc::new(move |arguments| {
            if arguments.len() < 2 {
                return Err(thrown_error(
                    "TypeError",
                    "DOMTokenList.replace() requires two arguments",
                    None,
                ));
            }
            let token = string_argument(arguments, 0);
            let new_token = string_argument(arguments, 1);
            let replaced = replace_list.mutate(|list| list.replace(&token, &new_token))?;
            Ok(Value::Boolean(replaced))
        }),
    );

    object.set_method(
        "supports",
        Arc::new(|_| {
            Err(thrown_error(
                "TypeError",
                "DOMTokenList.supports() is not available for classList, \
                 because the class attribute defines no supported tokens",
                None,
            ))
        }),
    );

    let to_string_list = list.clone();
    object.set_method(
        "toString",
        Arc::new(move |_| Ok(Value::String(to_string_list.value()?))),
    );

    object
}

/// Converts an HTML token-list error into a thrown `DOMException`-like
/// object whose `name` (`SyntaxError`, `InvalidCharacterError`, ...) and
/// legacy `code` scripts can inspect in a `catch` clause.
fn dom_exception(error: html::DomError) -> JsError {
    let (detail, code) = match error {
        html::DomError::Syntax => ("the token must not be empty", Some(12)),
        html::DomError::InvalidToken | html::DomError::InvalidCharacter => {
            ("the token must not contain ASCII whitespace", Some(5))
        }
        html::DomError::Type => ("the operation is not supported", None),
        html::DomError::NotFound => ("the element is no longer available", Some(8)),
    };
    thrown_error(&error.to_string(), detail, code)
}

/// Builds a thrown error object named `name` carrying `message`, with a
/// legacy `DOMException` `code` when one applies.
fn thrown_error(name: &str, message: &str, code: Option<u16>) -> JsError {
    let exception = Object::new();
    exception.set("name", Value::String(name.to_owned()));
    exception.set("message", Value::String(message.to_owned()));
    if let Some(code) = code {
        exception.set("code", Value::Number(f64::from(code)));
    }
    JsError::thrown(Value::Object(exception))
}

/// Returns the `index`th argument converted with `ToString`, treating a
/// missing argument as `undefined`.
fn string_argument(arguments: &[Value], index: usize) -> String {
    arguments
        .get(index)
        .unwrap_or(&Value::Undefined)
        .to_string()
}

/// Converts every argument with `ToString`.
fn string_arguments(arguments: &[Value]) -> Vec<String> {
    arguments.iter().map(Value::to_string).collect()
}

/// Parses a canonical array-index property key such as `"0"` or `"12"`.
fn array_index(key: &str) -> Option<usize> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    if !key.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    key.parse().ok()
}

/// Converts an `item()` argument the way WebIDL `unsigned long` does for
/// in-range values: non-numbers are coerced, fractions truncate, and
/// negative or non-finite values select no token.
fn to_index(value: &Value) -> Option<usize> {
    let number = match value {
        Value::Number(number) => *number,
        Value::Boolean(flag) => f64::from(u8::from(*flag)),
        Value::String(text) => text.trim().parse::<f64>().unwrap_or(f64::NAN),
        Value::Null => 0.0,
        _ => f64::NAN,
    };
    if number.is_nan() {
        return Some(0);
    }
    let number = number.trunc();
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    Some(number as usize)
}

/// JavaScript `ToBoolean`.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(flag) => *flag,
        Value::Number(number) => *number != 0.0 && !number.is_nan(),
        Value::String(text) => !text.is_empty(),
        Value::Function(_) | Value::Object(_) | Value::Array(_) => true,
    }
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
    fn class_list_reads_tokens_length_and_value() {
        let mut realm = realm_with("<div id='a' class='  one two\tone three '></div>");
        eval(
            &mut realm,
            "var list = document.getElementById('a').classList;",
        );
        assert_eq!(eval(&mut realm, "list.length"), Value::Number(3.0));
        assert_eq!(
            eval(&mut realm, "list[0] + ',' + list[1] + ',' + list[2]"),
            string("one,two,three")
        );
        assert_eq!(eval(&mut realm, "list[3]"), Value::Undefined);
        assert_eq!(eval(&mut realm, "list.item(1)"), string("two"));
        assert_eq!(eval(&mut realm, "list.item(3)"), Value::Null);
        assert_eq!(eval(&mut realm, "list.item(-1)"), Value::Null);
        assert_eq!(
            eval(&mut realm, "list.value"),
            string("  one two\tone three ")
        );
        assert_eq!(
            eval(&mut realm, "list.toString()"),
            string("  one two\tone three ")
        );
        assert_eq!(
            eval(&mut realm, "list.contains('two')"),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(&mut realm, "list.contains('tw')"),
            Value::Boolean(false)
        );
    }

    #[test]
    fn class_list_is_the_same_object_and_empty_without_a_class_attribute() {
        let mut realm = realm_with("<p id='p'></p>");
        assert_eq!(
            eval(
                &mut realm,
                "var p = document.getElementById('p'); p.classList === p.classList"
            ),
            Value::Boolean(true)
        );
        assert_eq!(eval(&mut realm, "p.classList.length"), Value::Number(0.0));
        assert_eq!(eval(&mut realm, "p.classList.value"), string(""));
        // Removing from an absent attribute must not create one.
        eval(&mut realm, "p.classList.remove('x');");
        assert_eq!(eval(&mut realm, "p.getAttribute('class')"), Value::Null);
    }

    #[test]
    fn class_list_add_and_remove_write_through_to_the_attribute() {
        let mut realm = realm_with("<div id='a' class='one'></div>");
        eval(
            &mut realm,
            "var el = document.getElementById('a'); var list = el.classList;\
             list.add('two', 'three', 'one');",
        );
        assert_eq!(
            eval(&mut realm, "el.getAttribute('class')"),
            string("one two three")
        );
        assert_eq!(eval(&mut realm, "list.length"), Value::Number(3.0));
        assert_eq!(eval(&mut realm, "list[2]"), string("three"));
        eval(&mut realm, "list.remove('one', 'missing', 'three');");
        assert_eq!(eval(&mut realm, "el.getAttribute('class')"), string("two"));
        assert_eq!(eval(&mut realm, "list.length"), Value::Number(1.0));
        assert_eq!(eval(&mut realm, "list[1]"), Value::Undefined);
    }

    #[test]
    fn class_list_reflects_later_set_attribute_calls() {
        let mut realm = realm_with("<div id='a' class='one'></div>");
        eval(
            &mut realm,
            "var el = document.getElementById('a'); var list = el.classList;\
             el.setAttribute('class', 'x y');",
        );
        assert_eq!(eval(&mut realm, "list.length"), Value::Number(2.0));
        assert_eq!(eval(&mut realm, "list[1]"), string("y"));
        assert_eq!(
            eval(&mut realm, "list.contains('one')"),
            Value::Boolean(false)
        );
    }

    #[test]
    fn class_list_toggle_honors_force() {
        let mut realm = realm_with("<div id='a' class='one'></div>");
        eval(
            &mut realm,
            "var el = document.getElementById('a'); var list = el.classList;",
        );
        assert_eq!(eval(&mut realm, "list.toggle('two')"), Value::Boolean(true));
        assert_eq!(
            eval(&mut realm, "el.getAttribute('class')"),
            string("one two")
        );
        assert_eq!(
            eval(&mut realm, "list.toggle('one')"),
            Value::Boolean(false)
        );
        assert_eq!(eval(&mut realm, "el.getAttribute('class')"), string("two"));
        assert_eq!(
            eval(&mut realm, "list.toggle('two', true)"),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(&mut realm, "list.toggle('three', false)"),
            Value::Boolean(false)
        );
        assert_eq!(eval(&mut realm, "el.getAttribute('class')"), string("two"));
        assert_eq!(
            eval(&mut realm, "list.toggle('two', 0)"),
            Value::Boolean(false)
        );
        assert_eq!(eval(&mut realm, "el.getAttribute('class')"), string(""));
    }

    #[test]
    fn class_list_replace_swaps_tokens_in_place() {
        let mut realm = realm_with("<div id='a' class='one two three'></div>");
        eval(
            &mut realm,
            "var el = document.getElementById('a'); var list = el.classList;",
        );
        assert_eq!(
            eval(&mut realm, "list.replace('two', 'zwei')"),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(&mut realm, "el.getAttribute('class')"),
            string("one zwei three")
        );
        assert_eq!(
            eval(&mut realm, "list.replace('missing', 'x')"),
            Value::Boolean(false)
        );
        assert_eq!(
            eval(&mut realm, "list.replace('one', 'three')"),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(&mut realm, "el.getAttribute('class')"),
            string("zwei three")
        );
    }

    #[test]
    fn class_list_value_setter_and_class_list_assignment_replace_the_attribute() {
        let mut realm = realm_with("<div id='a' class='one'></div>");
        eval(
            &mut realm,
            "var el = document.getElementById('a'); var list = el.classList;\
             list.value = ' a  b ';",
        );
        assert_eq!(
            eval(&mut realm, "el.getAttribute('class')"),
            string(" a  b ")
        );
        assert_eq!(eval(&mut realm, "list.length"), Value::Number(2.0));
        eval(&mut realm, "el.classList = 'c';");
        assert_eq!(eval(&mut realm, "el.getAttribute('class')"), string("c"));
        assert_eq!(
            eval(&mut realm, "el.classList === list"),
            Value::Boolean(true)
        );
    }

    #[test]
    fn class_list_rejects_invalid_tokens_with_dom_exceptions() {
        let mut realm = realm_with("<div id='a' class='one'></div>");
        eval(
            &mut realm,
            "var el = document.getElementById('a'); var list = el.classList;",
        );
        for (script, name) in [
            ("list.add('')", "SyntaxError"),
            ("list.add('ok', 'a b')", "InvalidCharacterError"),
            ("list.remove('')", "SyntaxError"),
            ("list.toggle('a\\tb')", "InvalidCharacterError"),
            ("list.replace('one', '')", "SyntaxError"),
            ("list.replace('one', 'x y')", "InvalidCharacterError"),
            ("list.supports('x')", "TypeError"),
        ] {
            assert_eq!(
                eval(
                    &mut realm,
                    &format!("try {{ {script}; 'none' }} catch (e) {{ e.name }}")
                ),
                string(name),
                "{script}"
            );
        }
        // Failed operations leave the attribute untouched.
        assert_eq!(eval(&mut realm, "el.getAttribute('class')"), string("one"));
    }

    #[test]
    fn class_list_mutations_run_attribute_changed_callback() {
        let mut realm = realm_with("<div id='a' class='one'></div>");
        crate::register_window(&mut realm).unwrap();
        let program = js::parse(
            "class Watched {\
               attributeChangedCallback(name, oldValue, newValue) {\
                 this.calls = (this.calls || '') + name + ':' + oldValue + '>' + newValue + ';';\
               }\
             } Watched",
        )
        .unwrap();
        let constructor = js::Vm::new()
            .execute(&js::bytecode::compile(&program).unwrap())
            .unwrap();
        let Value::Function(constructor) = constructor else {
            panic!("expected class constructor");
        };
        let observed = js::Object::new();
        observed.set("0", string("class"));
        observed.set("length", Value::Number(1.0));
        assert!(constructor.set_own_property("observedAttributes", Value::Array(observed)));
        realm
            .register_global_value("Watched", Value::Function(constructor))
            .unwrap();
        eval(
            &mut realm,
            "customElements.define('x-watched', Watched);\
             var el = document.createElement('x-watched');\
             el.classList.add('a');\
             el.classList.add('a');\
             el.classList.toggle('a');",
        );
        assert_eq!(
            eval(&mut realm, "el.calls"),
            string("class:null>a;class:a>;")
        );
    }
}
