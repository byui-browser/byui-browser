//! Tests for the first-pass `about:` allowlist.

use std::sync::atomic::Ordering;

use net::{Request, RequestError, ResponseType};

use super::support::{controller_with_services_and_providers, runtime};

#[test]
fn about_blank_returns_an_empty_html_response() {
    let (controller, services) = controller_with_services_and_providers();
    let response = runtime()
        .block_on(controller.fetch(Request::get("about:blank")))
        .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.status_text, "OK");
    assert_eq!(response.url, "about:blank");
    assert_eq!(response.response_type, ResponseType::Basic);
    assert_eq!(
        response.headers.get("content-type").as_deref(),
        Some("text/html;charset=utf-8")
    );
    assert_eq!(response.headers.get("content-length").as_deref(), Some("0"));
    assert!(!response.body.is_null());
    assert!(
        runtime()
            .block_on(response.body.bytes())
            .unwrap()
            .is_empty()
    );
    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(services.cookie_stores.load(Ordering::SeqCst), 0);
    assert_eq!(services.cache_partitions.load(Ordering::SeqCst), 0);
}

#[test]
fn about_blank_head_preserves_metadata_and_has_a_null_body() {
    let (controller, _) = controller_with_services_and_providers();
    let response = runtime()
        .block_on(controller.fetch(Request::new("about:blank", "HEAD").unwrap()))
        .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.url, "about:blank");
    assert_eq!(
        response.headers.get("content-type").as_deref(),
        Some("text/html;charset=utf-8")
    );
    assert_eq!(response.headers.get("content-length").as_deref(), Some("0"));
    assert!(response.body.is_null());
    assert!(
        runtime()
            .block_on(response.body.bytes())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn about_allowlist_rejects_everything_except_blank() {
    let (controller, _) = controller_with_services_and_providers();
    for url in [
        "about:srcdoc",
        "about:blank?query",
        "about:blank#fragment",
        "about://blank",
        "about:unknown",
    ] {
        let error = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap_err();
        assert!(
            matches!(error, RequestError::UnsupportedFeature("about URL")),
            "{url}: {error}"
        );
    }
}

#[test]
fn about_blank_rejects_methods_other_than_get_and_head() {
    let (controller, _) = controller_with_services_and_providers();
    let error = runtime()
        .block_on(controller.fetch(Request::new("about:blank", "POST").unwrap()))
        .unwrap_err();

    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("about scheme method")
    ));
}
