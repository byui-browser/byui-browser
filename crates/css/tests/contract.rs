//! Public-contract tests for `css`.
//!
//! Run ignored tests with `cargo test -p css -- --ignored` to see the backlog.

use css::{Selector, Specificity, parse_stylesheet, specificity};

#[test]
#[ignore = "TODO(css): parser not implemented"]
fn parses_one_rule() {
    let sheet = parse_stylesheet("p { color: red; }");
    assert_eq!(sheet.rules.len(), 1);
    assert_eq!(sheet.rules[0].selector, Selector("p".into()));
    assert_eq!(sheet.rules[0].declarations[0].property, "color");
    assert_eq!(sheet.rules[0].declarations[0].value, "red");
}

#[test]
#[ignore = "TODO(css): specificity not implemented"]
fn id_beats_class_beats_tag() {
    let id = specificity(&Selector("#main".into()));
    let class = specificity(&Selector(".card".into()));
    let tag = specificity(&Selector("div".into()));
    assert_eq!(id, Specificity(1, 0, 0));
    assert_eq!(class, Specificity(0, 1, 0));
    assert_eq!(tag, Specificity(0, 0, 1));
    assert!(id > class && class > tag);
}
