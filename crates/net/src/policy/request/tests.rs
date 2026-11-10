use std::io;

use futures_util::stream;

use super::{FetchScheme, RequestPolicy};
use crate::api::{error::RequestError, request::Request};

#[test]
/// Verifies HTTP and HTTPS URLs pass request validation.
fn accepts_http_and_https_urls() {
    let policy = RequestPolicy;
    assert!(
        policy
            .validate_request(&Request::get("http://example.test"), 65_536)
            .is_ok()
    );
    assert!(
        policy
            .validate_request(&Request::get("https://example.test"), 65_536)
            .is_ok()
    );
}

#[test]
/// Verifies streaming request bodies require explicit half-duplex mode.
fn streaming_bodies_require_half_duplex_mode() {
    let mut request = Request::new("https://example.test/upload", "POST").unwrap();
    request
        .set_body(Some(crate::Body::stream(stream::empty::<
            Result<Vec<u8>, io::Error>,
        >())))
        .unwrap();
    assert!(matches!(
        RequestPolicy.validate_request(&request, 65_536),
        Err(RequestError::StreamingBodyRequiresDuplex)
    ));

    request.set_duplex(Some(crate::RequestDuplex::Half));
    assert!(RequestPolicy.validate_request(&request, 65_536).is_ok());
}

#[test]
/// Verifies malformed URLs fail before transport dispatch.
fn rejects_invalid_urls_before_transport() {
    let error = RequestPolicy
        .validate_request(&Request::get("not a URL"), 65_536)
        .unwrap_err();

    assert!(matches!(error, RequestError::InvalidUrl(url) if url == "not a URL"));
}

#[test]
/// Verifies relative URLs are rejected by the network boundary.
fn rejects_relative_urls() {
    let error = RequestPolicy
        .validate_request(&Request::get("/relative/path"), 65_536)
        .unwrap_err();

    assert!(matches!(error, RequestError::InvalidUrl(url) if url == "/relative/path"));
}

#[test]
/// Verifies invalid ports produce a typed validation error.
fn rejects_urls_with_invalid_ports() {
    for value in [
        "http://example.test:not-a-port/",
        "http://example.test:65536/",
    ] {
        let error = RequestPolicy
            .validate_request(&Request::get(value), 65_536)
            .unwrap_err();

        assert!(matches!(error, RequestError::InvalidUrl(url) if url == value));
    }
}

#[test]
/// Verifies uppercase HTTP schemes normalize to the canonical URL.
fn accepts_uppercase_http_urls_and_returns_normalized_url() {
    let validated = RequestPolicy
        .validate_request(&Request::get("HTTP://EXAMPLE.TEST/Path"), 65_536)
        .unwrap();

    assert_eq!(validated.url.scheme(), "http");
    assert_eq!(validated.url.host_str(), Some("example.test"));
    assert_eq!(validated.url.path(), "/Path");
    assert_eq!(validated.scheme, FetchScheme::Http);
}

#[test]
/// Verifies unsupported schemes are rejected by the HTTP request policy.
fn rejects_non_http_schemes() {
    let error = RequestPolicy
        .validate_request(&Request::get("ftp://example.test/file"), 65_536)
        .unwrap_err();

    assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
}

#[test]
/// Verifies supported local schemes are classified for dispatch.
fn classifies_supported_non_http_schemes_for_dispatch() {
    for (url, expected) in [
        ("data:text/plain,hello", FetchScheme::Data),
        ("blob:https://example.test/id", FetchScheme::Blob),
        ("file:///tmp/page.html", FetchScheme::File),
        ("byui://welcome", FetchScheme::Byui),
        ("about:blank", FetchScheme::About),
    ] {
        let validated = RequestPolicy
            .validate_request(&Request::get(url), 65_536)
            .unwrap();
        assert_eq!(validated.scheme, expected);
    }
}

#[test]
fn navigation_mode_is_available_only_to_file_providers() {
    let policy = RequestPolicy;
    let mut file = Request::get("file:///tmp/page.html");
    file.set_mode(crate::RequestMode::Navigate).unwrap();
    assert_eq!(
        policy.validate_request(&file, 65_536).unwrap().scheme,
        FetchScheme::File
    );

    let mut http = Request::get("https://example.test/page.html");
    http.set_mode(crate::RequestMode::Navigate).unwrap();
    assert!(matches!(
        policy.validate_request(&http, 65_536),
        Err(RequestError::UnsupportedFeature("navigation fetch"))
    ));
}

#[test]
/// Verifies unsupported integrity metadata fails before transport.
fn unsupported_integrity_is_rejected_before_transport() {
    let mut request = Request::get("https://example.test/data");
    request.set_integrity(Some("sha256-example".into()));
    let error = RequestPolicy
        .validate_request(&request, 65_536)
        .unwrap_err();
    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("subresource integrity")
    ));
}

#[test]
/// Verifies cross-origin requests requiring preflight reach HTTP execution.
fn preflight_required_cross_origin_request_passes_request_validation() {
    let mut request = Request::new("https://remote.test/data", "PUT").unwrap();
    request.context.environment.origin = Some(crate::Origin::parse("https://client.test").unwrap());
    assert!(RequestPolicy.validate_request(&request, 65_536).is_ok());
}

#[test]
/// Verifies keepalive requests respect the configured body-size boundary.
fn keepalive_request_is_rejected_until_lifetime_is_supported() {
    let mut request = Request::get("https://example.test/");
    request.set_keepalive(true);
    let error = RequestPolicy
        .validate_request(&request, 65_536)
        .unwrap_err();
    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("keepalive lifetime")
    ));
}
