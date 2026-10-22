//! HTML team fixtures: `input.html` -> DOM tree dump.
use fixture_runner::{dump_dom, read, run_group};

#[test]
fn html_fixtures() {
    run_group("html", |case| {
        dump_dom(&html::parse_raw_html(read(case, "input.html")))
    });
}
