//! End-to-end coverage for Fetch scheme dispatch and local resources.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use net::{
    BlobProvider, Body, Config, FetchServices, FileProvider, Origin, PageProvider,
    ProviderErrorKind, ProviderFuture, Request, RequestController, RequestError, RequestMode,
    ResponseInfo, ResponseType, SchemeBody, SchemeProviders, SchemeResource, ServiceWorkerDecision,
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
    response_origins: Mutex<Vec<Option<Origin>>>,
}

enum FailurePoint {
    Request,
    Response,
    Worker,
}

struct FailingServices(FailurePoint);

impl FetchServices for FailingServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        if matches!(self.0, FailurePoint::Request) {
            Err(RequestError::InvalidResponse("request denied".into()))
        } else {
            Ok(())
        }
    }

    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        if matches!(self.0, FailurePoint::Response) {
            Err(RequestError::InvalidResponse("response denied".into()))
        } else {
            Ok(())
        }
    }

    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        panic!("data: fetches must not request cookies")
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        panic!("data: fetches must not store cookies")
    }

    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        if matches!(self.0, FailurePoint::Worker) {
            Ok(ServiceWorkerDecision::Intercept)
        } else {
            Ok(ServiceWorkerDecision::Network)
        }
    }

    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        panic!("data: fetches must not request a cache partition")
    }
}

impl FetchServices for RecordingServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        self.request_checks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn check_response(&self, _request: &Request, info: &ResponseInfo) -> Result<(), RequestError> {
        self.response_checks.fetch_add(1, Ordering::SeqCst);
        self.response_origins
            .lock()
            .unwrap()
            .push(info.response_origin.clone());
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

#[derive(Clone, Copy)]
enum BlobOutcome {
    Bytes,
    Stream,
    Null,
    Revoked,
    Malformed,
}

struct RecordingBlobProvider {
    calls: AtomicUsize,
    outcome: BlobOutcome,
}

impl BlobProvider for RecordingBlobProvider {
    fn resolve<'a>(&'a self, _request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(url.as_str(), "blob:https://creator.test/object-1");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcome;
        Box::pin(async move {
            match outcome {
                BlobOutcome::Revoked => Err(RequestError::Provider {
                    provider: "blob",
                    kind: ProviderErrorKind::Revoked,
                }),
                BlobOutcome::Malformed => Err(RequestError::Provider {
                    provider: "blob",
                    kind: ProviderErrorKind::MalformedMetadata,
                }),
                BlobOutcome::Bytes => Ok(blob_resource(SchemeBody::Bytes(b"blob-bytes".to_vec()))),
                BlobOutcome::Stream => Ok(blob_resource(SchemeBody::Stream(Box::pin(
                    futures_util::stream::iter([Ok(b"blob-".to_vec()), Ok(b"stream".to_vec())]),
                )))),
                BlobOutcome::Null => Ok(blob_resource(SchemeBody::Null)),
            }
        })
    }
}

#[derive(Clone, Copy)]
enum FileOutcome {
    Bytes,
    Stream,
    Null,
    Missing,
    Denied,
    Io,
    Malformed,
}

struct RecordingFileProvider {
    calls: AtomicUsize,
    outcome: FileOutcome,
}

impl FileProvider for RecordingFileProvider {
    fn open<'a>(&'a self, _request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(url.as_str(), "file:///tmp/page.html");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcome;
        Box::pin(async move {
            match outcome {
                FileOutcome::Missing => Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::ResourceMissing,
                }),
                FileOutcome::Denied => Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::AccessDenied,
                }),
                FileOutcome::Io => Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::Io,
                }),
                FileOutcome::Malformed => Ok(file_resource(
                    net::Url::parse("https://wrong.test/page.html").unwrap(),
                    SchemeBody::Bytes(b"must-not-expose".to_vec()),
                )),
                FileOutcome::Bytes => Ok(file_resource(
                    net::Url::parse("file:///tmp/page.html").unwrap(),
                    SchemeBody::Bytes(b"file-bytes".to_vec()),
                )),
                FileOutcome::Stream => Ok(file_resource(
                    net::Url::parse("file:///tmp/page.html").unwrap(),
                    SchemeBody::Stream(Box::pin(futures_util::stream::iter([
                        Ok(b"file-".to_vec()),
                        Ok(b"stream".to_vec()),
                    ]))),
                )),
                FileOutcome::Null => Ok(file_resource(
                    net::Url::parse("file:///tmp/page.html").unwrap(),
                    SchemeBody::Null,
                )),
            }
        })
    }
}

fn file_resource(url: net::Url, body: SchemeBody) -> SchemeResource {
    SchemeResource {
        url,
        status: 200,
        status_text: Some("OK".into()),
        headers: vec![(
            net::HeaderName::from_static("x-file"),
            net::HeaderValue::from_static("yes"),
        )],
        media_type: Some(net::HeaderValue::from_static("text/plain")),
        origin: Origin::opaque(),
        body,
    }
}

fn controller_with_file_provider(
    provider: Arc<RecordingFileProvider>,
) -> (RequestController, Arc<RecordingServices>) {
    let services = Arc::new(RecordingServices::default());
    let providers = SchemeProviders::new()
        .with_blob(Arc::new(UnreachableProvider))
        .with_file(provider)
        .with_page(Arc::new(UnreachableProvider));
    let controller = RequestController::with_services_and_providers(
        Config::default(),
        services.clone(),
        providers,
    )
    .expect("controller should initialize");
    (controller, services)
}

fn blob_resource(body: SchemeBody) -> SchemeResource {
    SchemeResource {
        url: net::Url::parse("blob:https://creator.test/object-1").unwrap(),
        status: 200,
        status_text: Some("OK".into()),
        headers: vec![(
            net::HeaderName::from_static("x-blob"),
            net::HeaderValue::from_static("yes"),
        )],
        media_type: Some(net::HeaderValue::from_static("text/plain")),
        origin: Origin::parse("https://creator.test").unwrap(),
        body,
    }
}

fn controller_with_blob_provider(
    provider: Arc<RecordingBlobProvider>,
) -> (RequestController, Arc<RecordingServices>) {
    let services = Arc::new(RecordingServices::default());
    let providers = SchemeProviders::new()
        .with_blob(provider)
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
fn data_urls_keep_url_components_out_of_the_payload() {
    for (url, expected_body) in [
        ("data:,hello?query", b"hello".as_slice()),
        ("data:,hello#fragment", b"hello".as_slice()),
    ] {
        let controller = RequestController::new(Config::default()).unwrap();
        let response = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap();

        assert_eq!(response.url, url);
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            expected_body
        );
    }
}

#[test]
fn data_urls_define_body_behavior_for_non_head_methods() {
    for method in ["POST", "OPTIONS", "PUT"] {
        let request = Request::new("data:text/plain,payload", method).unwrap();
        let controller = RequestController::new(Config::default()).unwrap();
        let response = runtime().block_on(controller.fetch(request)).unwrap();

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
    for credentials in [net::CredentialsMode::Omit, net::CredentialsMode::SameOrigin] {
        let mut request = Request::new("data:text/plain,payload", "POST").unwrap();
        request.set_credentials_mode(credentials);
        request
            .set_body(Some(Body::from_bytes(b"request-body".to_vec())))
            .unwrap();
        let response = runtime().block_on(controller.fetch(request)).unwrap();
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            b"payload"
        );
    }

    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(services.cookie_stores.load(Ordering::SeqCst), 0);
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
    let error = runtime()
        .block_on(installed_controller.fetch(Request::get("byui://welcome")))
        .unwrap_err();
    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("byui scheme handler")
    ));
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

#[test]
fn blob_fetches_resolve_once_and_preserve_provider_metadata() {
    let provider = Arc::new(RecordingBlobProvider {
        calls: AtomicUsize::new(0),
        outcome: BlobOutcome::Bytes,
    });
    let (controller, services) = controller_with_blob_provider(provider.clone());

    let response = runtime()
        .block_on(controller.fetch(Request::get("blob:https://creator.test/object-1")))
        .unwrap();

    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.status, 200);
    assert_eq!(response.url, "blob:https://creator.test/object-1");
    assert_eq!(response.headers.get("x-blob").as_deref(), Some("yes"));
    assert_eq!(
        response.headers.get("content-type").as_deref(),
        Some("text/plain")
    );
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"blob-bytes"
    );
    assert_eq!(
        services.response_origins.lock().unwrap().as_slice(),
        &[Some(Origin::parse("https://creator.test").unwrap())]
    );
    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(services.cookie_stores.load(Ordering::SeqCst), 0);
    assert_eq!(services.cache_partitions.load(Ordering::SeqCst), 0);
}

#[test]
fn blob_fetches_support_streams_and_explicit_null_bodies() {
    for (outcome, expected_body, expected_null) in [
        (BlobOutcome::Stream, b"blob-stream".as_slice(), false),
        (BlobOutcome::Null, b"".as_slice(), true),
    ] {
        let provider = Arc::new(RecordingBlobProvider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller_with_blob_provider(provider.clone());
        let request = if expected_null {
            Request::new("blob:https://creator.test/object-1", "HEAD").unwrap()
        } else {
            Request::get("blob:https://creator.test/object-1")
        };
        let response = runtime().block_on(controller.fetch(request)).unwrap();

        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.body.is_null(), expected_null);
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            expected_body
        );
    }
}

#[test]
fn blob_provider_failures_are_typed_and_do_not_expose_a_response() {
    for (outcome, expected_kind) in [
        (BlobOutcome::Revoked, ProviderErrorKind::Revoked),
        (BlobOutcome::Malformed, ProviderErrorKind::MalformedMetadata),
    ] {
        let provider = Arc::new(RecordingBlobProvider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller_with_blob_provider(provider.clone());
        let error = runtime()
            .block_on(controller.fetch(Request::get("blob:https://creator.test/object-1")))
            .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Provider {
                provider: "blob",
                kind
            } if kind == expected_kind
        ));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn file_fetches_resolve_once_and_use_shared_response_finalization() {
    let provider = Arc::new(RecordingFileProvider {
        calls: AtomicUsize::new(0),
        outcome: FileOutcome::Bytes,
    });
    let (controller, services) = controller_with_file_provider(provider.clone());

    let response = runtime()
        .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
        .unwrap();

    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.status, 200);
    assert_eq!(response.url, "file:///tmp/page.html");
    assert_eq!(response.headers.get("x-file").as_deref(), Some("yes"));
    assert_eq!(
        response.headers.get("content-type").as_deref(),
        Some("text/plain")
    );
    assert_eq!(response.response_type, ResponseType::Basic);
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"file-bytes"
    );
    assert_eq!(
        services.response_origins.lock().unwrap().as_slice(),
        &[Some(Origin::opaque())]
    );
    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(services.cookie_stores.load(Ordering::SeqCst), 0);
    assert_eq!(services.cache_partitions.load(Ordering::SeqCst), 0);
}

#[test]
fn file_fetches_support_streams_and_explicit_null_bodies() {
    for (outcome, method, expected_body, expected_null) in [
        (FileOutcome::Stream, "GET", b"file-stream".as_slice(), false),
        (FileOutcome::Null, "HEAD", b"".as_slice(), true),
    ] {
        let provider = Arc::new(RecordingFileProvider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller_with_file_provider(provider.clone());
        let request = Request::new("file:///tmp/page.html", method).unwrap();
        let response = runtime().block_on(controller.fetch(request)).unwrap();

        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.body.is_null(), expected_null);
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            expected_body
        );
    }
}

#[test]
fn file_provider_failures_retain_stable_categories() {
    for (outcome, expected_kind) in [
        (FileOutcome::Missing, ProviderErrorKind::ResourceMissing),
        (FileOutcome::Denied, ProviderErrorKind::AccessDenied),
        (FileOutcome::Io, ProviderErrorKind::Io),
    ] {
        let provider = Arc::new(RecordingFileProvider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller_with_file_provider(provider.clone());
        let error = runtime()
            .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
            .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Provider {
                provider: "file",
                kind
            } if kind == expected_kind
        ));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn file_provider_metadata_is_validated_before_exposure() {
    let provider = Arc::new(RecordingFileProvider {
        calls: AtomicUsize::new(0),
        outcome: FileOutcome::Malformed,
    });
    let (controller, _) = controller_with_file_provider(provider.clone());
    let error = runtime()
        .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
        .unwrap_err();

    assert!(matches!(
        error,
        RequestError::Provider {
            provider: "file",
            kind: ProviderErrorKind::MalformedMetadata
        }
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn file_requests_without_a_provider_fail_before_provider_or_transport_work() {
    let controller = RequestController::new(Config::default()).unwrap();
    let error = runtime()
        .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
        .unwrap_err();

    assert!(matches!(error, RequestError::MissingProvider("file")));
}
