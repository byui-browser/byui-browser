//! Tests for browser-authorized `file:` access.

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
    AbortController, Config, FileProvider, Origin, ProviderFuture, Request, RequestController,
    RequestError, SchemeBody, SchemeResource,
};
use tokio::sync::Notify;

use super::support::{ProviderErrorKind, RecordingServices, UnreachableProvider, runtime};

#[derive(Clone, Copy)]
enum Outcome {
    Bytes,
    Stream,
    Null,
    Missing,
    Unsupported,
    Denied,
    Io,
    Malformed,
}

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

impl FileProvider for PendingProvider {
    fn open<'a>(&'a self, request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(request.current_url(), Some(url));
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

impl FileProvider for StreamingProvider {
    fn open<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        let stream = PendingBodyStream {
            _drops: DropCounter(self.drops.clone()),
            polled: self.polled.clone(),
            first_chunk: Some(b"partial".to_vec()),
        };
        Box::pin(async move {
            Ok(resource(
                net::Url::parse("file:///tmp/page.html").unwrap(),
                SchemeBody::Stream(Box::pin(stream)),
            ))
        })
    }
}

struct Provider {
    calls: AtomicUsize,
    outcome: Outcome,
}

impl FileProvider for Provider {
    fn open<'a>(&'a self, request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(url.as_str(), "file:///tmp/page.html");
        assert_eq!(request.current_url(), Some(url));
        assert_eq!(request.url_list(), std::slice::from_ref(url));
        assert!(!request.signal().is_aborted());
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcome;
        Box::pin(async move {
            match outcome {
                Outcome::Missing => Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::ResourceMissing,
                }),
                Outcome::Unsupported => Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::UnsupportedResource,
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
        headers: vec![
            (
                net::HeaderName::from_static("x-file"),
                net::HeaderValue::from_static("yes"),
            ),
            (
                net::HeaderName::from_static("set-cookie"),
                net::HeaderValue::from_static("session=must-not-leak"),
            ),
        ],
        media_type: Some(net::HeaderValue::from_static("text/plain")),
        origin: Origin::opaque(),
        body,
    }
}

fn controller(provider: Arc<dyn FileProvider>) -> (RequestController, Arc<RecordingServices>) {
    let services = Arc::new(RecordingServices::default());
    let providers = net::SchemeProviders::new()
        .with_blob(Arc::new(UnreachableProvider))
        .with_file(provider)
        .with_page(Arc::new(UnreachableProvider));
    (
        RequestController::builder(Config::default())
            .with_fetch_services(services.clone())
            .with_scheme_providers(providers)
            .build()
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
    assert!(!response.headers.has("set-cookie"));
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
    assert_eq!(services.worker_checks.load(Ordering::SeqCst), 0);
}

#[test]
fn repeated_file_fetches_resolve_again_and_do_not_use_the_http_cache() {
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        outcome: Outcome::Bytes,
    });
    let (controller, services) = controller(provider.clone());

    for _ in 0..2 {
        let response = runtime()
            .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
            .unwrap();
        assert!(!response.from_cache);
        assert_eq!(
            runtime().block_on(response.body.bytes()).unwrap(),
            b"file-bytes"
        );
    }

    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(services.cache_partitions.load(Ordering::SeqCst), 0);
}

#[test]
fn file_fetches_support_streams_and_explicit_null_bodies() {
    for (outcome, method, expected, is_null) in [
        (Outcome::Stream, "GET", b"file-stream".as_slice(), false),
        (Outcome::Null, "GET", b"".as_slice(), true),
        (Outcome::Bytes, "HEAD", b"".as_slice(), true),
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
fn file_rejects_non_read_methods_before_provider_lookup() {
    for method in ["POST", "PUT", "DELETE", "OPTIONS"] {
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            outcome: Outcome::Bytes,
        });
        let (controller, _) = controller(provider.clone());

        let error = runtime()
            .block_on(controller.fetch(Request::new("file:///tmp/page.html", method).unwrap()))
            .unwrap_err();

        assert!(matches!(
            error,
            RequestError::UnsupportedFeature("file scheme method")
        ));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0, "{method}");
    }
}

#[test]
fn aborting_pending_file_resolution_drops_the_provider_future() {
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
        let mut request = Request::get("file:///tmp/page.html");
        request.set_signal(abort.signal());

        let task = tokio::spawn(async move { controller.fetch(request).await });
        started.notified().await;
        abort.abort();

        assert!(matches!(task.await.unwrap(), Err(RequestError::Aborted)));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn aborting_a_file_stream_releases_the_provider_stream() {
    runtime().block_on(async {
        let drops = Arc::new(AtomicUsize::new(0));
        let polled = Arc::new(Notify::new());
        let provider = Arc::new(StreamingProvider {
            drops: drops.clone(),
            polled: polled.clone(),
        });
        let (controller, _) = controller(provider);
        let abort = AbortController::new();
        let mut request = Request::get("file:///tmp/page.html");
        request.set_signal(abort.signal());
        let response = controller.fetch(request).await.unwrap();

        let task = tokio::spawn(async move { response.body.bytes().await });
        polled.notified().await;
        abort.abort();

        assert!(matches!(task.await.unwrap(), Err(RequestError::Aborted)));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn dropping_unread_or_partially_read_file_bodies_releases_the_provider_stream() {
    use futures_util::StreamExt;

    for read_first_chunk in [false, true] {
        let drops = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(StreamingProvider {
            drops: drops.clone(),
            polled: Arc::new(Notify::new()),
        });
        let (controller, _) = controller(provider);

        runtime().block_on(async {
            let response = controller
                .fetch(Request::get("file:///tmp/page.html"))
                .await
                .unwrap();
            let mut body = response.body;
            if read_first_chunk {
                assert_eq!(body.next().await.unwrap().unwrap(), b"partial".as_slice());
            }
            drop(body);
        });

        assert_eq!(
            drops.load(Ordering::SeqCst),
            1,
            "read_first_chunk={read_first_chunk}"
        );
    }
}

#[test]
fn file_provider_failures_retain_stable_categories() {
    for (outcome, kind) in [
        (Outcome::Missing, ProviderErrorKind::ResourceMissing),
        (Outcome::Unsupported, ProviderErrorKind::UnsupportedResource),
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
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let error = runtime()
        .block_on(controller.fetch(Request::get("file:///tmp/page.html")))
        .unwrap_err();
    assert!(matches!(error, RequestError::MissingProvider("file")));
}
