//! End-to-end coverage for Fetch scheme dispatch and local resources.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use net::{
    BlobProvider, Config, FetchServices, FileProvider, PageProvider, ProviderFuture, Request,
    RequestController, RequestError, RequestMode, ResponseInfo, ResponseType, SchemeProviders,
    ServiceWorkerDecision,
};

mod common;

use common::runtime;

#[derive(Default)]
struct RecordingServices {
    request_checks: AtomicUsize,
    response_checks: AtomicUsize,
    cookie_lookups: AtomicUsize,
    cookie_stores: AtomicUsize,
    cache_partitions: AtomicUsize,
    worker_checks: AtomicUsize,
}

impl FetchServices for RecordingServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        self.request_checks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        self.response_checks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        self.cookie_lookups.fetch_add(1, Ordering::SeqCst);
        Ok(Some("session=unexpected".into()))
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        self.cookie_stores.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        self.worker_checks.fetch_add(1, Ordering::SeqCst);
        Ok(ServiceWorkerDecision::Network)
    }

    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        self.cache_partitions.fetch_add(1, Ordering::SeqCst);
        Ok("local-scheme-test".into())
    }
}

/// A provider whose methods panic so a test fails if dispatch reaches it.
struct UnreachableProvider;

impl BlobProvider for UnreachableProvider {
    fn resolve<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        panic!("data and unsupported schemes must not call the blob provider")
    }
}

impl FileProvider for UnreachableProvider {
    fn open<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        panic!("data and unsupported schemes must not call the file provider")
    }
}

impl PageProvider for UnreachableProvider {
    fn resolve<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        panic!("data and unsupported schemes must not call the page provider")
    }
}

fn controller_with_services_and_providers() -> (RequestController, Arc<RecordingServices>) {
    let services = Arc::new(RecordingServices::default());
    let providers = SchemeProviders::new()
        .with_blob(Arc::new(UnreachableProvider))
        .with_file(Arc::new(UnreachableProvider))
        .with_page(Arc::new(UnreachableProvider));
    let controller = RequestController::with_services_and_providers(
        Config::default(),
        services.clone(),
        providers,
    )
    .expect("controller should initialize");
    (controller, services)
}

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
        (
            "data:text/plain,one,two,three",
            "text/plain",
            b"one,two,three".as_slice(),
        ),
    ];

    for (url, content_type, expected_body) in cases {
        let response = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap_or_else(|error| panic!("{url} should decode: {error}"));
        assert_eq!(response.response_type, ResponseType::Basic, "{url}");
        assert_eq!(response.status, 200, "{url}");
        assert_eq!(response.status_text, "OK", "{url}");
        assert_eq!(response.url, url, "{url}");
        assert_eq!(
            response.headers.get("content-type").as_deref(),
            Some(content_type)
        );
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            expected_body,
            "{url}"
        );
    }
}

#[test]
fn data_urls_reject_malformed_payload_and_media_type_inputs() {
    let controller = RequestController::new(Config::default()).unwrap();
    for url in [
        "data:text/plain",
        "data:,bad%",
        "data:,bad%0",
        "data:text/plain;base64,not-base64!",
        "data:text/plain;base64;base64,QQ==",
        "data:textplain,payload",
        "data:text/plain;charset,payload",
        "data:;charset=utf-8,payload",
    ] {
        let error = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .expect_err("malformed data URL must not expose a response");
        assert!(
            matches!(error, RequestError::InvalidResponse(_)),
            "{url}: {error}"
        );
    }
}

#[test]
fn data_head_preserves_metadata_and_has_a_null_body() {
    let controller = RequestController::new(Config::default()).unwrap();
    let request = Request::new("data:text/plain,body-that-must-not-be-exposed", "HEAD").unwrap();
    let response = runtime().block_on(controller.fetch(request)).unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("content-type").as_deref(),
        Some("text/plain")
    );
    assert!(response.body.is_null());
    assert!(
        runtime()
            .block_on(response.body.bytes())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn data_no_cors_responses_use_the_existing_opaque_filter() {
    let controller = RequestController::new(Config::default()).unwrap();
    let mut request = Request::get("data:text/plain,private");
    request.set_mode(RequestMode::NoCors).unwrap();
    let response = runtime().block_on(controller.fetch(request)).unwrap();

    assert_eq!(response.response_type, ResponseType::Opaque);
    assert_eq!(response.status, 0);
    assert!(response.status_text.is_empty());
    assert!(response.url.is_empty());
    assert!(response.headers.is_empty());
    assert!(response.body.is_null());
    assert!(
        runtime()
            .block_on(response.body.bytes())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn data_fetches_use_response_services_but_bypass_cookies_cache_and_providers() {
    let (controller, services) = controller_with_services_and_providers();

    for _ in 0..2 {
        let response = runtime()
            .block_on(controller.fetch(Request::get("data:text/plain,local")))
            .unwrap();
        assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"local");
    }

    assert_eq!(services.request_checks.load(Ordering::SeqCst), 2);
    assert_eq!(services.worker_checks.load(Ordering::SeqCst), 2);
    assert_eq!(services.response_checks.load(Ordering::SeqCst), 2);
    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(services.cookie_stores.load(Ordering::SeqCst), 0);
    assert_eq!(services.cache_partitions.load(Ordering::SeqCst), 0);
}

#[test]
fn unsupported_schemes_fail_before_services_or_providers() {
    let (controller, services) = controller_with_services_and_providers();
    let error = runtime()
        .block_on(controller.fetch(Request::get("ftp://example.test/resource")))
        .unwrap_err();

    assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
    assert_eq!(services.request_checks.load(Ordering::SeqCst), 0);
    assert_eq!(services.worker_checks.load(Ordering::SeqCst), 0);
    assert_eq!(services.response_checks.load(Ordering::SeqCst), 0);
    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(services.cookie_stores.load(Ordering::SeqCst), 0);
    assert_eq!(services.cache_partitions.load(Ordering::SeqCst), 0);
}

#[test]
fn provider_backed_schemes_distinguish_absent_and_installed_capabilities() {
    let absent_controller = RequestController::new(Config::default()).unwrap();
    for (url, provider) in [
        ("blob:https://example.test/id", "blob"),
        ("file:///tmp/page.html", "file"),
        ("byui://welcome", "byui page"),
    ] {
        let error = runtime()
            .block_on(absent_controller.fetch(Request::get(url)))
            .unwrap_err();
        assert!(matches!(error, RequestError::MissingProvider(name) if name == provider));
    }

    let (installed_controller, _) = controller_with_services_and_providers();
    for (url, handler) in [
        ("blob:https://example.test/id", "blob scheme handler"),
        ("file:///tmp/page.html", "file scheme handler"),
        ("byui://welcome", "byui scheme handler"),
    ] {
        let error = runtime()
            .block_on(installed_controller.fetch(Request::get(url)))
            .unwrap_err();
        assert!(matches!(error, RequestError::UnsupportedFeature(feature) if feature == handler));
    }
}

#[test]
fn about_urls_are_rejected_without_calling_scheme_providers() {
    let (controller, _) = controller_with_services_and_providers();
    for url in ["about:blank", "about:srcdoc", "about:unknown"] {
        let error = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap_err();
        assert!(matches!(
            error,
            RequestError::UnsupportedFeature("about scheme handler")
        ));
    }
}
