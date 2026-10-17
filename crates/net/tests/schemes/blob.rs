//! Tests for browser-owned `blob:` resolution.

use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use futures_util::Stream;
use net::{
    AbortController, BlobProvider, Config, Origin, ProviderFuture, Request, RequestController,
    RequestError, SchemeBody, SchemeResource,
};
use tokio::sync::Notify;

use super::support::{
    ProviderErrorKind, RecordingServices, ResponseType, UnreachableProvider, runtime,
};

struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct PendingBodyStream {
    _drops: DropCounter,
    polled: Arc<Notify>,
    first_chunk: Option<Vec<u8>>,
}

impl Stream for PendingBodyStream {
    type Item = Result<Vec<u8>, RequestError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(chunk) = self.first_chunk.take() {
            return Poll::Ready(Some(Ok(chunk)));
        }
        self.polled.notify_one();
        Poll::Pending
    }
}

struct PendingProvider {
    calls: AtomicUsize,
    started: Arc<Notify>,
    drops: Arc<AtomicUsize>,
}

impl BlobProvider for PendingProvider {
    fn resolve<'a>(&'a self, request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(request.current_url(), Some(url));
        assert_eq!(request.url_list(), std::slice::from_ref(url));
        assert!(!request.signal().is_aborted());
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        let drop_counter = DropCounter(self.drops.clone());
        Box::pin(async move {
            let _drop_counter = drop_counter;
            std::future::pending().await
        })
    }
}

struct StreamingProvider {
    drops: Arc<AtomicUsize>,
    polled: Arc<Notify>,
}

impl BlobProvider for StreamingProvider {
    fn resolve<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        let stream = PendingBodyStream {
            _drops: DropCounter(self.drops.clone()),
            polled: self.polled.clone(),
            first_chunk: Some(b"partial".to_vec()),
        };
        Box::pin(async move { Ok(resource(SchemeBody::Stream(Box::pin(stream)))) })
    }
}

struct FailingBodyStream {
    _drops: DropCounter,
}

impl Stream for FailingBodyStream {
    type Item = Result<Vec<u8>, RequestError>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(Some(Err(RequestError::Provider {
            provider: "blob",
            kind: ProviderErrorKind::Io,
        })))
    }
}

struct FailingStreamProvider {
    drops: Arc<AtomicUsize>,
}

impl BlobProvider for FailingStreamProvider {
    fn resolve<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        let stream = FailingBodyStream {
            _drops: DropCounter(self.drops.clone()),
        };
        Box::pin(async move { Ok(resource(SchemeBody::Stream(Box::pin(stream)))) })
    }
}

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
    fn resolve<'a>(&'a self, request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(url.as_str(), "blob:https://creator.test/object-1");
        assert_eq!(request.current_url(), Some(url));
        assert_eq!(request.url_list(), std::slice::from_ref(url));
        assert!(!request.signal().is_aborted());
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

fn controller(provider: Arc<dyn BlobProvider>) -> (RequestController, Arc<RecordingServices>) {
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
        (Outcome::Null, "GET", b"".as_slice(), true),
        (Outcome::Bytes, "HEAD", b"".as_slice(), true),
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
fn blob_rejects_non_read_methods_before_provider_lookup() {
    for method in ["POST", "PUT", "DELETE", "OPTIONS"] {
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            outcome: Outcome::Bytes,
        });
        let (controller, _) = controller(provider.clone());

        let error = runtime()
            .block_on(
                controller
                    .fetch(Request::new("blob:https://creator.test/object-1", method).unwrap()),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            RequestError::UnsupportedFeature("blob scheme method")
        ));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0, "{method}");
    }
}

#[test]
fn aborting_pending_blob_resolution_drops_the_provider_future() {
    runtime().block_on(async {
        let started = Arc::new(Notify::new());
        let drops = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(PendingProvider {
            calls: AtomicUsize::new(0),
            started: started.clone(),
            drops: drops.clone(),
        });
        let (controller, _) = controller(provider.clone());
        let abort = AbortController::new();
        let mut request = Request::get("blob:https://creator.test/object-1");
        request.set_signal(abort.signal());

        let task = tokio::spawn(async move { controller.fetch(request).await });
        started.notified().await;
        abort.abort();

        let error = task.await.unwrap().unwrap_err();
        assert!(matches!(error, RequestError::Aborted));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn an_already_aborted_blob_request_never_calls_the_provider() {
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        outcome: Outcome::Bytes,
    });
    let (controller, _) = controller(provider.clone());
    let abort = AbortController::new();
    abort.abort();
    let mut request = Request::get("blob:https://creator.test/object-1");
    request.set_signal(abort.signal());

    let error = runtime().block_on(controller.fetch(request)).unwrap_err();
    assert!(matches!(error, RequestError::Aborted));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn aborting_a_blob_stream_ends_the_body_and_releases_the_stream() {
    runtime().block_on(async {
        let drops = Arc::new(AtomicUsize::new(0));
        let polled = Arc::new(Notify::new());
        let provider = Arc::new(StreamingProvider {
            drops: drops.clone(),
            polled: polled.clone(),
        });
        let (controller, _) = controller(provider);
        let abort = AbortController::new();
        let mut request = Request::get("blob:https://creator.test/object-1");
        request.set_signal(abort.signal());
        let response = controller.fetch(request).await.unwrap();

        let task = tokio::spawn(async move { response.body.bytes().await });
        polled.notified().await;
        abort.abort();

        let error = task.await.unwrap().unwrap_err();
        assert!(matches!(error, RequestError::Aborted));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn dropping_an_unread_blob_body_releases_the_provider_stream() {
    let drops = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(StreamingProvider {
        drops: drops.clone(),
        polled: Arc::new(Notify::new()),
    });
    let (controller, _) = controller(provider);
    let response = runtime()
        .block_on(controller.fetch(Request::get("blob:https://creator.test/object-1")))
        .unwrap();

    drop(response);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_a_partially_read_blob_body_releases_the_provider_stream() {
    use futures_util::StreamExt;

    let drops = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(StreamingProvider {
        drops: drops.clone(),
        polled: Arc::new(Notify::new()),
    });
    let (controller, _) = controller(provider);

    runtime().block_on(async {
        let response = controller
            .fetch(Request::get("blob:https://creator.test/object-1"))
            .await
            .unwrap();
        let mut body = response.body;
        assert_eq!(body.next().await.unwrap().unwrap(), b"partial".as_slice());
        drop(body);
    });

    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failed_blob_stream_is_released_before_the_body_is_dropped() {
    use futures_util::StreamExt;

    let drops = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(FailingStreamProvider {
        drops: drops.clone(),
    });
    let (controller, _) = controller(provider);

    runtime().block_on(async {
        let response = controller
            .fetch(Request::get("blob:https://creator.test/object-1"))
            .await
            .unwrap();
        let mut body = response.body;
        let error = body.next().await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            RequestError::Provider {
                provider: "blob",
                kind: ProviderErrorKind::Io
            }
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        drop(body);
    });

    assert_eq!(drops.load(Ordering::SeqCst), 1);
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
