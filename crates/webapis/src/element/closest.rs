//! `Element.closest()` on script element wrappers.

use std::sync::{Arc, Mutex};

use html::{NodeID, NodeKind};
use js::{JsError, Object, Value};

use crate::document::Document;
use crate::element::{Element, wrap_element};
use crate::selector::{SelectorError, parse_selector_list};

/// Installs `Element.closest(selectors)` on the wrapper `object` for
/// `element`.
///
/// `closest` walks the element's inclusive ancestors (the element itself,
/// then its parent element, and so on) and returns the shared wrapper of the
/// first one matching `selectors`, or `null` when none matches. The walk stops
/// at the first non-element ancestor (such as the `Document`), so detached
/// subtrees built with `createElement`/`appendChild` work as expected.
///
/// Limitation: selectors use the crate's simple selector engine, which
/// supports type selectors, `*`, `#id`, `.class`, compounds of those, and
/// comma-separated lists. Combinators (`>`, descendant, `+`, `~`), attribute
/// selectors, and pseudo-classes are rejected with a `SyntaxError`. A missing
/// argument throws a `TypeError`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = document.clone();
    object.set_method(
        "closest",
        Arc::new(move |arguments| {
            let Some(selector) = arguments.first() else {
                return Err(JsError::new(
                    "TypeError: Element.closest() expects a selector string",
                ));
            };
            let selector = selector.to_string();
            // Find the match while holding the lock, then release it before
            // `wrap_element`, which locks the document itself.
            let found = closest_node(&document.lock().unwrap(), node, &selector)
                .map_err(|error| JsError::new(format!("SyntaxError: {error}")))?;
            match found {
                Some(found) => wrap_element(&document, found),
                None => Ok(Value::Null),
            }
        }),
    );
}

/// Returns the first inclusive ancestor of `start` that is an element
/// matching `selector`, stopping at the first non-element node.
fn closest_node(
    document: &Document,
    start: NodeID,
    selector: &str,
) -> Result<Option<NodeID>, SelectorError> {
    let selectors = parse_selector_list(selector)?;
    let Some(source) = document.html_document() else {
        return Ok(None);
    };
    let mut current = Some(start);
    while let Some(id) = current {
        let Some(node) = source.get_node(id) else {
            break;
        };
        let NodeKind::Element(data) = node.get_kind() else {
            break;
        };
        if selectors.iter().any(|selector| selector.matches(data)) {
            return Ok(Some(id));
        }
        current = node.get_parent();
    }
    Ok(None)
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

    const MARKUP: &str = "<section id='outer' class='box'><div id='mid' class='box inner'>\
                          <p id='leaf' class='text'></p></div></section>";

    #[test]
    fn closest_matches_the_element_itself_first() {
        let mut realm = realm_with(MARKUP);
        assert_eq!(
            eval(
                &mut realm,
                "var leaf = document.getElementById('leaf'); leaf.closest('p') === leaf"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(
                &mut realm,
                "var mid = document.getElementById('mid'); mid.closest('.box') === mid"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn closest_returns_the_nearest_matching_ancestor_wrapper() {
        let mut realm = realm_with(MARKUP);
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('leaf').closest('.box') === document.getElementById('mid')"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('leaf').closest('section.box') === document.getElementById('outer')"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('leaf').closest('#outer').getAttribute('class')"
            ),
            Value::String("box".to_owned())
        );
    }

    #[test]
    fn closest_returns_null_when_no_inclusive_ancestor_matches() {
        let mut realm = realm_with(MARKUP);
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('leaf').closest('table')"
            ),
            Value::Null
        );
        // Ancestors only: a descendant match does not count.
        assert_eq!(
            eval(&mut realm, "document.getElementById('outer').closest('p')"),
            Value::Null
        );
    }

    #[test]
    fn closest_accepts_selector_lists() {
        let mut realm = realm_with(MARKUP);
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('leaf').closest('table, .inner, section') === document.getElementById('mid')"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn closest_works_on_detached_subtrees() {
        let mut realm = realm_with("<main id='root'></main>");
        assert_eq!(
            eval(
                &mut realm,
                "var wrapper = document.createElement('article'); \
                 wrapper.setAttribute('class', 'card'); \
                 var child = document.createElement('span'); \
                 wrapper.appendChild(child); \
                 child.closest('.card') === wrapper"
            ),
            Value::Boolean(true)
        );
        assert_eq!(eval(&mut realm, "child.closest('main')"), Value::Null);
    }

    #[test]
    fn closest_rejects_invalid_selectors_and_missing_arguments() {
        let realm = realm_with(MARKUP);
        let error = realm
            .evaluate_script("document.getElementById('leaf').closest('div > p')")
            .unwrap_err()
            .to_string();
        assert!(error.contains("SyntaxError"), "{error}");
        let error = realm
            .evaluate_script("document.getElementById('leaf').closest()")
            .unwrap_err()
            .to_string();
        assert!(error.contains("TypeError"), "{error}");
    }
}
