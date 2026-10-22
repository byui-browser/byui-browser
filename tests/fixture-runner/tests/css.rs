//! CSS team fixtures.
//!
//! - `css/parse/*`: `input.css` -> `{:#?}` of the parsed stylesheet.
//! - `css/match/*`: `input.dom` (hand-built DOM) + `input.css` -> computed styles.
use fixture_runner::{dump_styles, parse_dom_data, read, run_group};

#[test]
fn css_parse_fixtures() {
    run_group("css/parse", |case| {
        format!("{:#?}\n", css::parse_stylesheet(&read(case, "input.css")))
    });
}

#[test]
fn css_match_fixtures() {
    run_group("css/match", |case| {
        let dom = parse_dom_data(&read(case, "input.dom"));
        let sheet = css::parse_stylesheet(&read(case, "input.css"));
        dump_styles(&dom, &css::style_document(&dom, &[sheet]))
    });
}
