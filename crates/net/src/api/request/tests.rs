use http::HeaderValue;
use url::Url;

use super::{CredentialsMode, Origin, Referrer, ReferrerPolicy, Request, RequestMode};
use crate::api::{body::Body, error::RequestError};

#[test]
fn credentials_mode_respects_tuple_and_opaque_origins() {
    let mut request = Request::get("https://client.test/resource");
    request.context.environment.origin = Some(Origin::parse("https://client.test").unwrap());

    request.context.credentials = CredentialsMode::Omit;
    assert!(!request.credentials_allowed(&Url::parse("https://client.test/other").unwrap()));

    request.context.credentials = CredentialsMode::SameOrigin;
    assert!(request.credentials_allowed(&Url::parse("https://client.test/other").unwrap()));
    assert!(!request.credentials_allowed(&Url::parse("https://client.test:8443/other").unwrap()));
    assert!(!request.credentials_allowed(&Url::parse("https://remote.test/other").unwrap()));

    request.context.environment.origin = Some(Origin::opaque());
    assert!(!request.credentials_allowed(&Url::parse("https://client.test/other").unwrap()));

    request.context.credentials = CredentialsMode::Include;
    assert!(request.credentials_allowed(&Url::parse("https://remote.test/other").unwrap()));
}

#[test]
fn referrer_policies_reduce_same_origin_cross_origin_and_downgrade_targets() {
    let source = Url::parse("https://client.test/path?query=one").unwrap();
    let same_origin = Url::parse("https://client.test/next").unwrap();
    let cross_origin = Url::parse("https://remote.test/next").unwrap();
    let downgrade = Url::parse("http://remote.test/next").unwrap();
    let mut request = Request::get("https://client.test/resource");
    request.context.environment.referrer = Some(source.clone());
    request.set_referrer(Referrer::Client, ReferrerPolicy::Empty);

    assert_eq!(
        request.referrer_value(&same_origin).as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        request.referrer_value(&cross_origin).as_deref(),
        Some("https://client.test")
    );
    assert_eq!(request.referrer_value(&downgrade), None);

    request.set_referrer(Referrer::Client, ReferrerPolicy::NoReferrer);
    assert_eq!(request.referrer_value(&same_origin), None);

    request.set_referrer(Referrer::Client, ReferrerPolicy::OriginWhenCrossOrigin);
    assert_eq!(
        request.referrer_value(&same_origin).as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        request.referrer_value(&cross_origin).as_deref(),
        Some("https://client.test")
    );

    request.set_referrer(Referrer::Client, ReferrerPolicy::SameOrigin);
    assert_eq!(
        request.referrer_value(&same_origin).as_deref(),
        Some(source.as_str())
    );
    assert_eq!(request.referrer_value(&cross_origin), None);

    request.set_referrer(Referrer::Client, ReferrerPolicy::UnsafeUrl);
    assert_eq!(
        request.referrer_value(&downgrade).as_deref(),
        Some(source.as_str())
    );
}

#[test]
fn request_mutators_enforce_method_body_and_no_cors_boundaries() {
    for method in ["CONNECT", "TRACE", "TRACK"] {
        assert!(matches!(
            Request::new("https://example.test/", method),
            Err(RequestError::ForbiddenMethod(actual)) if actual == method
        ));
    }

    let mut get = Request::get("https://example.test/");
    assert!(matches!(
        get.set_body(Some(Body::from_text("payload"))),
        Err(RequestError::BodyNotAllowed(method)) if method == "GET"
    ));

    let mut no_cors = Request::get("https://example.test/");
    no_cors.set_mode(RequestMode::NoCors).unwrap();
    assert!(matches!(
        no_cors.set_method("PUT"),
        Err(RequestError::NoCorsMethod(method)) if method == "PUT"
    ));
    no_cors.set_method("POST").unwrap();
    no_cors
        .set_body(Some(Body::from_bytes_with_content_type(
            b"{}".to_vec(),
            Some(HeaderValue::from_static("application/json")),
        )))
        .unwrap();
    assert!(matches!(
        no_cors.validate_body_state(64),
        Err(RequestError::ForbiddenHeader(name)) if name == "content-type"
    ));
}

#[test]
fn keepalive_body_limit_accepts_the_boundary_and_rejects_larger_bodies() {
    let mut at_limit = Request::new("https://example.test/", "POST").unwrap();
    at_limit.set_keepalive(true);
    at_limit
        .set_body(Some(Body::from_bytes(vec![0; 4])))
        .unwrap();
    assert!(at_limit.validate_body_state(4).is_ok());

    let mut over_limit = Request::new("https://example.test/", "POST").unwrap();
    over_limit.set_keepalive(true);
    over_limit
        .set_body(Some(Body::from_bytes(vec![0; 5])))
        .unwrap();
    assert!(matches!(
        over_limit.validate_body_state(4),
        Err(RequestError::KeepaliveBodyTooLarge { limit: 4 })
    ));
}

#[test]
fn streaming_request_can_transfer_once_without_an_implicit_clone() {
    let mut request = Request::new("https://example.test/", "POST").unwrap();
    request
        .set_body(Some(Body::stream(futures_util::stream::iter([Ok::<
            _,
            std::io::Error,
        >(
            b"payload".to_vec(),
        )]))))
        .unwrap();
    assert!(matches!(
        request.try_clone(),
        Err(RequestError::BodyNotCloneable)
    ));
    assert!(request.for_transport().unwrap().body.is_some());
    assert!(request.body.is_none());
}
