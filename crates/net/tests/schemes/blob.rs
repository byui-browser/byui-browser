//! Tests for browser-owned `blob:` resolution.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use net::{
    BlobProvider, Config, Origin, ProviderFuture, Request, RequestController, RequestError,
    SchemeBody, SchemeResource,
};

use super::support::{
    ProviderErrorKind, RecordingServices, ResponseType, UnreachableProvider, runtime,
};

#[derive(Clone, Copy)]
enum Outcome {
    Bytes,
    Stream,
    Null,
    Revoked,
    Malformed,
}

struct Provider {
    calls: AtomicUsize,
    outcome: Outcome,
}

impl BlobProvider for Provider {
    fn resolve<'a>(&'a self, _request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(url.as_str(), "blob:https://creator.test/object-1");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcome;
        Box::pin(async move {
            match outcome {
                Outcome::Revoked => Err(RequestError::Provider {
                    provider: "blob",
                    kind: ProviderErrorKind::Revoked,
                }),
                Outcome::Malformed => Err(RequestError::Provider {
                    provider: "blob",
                    kind: ProviderErrorKind::MalformedMetadata,
                }),
                Outcome::Bytes => Ok(resource(SchemeBody::Bytes(b"blob-bytes".to_vec()))),
                Outcome::Stream => Ok(resource(SchemeBody::Stream(Box::pin(
                    futures_util::stream::iter([Ok(b"blob-".to_vec()), Ok(b"stream".to_vec())]),
                )))),
                Outcome::Null => Ok(resource(SchemeBody::Null)),
            }
        })
    }
}

fn resource(body: SchemeBody) -> SchemeResource {
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

fn controller(provider: Arc<Provider>) -> (RequestController, Arc<RecordingServices>) {
    let services = Arc::new(RecordingServices::default());
    let providers = net::SchemeProviders::new()
        .with_blob(provider)
        .with_file(Arc::new(UnreachableProvider))
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
fn blob_fetches_resolve_once_and_preserve_provider_metadata() {
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        outcome: Outcome::Bytes,
    });
    let (controller, services) = controller(provider.clone());
    let response = runtime()
        .block_on(controller.fetch(Request::get("blob:https://creator.test/object-1")))
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.response_type, ResponseType::Basic);
    assert_eq!(response.headers.get("x-blob").as_deref(), Some("yes"));
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"blob-bytes"
    );
    assert_eq!(
        services.response_origins.lock().unwrap().as_slice(),
        &[Some(Origin::parse("https://creator.test").unwrap())]
    );
    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
}

#[test]
fn blob_fetches_support_streams_and_explicit_null_bodies() {
    for (outcome, method, expected, is_null) in [
        (Outcome::Stream, "GET", b"blob-stream".as_slice(), false),
        (Outcome::Null, "HEAD", b"".as_slice(), true),
    ] {
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller(provider.clone());
        let response = runtime()
            .block_on(
                controller
                    .fetch(Request::new("blob:https://creator.test/object-1", method).unwrap()),
            )
            .unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.body.is_null(), is_null);
        assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), expected);
    }
}

#[test]
fn blob_provider_failures_are_typed_and_do_not_expose_a_response() {
    for (outcome, kind) in [
        (Outcome::Revoked, ProviderErrorKind::Revoked),
        (Outcome::Malformed, ProviderErrorKind::MalformedMetadata),
    ] {
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller(provider.clone());
        let error = runtime()
            .block_on(controller.fetch(Request::get("blob:https://creator.test/object-1")))
            .unwrap_err();
        assert!(
            matches!(error, RequestError::Provider { provider: "blob", kind: actual } if actual == kind)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}
