//! `Element.querySelector()` on script element wrappers.
//!
//! Limitation: selector support is whatever
//! [`Document::query_selector_all_within`] accepts today: type selectors, `*`,
//! `#id`, `.class`, compounds of those, and comma-separated selector lists.
//! Combinators (descendant, child, sibling), attribute selectors, and
//! pseudo-classes are not supported and are reported as `SyntaxError`.

use std::sync::{Arc, Mutex};

use js::{JsError, Object, Value};

use crate::document::Document;
use crate::element::{Element, wrap_element};

/// Installs `Element.querySelector()` on the wrapper `object` for `element`.
///
/// `querySelector(selectors)` returns the shared wrapper of the first
/// descendant of `element`, in tree order, that matches `selectors`, or
/// `null` when nothing matches. The element itself and nodes outside its
/// subtree never match. The argument is converted with `ToString`; a missing
/// argument throws `TypeError` and an unparsable or unsupported selector
/// throws `SyntaxError` (see the module docs for the supported syntax).
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = document.clone();
    object.set_method(
        "querySelector",
        Arc::new(move |arguments| {
            let Some(selector) = arguments.first() else {
                return Err(JsError::new(
                    "TypeError: Element.querySelector() expects a selector string",
                ));
            };
            let selector = selector.to_string();
            // Release the document lock before wrapping: `wrap_element`
            // locks the document itself.
            let first = document
                .lock()
                .unwrap()
                .query_selector_all_within(node, &selector)
                .map_err(|error| JsError::new(format!("SyntaxError: {error}")))?
                .into_iter()
                .next();
            match first {
                Some(matched) => wrap_element(&document, matched),
                None => Ok(Value::Null),
            }
        }),
    );
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

    const MARKUP: &str = "<p class='x' id='before'></p>\
        <div id='root' class='x'><span><b class='x' id='deep'></b></span><p class='x' id='later'></p></div>\
        <p class='x' id='after'></p>";

    #[test]
    fn element_query_selector_returns_first_descendant_in_tree_order() {
        let mut realm = realm_with(MARKUP);
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').querySelector('.x').getAttribute('id')"
            ),
            string("deep")
        );
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').querySelector('p').getAttribute('id')"
            ),
            string("later")
        );
    }

    #[test]
    fn element_query_selector_returns_null_without_match() {
        let mut realm = realm_with(MARKUP);
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').querySelector('i')"
            ),
            Value::Null
        );
        // Elements outside the subtree never match.
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').querySelector('#after')"
            ),
            Value::Null
        );
    }

    #[test]
    fn element_query_selector_returns_shared_wrapper() {
        let mut realm = realm_with(MARKUP);
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').querySelector('#deep') === document.getElementById('deep')"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn element_query_selector_excludes_the_element_itself() {
        let mut realm = realm_with(MARKUP);
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').querySelector('#root')"
            ),
            Value::Null
        );
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').querySelector('div')"
            ),
            Value::Null
        );
    }

    #[test]
    fn element_query_selector_accepts_selector_lists() {
        let mut realm = realm_with(MARKUP);
        // The first match in tree order wins, not the first selector in the list.
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').querySelector('p.x, #deep, i').getAttribute('id')"
            ),
            string("deep")
        );
    }

    #[test]
    fn element_query_selector_throws_syntax_error_for_invalid_selectors() {
        let realm = realm_with(MARKUP);
        let error = realm
            .evaluate_script("document.getElementById('root').querySelector('##')")
            .unwrap_err();
        assert!(
            format!("{error:?}").contains("SyntaxError"),
            "unexpected error: {error:?}"
        );
        let error = realm
            .evaluate_script("document.getElementById('root').querySelector()")
            .unwrap_err();
        assert!(
            format!("{error:?}").contains("TypeError"),
            "unexpected error: {error:?}"
        );
    }
}
