//! Public-contract tests for `security`.
//!
//! Run ignored tests with `cargo test -p security -- --ignored` to see the backlog.

use security::{Cookie, CookieJar, Origin, same_origin};

#[test]
#[ignore = "TODO(security): origin parsing not implemented"]
fn parses_origin_with_default_port() {
    let origin = Origin::parse("https://byui.instructure.com/courses/1").expect("origin");
    assert_eq!(origin.scheme, "https");
    assert_eq!(origin.host, "byui.instructure.com");
    assert_eq!(origin.port, 443);
}

#[test]
#[ignore = "TODO(security): origin parsing not implemented"]
fn path_does_not_affect_origin() {
    let a = Origin::parse("https://example.test/a").expect("origin");
    let b = Origin::parse("https://example.test/b").expect("origin");
    assert!(same_origin(&a, &b));
}

#[test]
#[ignore = "TODO(security): cookie jar not implemented"]
fn secure_cookie_is_not_sent_over_http() {
    let https = Origin::parse("https://example.test/").expect("origin");
    let http = Origin::parse("http://example.test/").expect("origin");
    let mut jar = CookieJar::new();
    jar.store(
        &https,
        Cookie {
            name: "session".into(),
            value: "abc".into(),
            domain: "example.test".into(),
            secure: true,
        },
    );
    assert_eq!(jar.cookies_for(&https).len(), 1);
    assert!(jar.cookies_for(&http).is_empty());
}
