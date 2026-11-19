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
    fn parent_element_is_the_element_parent_or_null() {
        let mut realm = realm_with("<div id='p'><span id='c'></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('c').parentElement === document.getElementById('p')"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('p').parentElement"),
            Value::Null
        );
        assert_eq!(
            eval(&mut realm, "document.createElement('b').parentElement"),
            Value::Null
        );
        eval(
            &mut realm,
            "var child = document.getElementById('c'); child.parentElement = null;",
        );
        assert_eq!(
            eval(
                &mut realm,
                "child.parentElement === document.getElementById('p')"
            ),
            Value::Boolean(true)
        );
    }
}
