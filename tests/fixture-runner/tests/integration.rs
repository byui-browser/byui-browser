//! Integration fixtures for the joins between teams.
use fixture_runner::{dump_dom, dump_styles, fetch, read, read_http, run_group, serve_once};

/// HTML + CSS in, computed styles out (`css::style_document`).
#[test]
fn parse_style_fixtures() {
    run_group("integration/parse-style", |case| {
        let dom = html::parse_raw_html(read(case, "input.html"));
        let sheet = css::parse_stylesheet(&read(case, "input.css"));
        dump_styles(&dom, &css::style_document(&dom, &[sheet]))
    });
}

/// A recorded HTML response served locally in, DOM out.
#[test]
fn net_parse_fixtures() {
    run_group("integration/net-parse", |case| {
        let url = serve_once(read_http(case));
        let (_, body) = fetch(&url).expect("fetch");
        dump_dom(&html::parse_raw_html(
            String::from_utf8_lossy(&body).into_owned(),
        ))
    });
}
