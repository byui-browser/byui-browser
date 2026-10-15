//! Tests for browser-owned `byui:` page routing.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use net::{
    Config, Origin, PageProvider, ProviderFuture, Request, RequestController, RequestError,
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

impl PageProvider for Provider {
    fn resolve<'a>(&'a self, _request: &'a Request, url: &'a net::Url) -> ProviderFuture<'a> {
        assert_eq!(url.as_str(), "byui://welcome/page?lang=en");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcome;
        Box::pin(async move {
            match outcome {
                Outcome::Missing => Err(RequestError::Provider {
                    provider: "byui page",
                    kind: ProviderErrorKind::ResourceMissing,
                }),
                Outcome::Denied => Err(RequestError::Provider {
                    provider: "byui page",
                    kind: ProviderErrorKind::AccessDenied,
                }),
                Outcome::Io => Err(RequestError::Provider {
                    provider: "byui page",
                    kind: ProviderErrorKind::Io,
                }),
                Outcome::Malformed => Ok(resource(
                    net::Url::parse("file:///tmp/not-a-page").unwrap(),
                    SchemeBody::Bytes(b"must-not-expose".to_vec()),
                )),
                Outcome::Bytes => Ok(resource(
                    net::Url::parse("byui://welcome/page?lang=en").unwrap(),
                    SchemeBody::Bytes(b"<h1>Welcome</h1>".to_vec()),
                )),
                Outcome::Stream => Ok(resource(
                    net::Url::parse("byui://welcome/page?lang=en").unwrap(),
                    SchemeBody::Stream(Box::pin(futures_util::stream::iter([
                        Ok(b"<h1>Wel".to_vec()),
                        Ok(b"come</h1>".to_vec()),
                    ]))),
                )),
                Outcome::Null => Ok(resource(
                    net::Url::parse("byui://welcome/page?lang=en").unwrap(),
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
            net::HeaderName::from_static("x-page"),
            net::HeaderValue::from_static("welcome"),
        )],
        media_type: Some(net::HeaderValue::from_static("text/html")),
        origin: Origin::opaque(),
        body,
    }
}

fn controller(provider: Arc<Provider>) -> (RequestController, Arc<RecordingServices>) {
    let services = Arc::new(RecordingServices::default());
    let providers = net::SchemeProviders::new()
        .with_blob(Arc::new(UnreachableProvider))
        .with_file(Arc::new(UnreachableProvider))
        .with_page(provider);
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
fn byui_pages_resolve_once_and_preserve_page_metadata() {
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        outcome: Outcome::Bytes,
    });
    let (controller, services) = controller(provider.clone());
    let response = runtime()
        .block_on(controller.fetch(Request::get("byui://welcome/page?lang=en")))
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.url, "byui://welcome/page?lang=en");
    assert_eq!(response.headers.get("x-page").as_deref(), Some("welcome"));
    assert_eq!(
        response.headers.get("content-type").as_deref(),
        Some("text/html")
    );
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"<h1>Welcome</h1>"
    );
    assert_eq!(
        services.response_origins.lock().unwrap().as_slice(),
        &[Some(Origin::opaque())]
    );
    assert_eq!(services.cookie_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(services.cache_partitions.load(Ordering::SeqCst), 0);
}

#[test]
fn byui_pages_support_streams_and_explicit_null_bodies() {
    for (outcome, method, expected, is_null) in [
        (
            Outcome::Stream,
            "GET",
            b"<h1>Welcome</h1>".as_slice(),
            false,
        ),
        (Outcome::Null, "HEAD", b"".as_slice(), true),
    ] {
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let (controller, _) = controller(provider.clone());
        let response = runtime()
            .block_on(
                controller.fetch(Request::new("byui://welcome/page?lang=en", method).unwrap()),
            )
            .unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.body.is_null(), is_null);
        assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), expected);
    }
}

#[test]
fn byui_page_provider_failures_retain_stable_categories() {
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
            .block_on(controller.fetch(Request::get("byui://welcome/page?lang=en")))
            .unwrap_err();
        assert!(
            matches!(error, RequestError::Provider { provider: "byui page", kind: actual } if actual == kind)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn byui_page_provider_metadata_is_validated_before_exposure() {
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        outcome: Outcome::Malformed,
    });
    let (controller, _) = controller(provider.clone());
    let error = runtime()
        .block_on(controller.fetch(Request::get("byui://welcome/page?lang=en")))
        .unwrap_err();
    assert!(matches!(
        error,
        RequestError::Provider {
            provider: "byui",
            kind: ProviderErrorKind::MalformedMetadata
        }
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn byui_requests_without_a_page_provider_fail_before_side_effects() {
    let controller = RequestController::new(Config::default()).unwrap();
    let error = runtime()
        .block_on(controller.fetch(Request::get("byui://welcome/page?lang=en")))
        .unwrap_err();
    assert!(matches!(error, RequestError::MissingProvider("byui page")));
}
