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

    fn string(value: &str) -> Value {
        Value::String(value.to_owned())
    }

    #[test]
    fn contains_is_true_for_self_and_descendants() {
        let mut realm = realm_with("<div id='p'>text<span id='c'><i id='g'></i></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); \
                 var child = document.getElementById('c'); \
                 var grand = document.getElementById('g'); \
                 parent.contains(parent) && parent.contains(child) && parent.contains(grand) \
                 && parent.contains(parent.firstChild) && child.contains(grand) \
                 && !child.contains(parent) && !parent.contains(null) && !parent.contains(undefined)"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn contains_rejects_a_non_node() {
        let mut realm = realm_with("<div id='p'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('p').contains('x'); } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("TypeError")
        );
    }
}
