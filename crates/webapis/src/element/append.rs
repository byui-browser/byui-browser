//! `ParentNode.append()` on script element wrappers.
//!
//! Strings become text nodes. Element wrappers are moved, so a node passed
//! in more than once ends up at its last position. The method returns
//! `undefined`. Hierarchy errors (`HierarchyRequestError`) leave the tree
//! unchanged and are reported as host errors, matching `appendChild()`.
//!
//! Custom elements that move into or out of the document receive
//! `disconnectedCallback` and `connectedCallback` the same way `appendChild()`
//! does.

use std::sync::{Arc, Mutex};

use js::{Object, Value};

use crate::document::Document;
use crate::element::Element;

use super::{mutate_tree, node_inputs};

/// Installs `ParentNode.append(...nodes)` on the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = Arc::clone(document);
    object.set_method(
        "append",
        Arc::new(move |arguments| {
            let (inputs, inserted) = node_inputs(arguments);
            log::debug!("Element.append() with {} arguments", arguments.len());
            mutate_tree(&document, &inserted, &[], |document| {
                document.append(node, &inputs)
            })?;
            Ok(Value::Undefined)
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

    #[test]
    fn append_inserts_strings_and_moves_elements() {
        let mut realm = realm_with("<div id='p'><a id='one'></a></div><b id='two'></b>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); \
                 parent.append('x<', document.getElementById('two'), 7); parent.outerHTML"
            ),
            string("<div id=\"p\"><a id=\"one\"></a>x&lt;<b id=\"two\"></b>7</div>")
        );
        assert_eq!(
            eval(
                &mut realm,
                "parent.append(document.getElementById('one')); parent.outerHTML"
            ),
            string("<div id=\"p\">x&lt;<b id=\"two\"></b>7<a id=\"one\"></a></div>")
        );
    }

    #[test]
    fn append_with_no_arguments_leaves_the_element_unchanged() {
        let mut realm = realm_with("<div id='p'><a id='one'></a></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); parent.append(); parent.outerHTML"
            ),
            string("<div id=\"p\"><a id=\"one\"></a></div>")
        );
    }

    #[test]
    fn append_rejects_an_ancestor_and_runs_connection_callbacks() {
        let mut realm = realm_with("<div id='p'><a id='one'></a></div>");
        assert!(
            realm
                .evaluate_script(
                    "document.getElementById('one').append(document.getElementById('p'))"
                )
                .unwrap_err()
                .to_string()
                .contains("HierarchyRequestError")
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('p').outerHTML"),
            string("<div id=\"p\"><a id=\"one\"></a></div>")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var log = ''; \
                 var fresh = document.createElement('b'); \
                 fresh.connectedCallback = () => { log = log + 'fresh;'; }; \
                 var one = document.getElementById('one'); \
                 one.disconnectedCallback = () => { log = log + 'out;'; }; \
                 one.connectedCallback = () => { log = log + 'in;'; }; \
                 document.getElementById('p').append(fresh, one); log"
            ),
            string("fresh;out;in;")
        );
    }
}
