//! Tests for scheme-dispatch boundaries affecting HTTP and unsupported URLs.

use net::{Config, Request, RequestController, RequestError};

use super::support::{controller_with_services_and_providers, runtime};

#[test]
fn unsupported_schemes_fail_before_services_or_providers() {
    let (controller, services) = controller_with_services_and_providers();
    let error = runtime()
        .block_on(controller.fetch(Request::get("ftp://example.test/resource")))
        .unwrap_err();
    assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
    assert_eq!(
        services
            .request_checks
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        services
            .response_checks
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

#[test]
fn provider_capabilities_are_reported_before_handler_side_effects() {
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    for (url, provider) in [
        ("blob:https://example.test/id", "blob"),
        ("file:///tmp/page.html", "file"),
        ("byui://welcome", "byui page"),
    ] {
        let error = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap_err();
        assert!(matches!(error, RequestError::MissingProvider(name) if name == provider));
    }
}
