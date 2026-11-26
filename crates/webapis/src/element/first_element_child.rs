//! Regression coverage for the shared `Node.firstElementChild` getter.

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
    fn first_element_child_skips_text_and_comments() {
        let mut realm = realm_with(
            "<div id='parent'>text<!--comment--><span id='first'></span><b id='second'></b></div>",
        );
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('parent'); \
                 parent.firstElementChild === document.getElementById('first') && \
                 parent.firstElementChild === parent.children[0]"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn first_element_child_is_null_when_no_element_child_exists() {
        let mut realm = realm_with("<div id='parent'>text<!--comment--></div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('parent').firstElementChild"
            ),
            Value::Null
        );
    }

    #[test]
    fn first_element_child_is_live_and_read_only() {
        let mut realm = realm_with("<div id='parent'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('parent'); \
                 var first = document.createElement('i'); \
                 var second = document.createElement('b'); \
                 parent.firstElementChild = second; parent.append(first); \
                 var initial = parent.firstElementChild === first; \
                 parent.insertBefore(second, first); \
                 var reordered = parent.firstElementChild === second; \
                 parent.removeChild(second); \
                 reordered && initial && parent.firstElementChild === first"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn document_first_element_child_returns_the_document_element() {
        let mut realm = realm_with("<!doctype html><html><body></body></html>");
        assert_eq!(
            eval(&mut realm, "document.firstElementChild.nodeName === 'HTML'"),
            Value::Boolean(true)
        );
    }

    #[test]
    fn first_element_child_is_only_exposed_on_parent_nodes() {
        let mut realm = realm_with("<div id='parent'>text<!--comment--></div>");
        assert_eq!(
            eval(
                &mut realm,
                "typeof document.getElementById('parent').firstChild.firstElementChild"
            ),
            Value::String("undefined".into())
        );
    }
}
