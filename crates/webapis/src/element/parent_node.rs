//! Regression coverage for shared Node behavior on Element wrappers.
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

    #[test]
    fn parent_node_is_the_parent_element_or_the_document() {
        let mut realm = realm_with("<div id='p'><span id='c'></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('c').parentNode === document.getElementById('p') \
                 && document.getElementById('p').parentNode === document \
                 && document.createElement('b').parentNode === null"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn parent_node_follows_append_and_ignores_assignment() {
        let mut realm = realm_with("<div id='p'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var child = document.createElement('span'); \
                 document.getElementById('p').append(child); \
                 child.parentNode = null; \
                 child.parentNode === document.getElementById('p')"
            ),
            Value::Boolean(true)
        );
    }
}
