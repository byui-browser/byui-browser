//! `Element.hasAttribute()` on script element wrappers.

use std::sync::{Arc, Mutex};

use js::{JsError, JsResult, Object, Value};

use crate::document::Document;
use crate::element::Element;

/// Installs `Element.hasAttribute()` on the wrapper `object` for `element`.
///
/// The method follows the DOM Standard's `hasAttribute(qualifiedName)`
/// algorithm: the argument is converted with JavaScript `ToString`, HTML
/// elements match attribute names after ASCII lowercasing, and other
/// namespaces (SVG, MathML) match the name exactly. Each call reads a fresh
/// element snapshot from `document`, so later `setAttribute()` changes are
/// observed. Calling it with no argument throws a `TypeError`.
///
/// Limitation: `ToString` on objects uses the engine's built-in formatting
/// and does not invoke a user-defined `toString()` method. The HTML parser
/// also lowercases every attribute name, including on SVG elements, so
/// parsed SVG attributes such as `viewBox` are stored as `viewbox`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = Arc::clone(document);
    object.set_method(
        "hasAttribute",
        Arc::new(move |arguments: &[Value]| {
            let qualified_name = qualified_name_argument(arguments)?;
            let snapshot = document.lock().unwrap().element_snapshot(node);
            let Some(element) = snapshot else {
                log::debug!("hasAttribute on a node that is no longer in the document");
                return Ok(Value::Boolean(false));
            };
            log::trace!("Element.hasAttribute({qualified_name:?})");
            Ok(Value::Boolean(has_attribute(&element, &qualified_name)))
        }),
    );
}

/// Converts the first argument with `ToString`, throwing when it is missing.
fn qualified_name_argument(arguments: &[Value]) -> JsResult<String> {
    match arguments.first() {
        Some(value) => Ok(value.to_string()),
        None => {
            log::debug!("Element.hasAttribute() called without arguments");
            Err(JsError::new(
                "TypeError: Element.hasAttribute() requires 1 argument, but only 0 present",
            ))
        }
    }
}

/// Returns whether `element` has an attribute matching `qualified_name`.
fn has_attribute(element: &Element, qualified_name: &str) -> bool {
    element.get_attribute(qualified_name).is_some()
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

    fn eval(realm: &Realm, script: &str) -> Value {
        realm
            .evaluate_script(script)
            .unwrap_or_else(|error| panic!("{script}: {error:?}"))
    }

    #[test]
    fn has_attribute_returns_booleans_for_present_and_missing_attributes() {
        let realm = realm_with("<main id='app' data-state='ready' hidden></main>");
        assert_eq!(
            eval(
                &realm,
                "document.getElementById('app').hasAttribute('data-state')"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(
                &realm,
                "document.getElementById('app').hasAttribute('hidden')"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(
                &realm,
                "document.getElementById('app').hasAttribute('missing')"
            ),
            Value::Boolean(false)
        );
    }

    #[test]
    fn has_attribute_matches_html_names_ascii_case_insensitively() {
        let realm = realm_with("<main id='app' data-state='ready'></main>");
        assert_eq!(
            eval(
                &realm,
                "document.getElementById('app').hasAttribute('DATA-State')"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(&realm, "document.getElementById('app').hasAttribute('ID')"),
            Value::Boolean(true)
        );
    }

    #[test]
    fn has_attribute_reflects_later_set_attribute_calls() {
        let realm = realm_with("<main id='app'></main>");
        assert_eq!(
            eval(
                &realm,
                "var app = document.getElementById('app'); \
                 var before = app.hasAttribute('data-added'); \
                 app.setAttribute('DATA-ADDED', 'yes'); \
                 before + ':' + app.hasAttribute('data-added')"
            ),
            Value::String("false:true".to_owned())
        );
    }

    #[test]
    fn has_attribute_converts_argument_with_to_string() {
        let realm = realm_with("<main id='app' null='1' undefined='2' 42='3'></main>");
        assert_eq!(
            eval(
                &realm,
                "var app = document.getElementById('app'); \
                 app.hasAttribute(null) + ':' + app.hasAttribute(undefined) + ':' \
                 + app.hasAttribute(42) + ':' + app.hasAttribute(true)"
            ),
            Value::String("true:true:true:false".to_owned())
        );
    }

    #[test]
    fn has_attribute_without_argument_throws_type_error() {
        let realm = realm_with("<main id='app'></main>");
        let error = realm
            .evaluate_script("document.getElementById('app').hasAttribute()")
            .unwrap_err();
        assert!(error.to_string().contains("TypeError"), "{error}");
        assert_eq!(
            eval(
                &realm,
                "var caught = 'none'; \
                 try { document.getElementById('app').hasAttribute(); } \
                 catch (error) { caught = 'threw'; } \
                 caught"
            ),
            Value::String("threw".to_owned())
        );
    }

    #[test]
    fn has_attribute_matches_svg_names_case_sensitively() {
        let realm = realm_with("<svg id='icon'></svg>");
        assert_eq!(
            eval(
                &realm,
                "var icon = document.getElementById('icon'); \
                 icon.setAttribute('viewBox', '0 0 1 1'); \
                 icon.hasAttribute('viewBox') + ':' + icon.hasAttribute('viewbox')"
            ),
            Value::String("true:false".to_owned())
        );
    }

    #[test]
    fn has_attribute_on_shared_wrapper_returns_boolean_type() {
        let realm = realm_with("<main id='app' title='t'></main>");
        assert_eq!(
            eval(
                &realm,
                "typeof document.getElementById('app').hasAttribute('title')"
            ),
            Value::String("boolean".to_owned())
        );
    }
}
