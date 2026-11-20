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
    fn first_child_returns_text_before_an_element_child() {
        let mut realm = realm_with("<div id='p'>hello<span id='s'></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var child = document.getElementById('p').firstChild; \
                 child.nodeType + ':' + child.nodeName + ':' + child.data + ':' + child.nodeValue \
                 + ':' + (child === document.getElementById('p').firstChild)"
            ),
            string("3:#text:hello:hello:true")
        );
    }

    #[test]
    fn first_child_of_an_element_is_the_shared_wrapper() {
        let mut realm = realm_with("<div id='p'><span id='s'></span>tail</div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('p').firstChild === document.getElementById('s')"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn first_child_returns_a_comment_and_null_when_empty() {
        let mut realm = realm_with("<div id='c'><!--note--><span></span></div><div id='e'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var comment = document.getElementById('c').firstChild; \
                 comment.nodeType + ':' + comment.data"
            ),
            string("8:note")
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('e').firstChild"),
            Value::Null
        );
        eval(
            &mut realm,
            "document.getElementById('e').firstChild = document.getElementById('c');",
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('e').firstChild"),
            Value::Null
        );
    }
}
