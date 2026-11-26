//! Regression coverage for the shared `Node.lastElementChild` getter.

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
    fn last_element_child_skips_text_and_comments() {
        let mut realm = realm_with(
            "<div id='parent'><span id='first'></span><!--comment-->text<b id='last'></b></div>",
        );
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('parent'); \
                 parent.lastElementChild === document.getElementById('last') && \
                 parent.lastElementChild === parent.children[parent.children.length - 1]"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn last_element_child_is_null_when_no_element_child_exists() {
        let mut realm = realm_with("<div id='parent'>text<!--comment--></div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('parent').lastElementChild"
            ),
            Value::Null
        );
    }

    #[test]
    fn last_element_child_is_live_and_read_only() {
        let mut realm = realm_with("<div id='parent'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('parent'); \
                 var first = document.createElement('i'); \
                 var second = document.createElement('b'); \
                 parent.lastElementChild = first; parent.append(first, second); \
                 var initial = parent.lastElementChild === second; \
                 parent.insertBefore(second, first); \
                 var reordered = parent.lastElementChild === first; \
                 parent.removeChild(first); \
                 reordered && initial && parent.lastElementChild === second"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn document_last_element_child_returns_the_document_element() {
        let mut realm = realm_with("<!doctype html><html><body></body></html>");
        assert_eq!(
            eval(&mut realm, "document.lastElementChild.nodeName === 'HTML'"),
            Value::Boolean(true)
        );
    }

    #[test]
    fn last_element_child_is_only_exposed_on_parent_nodes() {
        let mut realm = realm_with("<div id='parent'>text<!--comment--></div>");
        assert_eq!(
            eval(
                &mut realm,
                "typeof document.getElementById('parent').firstChild.lastElementChild"
            ),
            Value::String("undefined".into())
        );
    }
}
