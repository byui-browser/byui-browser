//! Public-contract tests for `net`.
//!
//! Run ignored tests with `cargo test -p net -- --ignored` to see the backlog.
//! None of these may touch a live network; use a local listener or a fake transport.

use net::{NetError, Request, Url, fetch};

#[test]
#[ignore = "TODO(net): URL parsing not implemented"]
fn parses_https_url() {
    let url = Url::parse("https://byui.instructure.com/login").expect("valid url");
    assert_eq!(url.as_str(), "https://byui.instructure.com/login");
}

#[test]
#[ignore = "TODO(net): URL parsing not implemented"]
fn rejects_non_http_scheme() {
    assert!(matches!(
        Url::parse("ftp://example.test/"),
        Err(NetError::UnsupportedScheme(_))
    ));
}

#[test]
#[ignore = "TODO(net): transport not implemented"]
fn fetch_returns_status_from_server() {
    // Expect the team to stand up a local listener here; no external hosts.
    let url = Url::parse("http://127.0.0.1:0/").expect("valid url");
    let response = fetch(&Request::get(url)).expect("response");
    assert_eq!(response.status, 200);
}
