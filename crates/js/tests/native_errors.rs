//! `Error`, `RangeError`, `ReferenceError`, `URIError`, and
//! `decodeURIComponent`, in both the persistent realm and the standalone VM.

use js::{Realm, Value, eval};

fn string(text: &str) -> Value {
    Value::String(text.into())
}

fn both(source: &str) -> Value {
    let standalone = eval(source).unwrap();
    let realm = Realm::new().evaluate_script(source).unwrap();
    assert_eq!(standalone, realm, "{source}");
    standalone
}

#[test]
fn range_and_reference_errors_carry_name_and_message() {
    assert_eq!(both("new RangeError('too big').message"), string("too big"));
    assert_eq!(both("new RangeError('too big').name"), string("RangeError"));
    assert_eq!(
        both("ReferenceError('gone').name"),
        string("ReferenceError")
    );
    assert_eq!(both("new ReferenceError().message"), string(""));
    assert_eq!(
        both("new RangeError('too big').toString()"),
        string("RangeError: too big")
    );
    assert_eq!(
        both("new ReferenceError().toString()"),
        string("ReferenceError")
    );
}

#[test]
fn native_errors_inherit_from_error() {
    assert_eq!(
        both(
            "const e = new RangeError('x'); \
             e instanceof RangeError && e instanceof Error && !(e instanceof ReferenceError)"
        ),
        Value::Boolean(true)
    );
    assert_eq!(
        both("new ReferenceError('x') instanceof Error"),
        Value::Boolean(true)
    );
    // The tree-walk realm cannot read function properties yet, so the
    // prototype link is checked in the standalone VM only.
    assert_eq!(
        eval("RangeError.prototype.constructor === RangeError").unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        both("new Error('m', { cause: 7 }).cause"),
        Value::Number(7.0)
    );
}

#[test]
fn thrown_native_errors_are_caught_unchanged() {
    assert_eq!(
        both(
            "try { throw new RangeError('bad length') } \
             catch (e) { e instanceof RangeError ? e.message : 'wrong' }"
        ),
        string("bad length")
    );
}

#[test]
fn decode_uri_component_decodes_escapes() {
    assert_eq!(
        both("decodeURIComponent('a%20b%26c%3Dd')"),
        string("a b&c=d")
    );
    assert_eq!(both("decodeURIComponent('%E2%82%AC')"), string("€"));
    assert_eq!(both("decodeURIComponent('plain')"), string("plain"));
}

#[test]
fn decode_uri_component_throws_uri_error_on_malformed_input() {
    assert_eq!(
        both(
            "try { decodeURIComponent('%E2%82') } \
             catch (e) { e.name + '|' + (e instanceof URIError) + '|' + (e instanceof Error) }"
        ),
        string("URIError|true|true")
    );
}
