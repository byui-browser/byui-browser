use http::header::{HeaderMap, HeaderName, HeaderValue};

use super::{HeaderGuard, Headers, cors_unsafe_request_header_names, is_cors_safelisted_header};
use crate::api::{error::RequestError, request::CredentialsMode};

/// Builds a valid static header value for header behavior tests.
fn value(value: &'static str) -> HeaderValue {
    HeaderValue::from_static(value)
}

#[test]
/// Verifies ordinary duplicates combine while cookie values stay separate.
fn headers_combine_duplicate_values_and_preserve_cookie_values() {
    let mut headers = Headers::with_guard(HeaderGuard::None);
    headers
        .append(HeaderName::from_static("x-test"), value("one"))
        .unwrap();
    headers
        .append(HeaderName::from_bytes(b"X-Test").unwrap(), value("two"))
        .unwrap();
    headers
        .append(HeaderName::from_static("set-cookie"), value("a=1"))
        .unwrap();
    headers
        .append(HeaderName::from_static("set-cookie"), value("b=2"))
        .unwrap();

    assert_eq!(headers.get("x-test").as_deref(), Some("one, two"));
    assert_eq!(headers.get_set_cookie(), ["a=1", "b=2"]);
    assert_eq!(
        headers
            .iter_raw()
            .map(|(name, _)| name.as_str().to_owned())
            .collect::<Vec<_>>(),
        ["x-test", "x-test", "set-cookie", "set-cookie"]
    );
}

#[test]
/// Verifies set and delete follow guarded Fetch mutation semantics.
fn set_and_delete_have_fetch_mutation_semantics() {
    let mut headers = Headers::new();
    headers
        .append(HeaderName::from_static("x-test"), value("one"))
        .unwrap();
    headers
        .append(HeaderName::from_static("x-test"), value("two"))
        .unwrap();
    headers
        .set(HeaderName::from_static("x-test"), value("three"))
        .unwrap();
    assert_eq!(headers.get("x-test").as_deref(), Some("three"));
    headers.delete("X-TEST").unwrap();
    assert!(!headers.has("x-test"));
}

#[test]
/// Verifies immutable headers reject every public mutation path.
fn immutable_headers_reject_all_mutation() {
    let mut headers = Headers::with_guard(HeaderGuard::Immutable);
    assert!(matches!(
        headers.append(HeaderName::from_static("x-test"), value("one")),
        Err(RequestError::ImmutableHeaders)
    ));
    assert!(matches!(
        headers.delete("x-test"),
        Err(RequestError::ImmutableHeaders)
    ));
}

#[test]
/// Verifies changing to no-CORS removes headers that are no longer allowed.
fn no_cors_guard_removes_existing_unsafelisted_headers() {
    let mut headers = Headers::new();
    headers
        .append(HeaderName::from_static("x-custom"), value("one"))
        .unwrap();
    headers
        .append(HeaderName::from_static("accept"), value("text/plain"))
        .unwrap();
    headers.set_guard(HeaderGuard::RequestNoCors).unwrap();
    assert!(!headers.has("x-custom"));
    assert!(headers.has("accept"));
}

#[test]
/// Verifies string construction validates names and normalizes values.
fn string_construction_validates_normalizes_and_trims() {
    let headers = Headers::try_from_strings([
        ("X-First", "  one\t"),
        ("x-first", "two"),
        ("Accept", "text/plain"),
    ])
    .unwrap();

    assert_eq!(headers.get("X-FIRST").as_deref(), Some("one, two"));
    assert_eq!(
        headers.iter().collect::<Vec<_>>(),
        [
            (HeaderName::from_static("x-first"), "one, two".to_owned()),
            (HeaderName::from_static("accept"), "text/plain".to_owned()),
        ]
    );
    assert!(matches!(
        Headers::try_from_strings([("bad name", "value")]),
        Err(RequestError::InvalidHeaderName(name)) if name == "bad name"
    ));
    assert!(matches!(
        Headers::try_from_strings([("x-test", "bad\r\nvalue")]),
        Err(RequestError::InvalidHeaderValue(name)) if name == "x-test"
    ));
}

#[test]
/// Verifies map construction preserves duplicate entries and the guard.
fn map_and_headers_construction_preserve_duplicates_and_guard() {
    let mut map = HeaderMap::new();
    map.append("x-test", value("one"));
    map.append("x-test", value("two"));
    let headers = Headers::try_from_header_map(&map).unwrap();
    assert_eq!(headers.get("x-test").as_deref(), Some("one, two"));

    let cloned = Headers::from_headers(&headers);
    assert_eq!(cloned, headers);
    assert_eq!(cloned.guard(), HeaderGuard::Request);
}

#[test]
/// Verifies public iteration exposes combined values and separate cookies.
fn public_iteration_combines_ordinary_values_but_not_set_cookie() {
    let mut headers = Headers::with_guard(HeaderGuard::None);
    headers.append_str("x-first", "one").unwrap();
    headers.append_str("set-cookie", "a=1").unwrap();
    headers.append_str("x-second", "middle").unwrap();
    headers.append_str("x-first", "two").unwrap();
    headers.append_str("set-cookie", "b=2").unwrap();

    assert_eq!(
        headers.iter().collect::<Vec<_>>(),
        [
            (HeaderName::from_static("x-first"), "one, two".to_owned()),
            (HeaderName::from_static("set-cookie"), "a=1".to_owned()),
            (HeaderName::from_static("x-second"), "middle".to_owned()),
            (HeaderName::from_static("set-cookie"), "b=2".to_owned()),
        ]
    );
    assert_eq!(headers.get_set_cookie(), ["a=1", "b=2"]);
    assert_eq!(headers.iter_raw().count(), 5);
}

#[test]
/// Verifies header guards can only transition toward stricter states.
fn guard_transitions_are_monotonic() {
    let allowed = [
        (HeaderGuard::None, HeaderGuard::Request),
        (HeaderGuard::None, HeaderGuard::RequestNoCors),
        (HeaderGuard::None, HeaderGuard::Response),
        (HeaderGuard::None, HeaderGuard::Immutable),
        (HeaderGuard::Request, HeaderGuard::RequestNoCors),
        (HeaderGuard::Request, HeaderGuard::Immutable),
        (HeaderGuard::RequestNoCors, HeaderGuard::Immutable),
        (HeaderGuard::Response, HeaderGuard::Immutable),
    ];
    for (from, to) in allowed {
        let mut headers = Headers::with_guard(from);
        headers.set_guard(to).unwrap();
        assert_eq!(headers.guard(), to);
    }

    let rejected = [
        (HeaderGuard::Request, HeaderGuard::None),
        (HeaderGuard::RequestNoCors, HeaderGuard::Request),
        (HeaderGuard::Response, HeaderGuard::Request),
        (HeaderGuard::Immutable, HeaderGuard::None),
        (HeaderGuard::Immutable, HeaderGuard::Request),
        (HeaderGuard::Immutable, HeaderGuard::Response),
    ];
    for (from, to) in rejected {
        let mut headers = Headers::with_guard(from);
        assert!(matches!(
            headers.set_guard(to),
            Err(RequestError::InvalidHeaderGuardTransition {
                from: actual_from,
                to: actual_to,
            }) if actual_from == from && actual_to == to
        ));
        assert_eq!(headers.guard(), from);
    }
}

#[test]
/// Verifies immutable guards reject typed, string, and internal mutations.
fn immutable_headers_reject_typed_string_and_internal_mutations() {
    let mut headers = Headers::with_guard(HeaderGuard::Immutable);
    assert!(matches!(
        headers.set(HeaderName::from_static("x-test"), value("one")),
        Err(RequestError::ImmutableHeaders)
    ));
    assert!(matches!(
        headers.append_str("x-test", "one"),
        Err(RequestError::ImmutableHeaders)
    ));
    assert!(matches!(
        headers.set_str("x-test", "one"),
        Err(RequestError::ImmutableHeaders)
    ));
    assert!(matches!(
        headers.insert_internal_header(HeaderName::from_static("x-test"), value("one")),
        Err(RequestError::ImmutableHeaders)
    ));
}

#[test]
/// Verifies CORS exposure handles wildcard, credential, duplicate, and malformed cases.
fn cors_exposure_handles_wildcards_credentials_duplicates_and_malformed_values() {
    let mut headers = HeaderMap::new();
    headers.append("access-control-expose-headers", value("X-Visible"));
    headers.append("access-control-expose-headers", value("x-second"));
    headers.insert("x-visible", value("yes"));
    headers.insert("x-second", value("also"));
    headers.insert("x-secret", value("hidden"));
    headers.insert("content-type", value("text/plain"));
    headers.insert("set-cookie", value("sid=secret"));

    let exposed = Headers::exposed_response(&headers, true, CredentialsMode::SameOrigin);
    assert!(exposed.has("x-visible"));
    assert!(exposed.has("x-second"));
    assert!(exposed.has("content-type"));
    assert!(!exposed.has("x-secret"));
    assert!(!exposed.has("set-cookie"));

    headers.insert("access-control-expose-headers", value("*"));
    let wildcard = Headers::exposed_response(&headers, true, CredentialsMode::Omit);
    assert!(wildcard.has("x-secret"));
    assert!(!wildcard.has("set-cookie"));
    let credentialed = Headers::exposed_response(&headers, true, CredentialsMode::Include);
    assert!(!credentialed.has("x-secret"));

    headers.insert(
        "access-control-expose-headers",
        value("x-visible, bad name"),
    );
    let malformed = Headers::exposed_response(&headers, true, CredentialsMode::Omit);
    assert!(!malformed.has("x-visible"));
    assert!(malformed.has("content-type"));
}

#[test]
/// Verifies basic response filtering hides all cookie response headers.
fn basic_response_filter_removes_all_cookie_response_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("set-cookie", value("a=1"));
    headers.insert("set-cookie2", value("b=2"));
    headers.insert("x-visible", value("yes"));
    let exposed = Headers::exposed_response(&headers, false, CredentialsMode::SameOrigin);
    assert!(!exposed.has("set-cookie"));
    assert!(!exposed.has("set-cookie2"));
    assert!(exposed.has("x-visible"));
}

#[test]
fn no_cors_safelist_checks_content_type_range_and_forbidden_headers() {
    let content_type = HeaderName::from_static("content-type");
    assert!(is_cors_safelisted_header(
        &content_type,
        &value("text/plain; charset=UTF-8")
    ));
    assert!(!is_cors_safelisted_header(
        &content_type,
        &value("application/json")
    ));

    let range = HeaderName::from_static("range");
    assert!(is_cors_safelisted_header(&range, &value("bytes=0-499")));
    assert!(is_cors_safelisted_header(&range, &value("bytes=500-")));
    assert!(!is_cors_safelisted_header(&range, &value("bytes=-500")));
    assert!(!is_cors_safelisted_header(&range, &value("bytes=0-1-2")));

    let mut request = Headers::new();
    assert!(matches!(
        request.append(HeaderName::from_static("host"), value("example.test")),
        Err(RequestError::ForbiddenHeader(name)) if name == "host"
    ));
    assert!(matches!(
        request.append(HeaderName::from_static("sec-fetch-site"), value("same-origin")),
        Err(RequestError::ForbiddenHeader(name)) if name == "sec-fetch-site"
    ));
}

#[test]
fn cors_unsafe_names_are_sorted_deduplicated_and_apply_the_aggregate_limit() {
    let mut headers = Headers::new();
    headers
        .append(
            HeaderName::from_static("x-zed"),
            HeaderValue::from_static("one"),
        )
        .unwrap();
    headers
        .append(
            HeaderName::from_static("x-alpha"),
            HeaderValue::from_static("two"),
        )
        .unwrap();
    headers
        .append(
            HeaderName::from_static("x-zed"),
            HeaderValue::from_static("three"),
        )
        .unwrap();
    assert_eq!(
        cors_unsafe_request_header_names(&headers)
            .iter()
            .map(HeaderName::as_str)
            .collect::<Vec<_>>(),
        ["x-alpha", "x-zed"]
    );

    let mut aggregate = Headers::new();
    for _ in 0..9 {
        aggregate
            .append(
                HeaderName::from_static("accept-language"),
                HeaderValue::from_bytes(&[b'a'; 128]).unwrap(),
            )
            .unwrap();
    }
    assert_eq!(
        cors_unsafe_request_header_names(&aggregate),
        [HeaderName::from_static("accept-language")]
    );
}
