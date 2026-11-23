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

    #[test]
    fn parent_element_does_not_return_non_element_parents() {
        let mut realm = realm_with("<!doctype html><main id='p'>text<!--comment--></main>");
        assert_eq!(
            eval(
                &mut realm,
                "var p = document.getElementById('p'); \
                 var text = p.firstChild; var comment = text.nextSibling; \
                 var doctype = document.firstChild; \
                 text.parentElement === p && comment.parentElement === p && \
                 doctype.parentElement === null && document.parentElement === null"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn parent_element_is_live_and_returns_the_existing_wrapper() {
        let mut realm =
            realm_with("<main id='a'><span id='child'></span></main><section id='b'></section>");
        assert_eq!(
            eval(
                &mut realm,
                "var a = document.getElementById('a'); var b = document.getElementById('b'); \
                 var child = document.getElementById('child'); \
                 var initial = child.parentElement === a; \
                 b.appendChild(child); \
                 var moved = child.parentElement === b && child.parentElement === b; \
                 b.removeChild(child); \
                 initial && moved && child.parentElement === null"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn parent_element_is_immediate_and_null_under_a_document() {
        let mut realm = realm_with(
            "<main id='outer'><section id='middle'><span id='leaf'></span></section></main>",
        );
        assert_eq!(
            eval(
                &mut realm,
                "var outer = document.getElementById('outer'); \
                 var middle = document.getElementById('middle'); \
                 var leaf = document.getElementById('leaf'); \
                 var detachedDocument = new Document(); \
                 var detachedRoot = detachedDocument.createElement('article'); \
                 detachedDocument.appendChild(detachedRoot); \
                 leaf.parentElement === middle && middle.parentElement === outer && \
                 outer.parentElement === null && detachedRoot.parentElement === null"
            ),
            Value::Boolean(true)
        );
    }
}
