//! Tests for browser-authorized `file:` access.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use net::{
    Config, FileProvider, Origin, ProviderFuture, Request, RequestController, RequestError,
    SchemeBody, SchemeResource,
};

use super::support::{ProviderErrorKind, RecordingServices, UnreachableProvider, runtime};

#[derive(Clone, Copy)]
enum Outcome {
    Bytes,
    Stream,
    Null,
    Missing,
    Denied,
    Io,
    Malformed,
}

struct Provider {
    calls: AtomicUsize,
    outcome: Outcome,
}

impl FileProvider for Provider {
    fn open<'a>(&'a self, _request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(url.as_str(), "file:///tmp/page.html");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcome;
        Box::pin(async move {
            match outcome {
                Outcome::Missing => Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::ResourceMissing,
                }),
                Outcome::Denied => Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::AccessDenied,
                }),
                Outcome::Io => Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::Io,
                }),
                Outcome::Malformed => Ok(resource(
                    net::Url::parse("https://wrong.test/page.html").unwrap(),
                    SchemeBody::Bytes(b"must-not-expose".to_vec()),
                )),
                Outcome::Bytes => Ok(resource(
                    net::Url::parse("file:///tmp/page.html").unwrap(),
                    SchemeBody::Bytes(b"file-bytes".to_vec()),
                )),
                Outcome::Stream => Ok(resource(
                    net::Url::parse("file:///tmp/page.html").unwrap(),
                    SchemeBody::Stream(Box::pin(futures_util::stream::iter([
                        Ok(b"file-".to_vec()),
                        Ok(b"stream".to_vec()),
                    ]))),
                )),
                Outcome::Null => Ok(resource(
                    net::Url::parse("file:///tmp/page.html").unwrap(),
                    SchemeBody::Null,
                )),
            }
        })
    }
}

fn resource(url: net::Url, body: SchemeBody) -> SchemeResource {
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

fn controller(provider: Arc<Provider>) -> (RequestController, Arc<RecordingServices>) {
    let services = Arc::new(RecordingServices::default());
    let providers = net::SchemeProviders::new()
        .with_blob(Arc::new(UnreachableProvider))
        .with_file(provider)
        .with_page(Arc::new(UnreachableProvider));
    (
        RequestController::with_services_and_providers(
            Config::default(),
            services.clone(),
            providers,
        )
        .unwrap(),
        services,
    )
}

#[test]
fn file_fetches_resolve_once_and_use_shared_response_finalization() {
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        outcome: Outcome::Bytes,
    });
    let (controller, services) = controller(provider.clone());
    let response = runtime()
        .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.url, "file:///tmp/page.html");
    assert_eq!(response.headers.get("x-file").as_deref(), Some("yes"));
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"file-bytes"
    );
    assert_eq!(
        services.response_origins.lock().unwrap().as_slice(),
        &[Some(Origin::opaque())]
    );
    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(services.cache_partitions.load(Ordering::SeqCst), 0);
}

#[test]
fn file_fetches_support_streams_and_explicit_null_bodies() {
    for (outcome, method, expected, is_null) in [
        (Outcome::Stream, "GET", b"file-stream".as_slice(), false),
        (Outcome::Null, "HEAD", b"".as_slice(), true),
    ] {
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller(provider.clone());
        let response = runtime()
            .block_on(controller.fetch(Request::new("file:///tmp/page.html", method).unwrap()))
            .unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.body.is_null(), is_null);
        assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), expected);
    }
}

#[test]
fn file_provider_failures_retain_stable_categories() {
    for (outcome, kind) in [
        (Outcome::Missing, ProviderErrorKind::ResourceMissing),
        (Outcome::Denied, ProviderErrorKind::AccessDenied),
        (Outcome::Io, ProviderErrorKind::Io),
    ] {
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller(provider.clone());
        let error = runtime()
            .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
            .unwrap_err();
        assert!(
            matches!(error, RequestError::Provider { provider: "file", kind: actual } if actual == kind)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn file_provider_metadata_is_validated_before_exposure() {
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        outcome: Outcome::Malformed,
    });
    let (controller, _) = controller(provider.clone());
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
fn file_requests_without_a_provider_fail_before_side_effects() {
    let controller = RequestController::new(Config::default()).unwrap();
    let error = runtime()
        .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
        .unwrap_err();
    assert!(matches!(error, RequestError::MissingProvider("file")));
}
