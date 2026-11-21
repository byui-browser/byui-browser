//! Regression coverage for shared Node behavior on Element wrappers.
use crate::node::TYPE_CONSTANTS as NODE_TYPE_CONSTANTS;
#[cfg(test)]
mod tests {
    use html::parse_raw_html;
    use js::{Realm, Value};

    use super::NODE_TYPE_CONSTANTS;
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
    fn node_type_is_element_node_for_get_element_by_id() {
        let mut realm = realm_with("<div id='a'><p id='b'></p></div>");
        assert_eq!(
            eval(&mut realm, "document.getElementById('a').nodeType"),
            Value::Number(1.0)
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('b').nodeType"),
            Value::Number(1.0)
        );
    }

    #[test]
    fn node_type_is_element_node_for_create_element() {
        let mut realm = realm_with("<div></div>");
        assert_eq!(
            eval(&mut realm, "document.createElement('span').nodeType"),
            Value::Number(1.0)
        );
    }

    #[test]
    fn node_type_compares_equal_to_element_node_constant() {
        let mut realm = realm_with("<div id='a'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "const el = document.getElementById('a'); el.nodeType === el.ELEMENT_NODE"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(
                &mut realm,
                "const c = document.createElement('p'); c.nodeType === c.ELEMENT_NODE && c.nodeType !== c.TEXT_NODE"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn node_type_constants_match_dom_standard() {
        let mut realm = realm_with("<div id='a'></div>");
        for (name, value) in NODE_TYPE_CONSTANTS {
            assert_eq!(
                eval(&mut realm, &format!("document.getElementById('a').{name}")),
                Value::Number(f64::from(value)),
                "{name}"
            );
        }
        assert_eq!(
            eval(&mut realm, "document.createElement('b').NOTATION_NODE"),
            Value::Number(12.0)
        );
    }

    #[test]
    fn node_type_and_constants_ignore_assignment() {
        let mut realm = realm_with("<div id='a'></div>");
        eval(
            &mut realm,
            "const el = document.getElementById('a'); \
             el.nodeType = 3; el.ELEMENT_NODE = 99; el.TEXT_NODE = 0;",
        );
        assert_eq!(eval(&mut realm, "el.nodeType"), Value::Number(1.0));
        assert_eq!(eval(&mut realm, "el.ELEMENT_NODE"), Value::Number(1.0));
        assert_eq!(eval(&mut realm, "el.TEXT_NODE"), Value::Number(3.0));
        assert_eq!(
            eval(&mut realm, "delete el.nodeType"),
            Value::Boolean(false)
        );
        assert_eq!(eval(&mut realm, "el.nodeType"), Value::Number(1.0));
    }
}
