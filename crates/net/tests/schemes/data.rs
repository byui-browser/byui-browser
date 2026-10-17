//! Tests for engine-owned `data:` fetching.

use std::sync::Arc;

use net::{Config, Request, RequestController, RequestError, RequestMode};

use super::support::{
    FailingServices, FailurePoint, ResponseType, controller_with_services_and_providers, runtime,
};

#[test]
fn data_urls_decode_standard_payload_forms_and_metadata() {
    let controller = RequestController::new(Config::default()).unwrap();
    let cases = [
        (
            "data:,hello",
            "text/plain;charset=US-ASCII",
            b"hello".as_slice(),
        ),
        (
            "data:text/plain;charset=utf-8,hello%20world",
            "text/plain;charset=utf-8",
            b"hello world".as_slice(),
        ),
        (
            "data:application/octet-stream,%FF%00",
            "application/octet-stream",
            &[0xff, 0x00],
        ),
        (
            "data:text/plain;base64,SGVsbG8=",
            "text/plain",
            b"Hello".as_slice(),
        ),
        (
            "data:;base64,",
            "text/plain;charset=US-ASCII",
            b"".as_slice(),
        ),
    ];

    for (url, content_type, expected_body) in cases {
        let response = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap();
        assert_eq!(response.response_type, ResponseType::Basic);
        assert_eq!(response.status, 200);
        assert_eq!(response.url, url);
        assert_eq!(
            response.headers.get("content-type").as_deref(),
            Some(content_type)
        );
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            expected_body
        );
    }
}

#[test]
fn data_urls_reject_malformed_payload_and_media_type_inputs() {
    let controller = RequestController::new(Config::default()).unwrap();
    for url in [
        "data:text/plain",
        "data:,bad%",
        "data:text/plain;base64,not-base64!",
        "data:text/plain;base64;base64,QQ==",
        "data:textplain,payload",
        "data:text/plain;charset,payload",
    ] {
        let error = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap_err();
        assert!(
            matches!(error, RequestError::InvalidResponse(_)),
            "{url}: {error}"
        );
    }
}

#[test]
fn data_urls_keep_url_components_out_of_the_payload() {
    for (url, expected_body) in [
        ("data:,hello?query", b"hello".as_slice()),
        ("data:,hello#fragment", b"hello".as_slice()),
    ] {
        let controller = RequestController::new(Config::default()).unwrap();
        let response = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap();
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            expected_body
        );
    }
}

#[test]
fn data_urls_define_body_behavior_for_non_head_methods() {
    for method in ["POST", "OPTIONS", "PUT"] {
        let controller = RequestController::new(Config::default()).unwrap();
        let response = runtime()
            .block_on(controller.fetch(Request::new("data:text/plain,payload", method).unwrap()))
            .unwrap();
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            b"payload"
        );
    }
    let controller = RequestController::new(Config::default()).unwrap();
    let response = runtime()
        .block_on(controller.fetch(Request::new("data:text/plain,payload", "HEAD").unwrap()))
        .unwrap();
    assert!(response.body.is_null());
}

#[test]
fn data_urls_ignore_request_bodies_and_credentials_for_local_fetching() {
    let (controller, services) = controller_with_services_and_providers();
    let response = runtime()
        .block_on(controller.fetch(Request::get("data:text/plain,payload")))
        .unwrap();
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"payload"
    );
    assert_eq!(
        services
            .cookie_lookups
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        services
            .cookie_stores
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[test]
fn data_url_service_failures_do_not_expose_a_response() {
    for failure in [
        FailurePoint::Request,
        FailurePoint::Worker,
        FailurePoint::Response,
    ] {
        let services = Arc::new(FailingServices(failure));
        let controller = RequestController::with_services(Config::default(), services).unwrap();
        let error = runtime()
            .block_on(controller.fetch(Request::get("data:text/plain,private")))
            .unwrap_err();
        assert!(matches!(
            error,
            RequestError::InvalidResponse(_) | RequestError::UnsupportedFeature(_)
        ));
    }
}

#[test]
fn data_no_cors_responses_use_the_existing_opaque_filter() {
    let controller = RequestController::new(Config::default()).unwrap();
    let mut request = Request::get("data:text/plain,private");
    request.set_mode(RequestMode::NoCors).unwrap();
    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(response.response_type, ResponseType::Opaque);
    assert_eq!(response.status, 0);
    assert!(response.url.is_empty());
    assert!(response.body.is_null());
}

#[test]
fn data_fetches_use_response_services_but_bypass_cookies_cache_and_providers() {
    let (controller, services) = controller_with_services_and_providers();
    let response = runtime()
        .block_on(controller.fetch(Request::get("data:text/plain,local")))
        .unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"local");
    assert_eq!(
        services
            .response_checks
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        services
            .cookie_lookups
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        services
            .cache_partitions
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}
