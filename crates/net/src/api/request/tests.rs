use http::HeaderValue;
use url::Url;

use super::{
    CredentialsMode, Origin, RedirectTaint, Referrer, ReferrerPolicy, Request, RequestMode,
};
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
fn redirect_taint_is_derived_from_the_complete_url_list() {
    let mut request = Request::get("https://app.example.test/start");
    request.request_origin = Some(Origin::parse("https://app.example.test").unwrap());
    request.url_list = vec![Url::parse("https://app.example.test/start").unwrap()];

    assert_eq!(request.redirect_taint(), RedirectTaint::SameOrigin);
    assert_eq!(
        request.serialized_origin().as_deref(),
        Some("https://app.example.test")
    );

    request
        .url_list
        .push(Url::parse("https://api.example.test/next").unwrap());
    assert_eq!(request.redirect_taint(), RedirectTaint::SameOrigin);

    request
        .url_list
        .push(Url::parse("https://cdn.example.test/asset").unwrap());
    assert_eq!(request.redirect_taint(), RedirectTaint::SameSite);
    assert_eq!(request.serialized_origin().as_deref(), Some("null"));

    request
        .url_list
        .push(Url::parse("https://remote.test/final").unwrap());
    assert_eq!(request.redirect_taint(), RedirectTaint::SameSite);

    request
        .url_list
        .push(Url::parse("https://other.test/final").unwrap());
    assert_eq!(request.redirect_taint(), RedirectTaint::CrossSite);
    assert_eq!(request.serialized_origin().as_deref(), Some("null"));
}

#[test]
fn redirect_taint_does_not_reset_when_a_chain_returns_to_its_origin() {
    let mut request = Request::get("https://app.example.test/start");
    request.request_origin = Some(Origin::parse("https://app.example.test").unwrap());
    request.url_list = [
        "https://app.example.test/start",
        "https://remote.test/middle",
        "https://app.example.test/final",
    ]
    .into_iter()
    .map(|url| Url::parse(url).unwrap())
    .collect();

    assert_eq!(request.redirect_taint(), RedirectTaint::CrossSite);
    assert_eq!(request.serialized_origin().as_deref(), Some("null"));
    assert!(!request.credentials_allowed(&Url::parse("https://app.example.test/final").unwrap()));
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
fn origin_header_generation_respects_target_mode_method_and_referrer_policy() {
    let same = Url::parse("https://client.test/resource").unwrap();
    let cross = Url::parse("https://remote.test/resource").unwrap();
    let downgrade = Url::parse("http://remote.test/resource").unwrap();
    let mut request = Request::get(same.as_str());
    request.context.environment.origin = Some(Origin::parse("https://client.test").unwrap());

    assert!(!request.fetch_headers(&same).unwrap().has("origin"));
    assert_eq!(
        request
            .fetch_headers(&cross)
            .unwrap()
            .get("origin")
            .unwrap(),
        "https://client.test"
    );

    request.set_mode(RequestMode::NoCors).unwrap();
    request.set_method("POST").unwrap();
    request.set_referrer(Referrer::Client, ReferrerPolicy::NoReferrer);
    assert_eq!(
        request
            .fetch_headers(&cross)
            .unwrap()
            .get("origin")
            .unwrap(),
        "null"
    );

    request.set_referrer(Referrer::Client, ReferrerPolicy::SameOrigin);
    assert_eq!(
        request.fetch_headers(&same).unwrap().get("origin").unwrap(),
        "https://client.test"
    );
    assert_eq!(
        request
            .fetch_headers(&cross)
            .unwrap()
            .get("origin")
            .unwrap(),
        "null"
    );

    request.set_referrer(Referrer::Client, ReferrerPolicy::StrictOrigin);
    assert_eq!(
        request
            .fetch_headers(&downgrade)
            .unwrap()
            .get("origin")
            .unwrap(),
        "null"
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
