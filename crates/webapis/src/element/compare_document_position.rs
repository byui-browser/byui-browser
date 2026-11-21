//! Regression coverage for shared Node behavior on Element wrappers.
const DISCONNECTED: u16 = 1;
const PRECEDING: u16 = 2;
const FOLLOWING: u16 = 4;
const CONTAINS: u16 = 8;
const CONTAINED_BY: u16 = 16;
#[cfg(test)]
mod tests {
    use html::parse_raw_html;
    use js::{Realm, Value};

    use super::{CONTAINED_BY, CONTAINS, DISCONNECTED, FOLLOWING, PRECEDING};
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

    fn number(value: u16) -> Value {
        Value::Number(f64::from(value))
    }

    #[test]
    fn compare_document_position_reports_ancestors_descendants_and_siblings() {
        let mut realm = realm_with("<div id='p'><a id='a'></a><b id='b'></b></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); \
                 var a = document.getElementById('a'); \
                 var b = document.getElementById('b'); \
                 parent.compareDocumentPosition(parent) + ':' \
                 + parent.compareDocumentPosition(a) + ':' \
                 + a.compareDocumentPosition(parent) + ':' \
                 + a.compareDocumentPosition(b) + ':' \
                 + b.compareDocumentPosition(a) + ':' \
                 + a.compareDocumentPosition(document)"
            ),
            string(&format!(
                "0:{}:{}:{}:{}:{}",
                CONTAINED_BY | FOLLOWING,
                CONTAINS | PRECEDING,
                FOLLOWING,
                PRECEDING,
                CONTAINS | PRECEDING
            ))
        );
        assert_eq!(
            eval(&mut realm, "parent.DOCUMENT_POSITION_CONTAINED_BY"),
            number(CONTAINED_BY)
        );
        assert_eq!(
            eval(&mut realm, "parent.DOCUMENT_POSITION_DISCONNECTED"),
            number(DISCONNECTED)
        );
    }

    #[test]
    fn compare_document_position_marks_disconnected_nodes() {
        let mut realm = realm_with("<div id='p'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); \
                 var created = document.createElement('i'); \
                 (parent.compareDocumentPosition(created) & parent.DOCUMENT_POSITION_DISCONNECTED) !== 0 \
                 && (parent.compareDocumentPosition(created) & parent.DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC) !== 0 \
                 && (created.compareDocumentPosition(parent) & created.DOCUMENT_POSITION_DISCONNECTED) !== 0"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn compare_document_position_requires_a_node() {
        let mut realm = realm_with("<div id='p'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('p').compareDocumentPosition(); } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("TypeError")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('p').compareDocumentPosition(null); } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("TypeError")
        );
    }
}
