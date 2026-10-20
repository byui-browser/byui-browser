//! Request construction and transport-facing integration tests.

mod common;

use common::{TestServer, runtime};
use net::{
    Body, Config, FetchEnvironment, FetchServices, Request, RequestController, RequestError,
    ResponseInfo, SchemeProviders, ServiceWorkerDecision,
};
use std::sync::Arc;

struct NoopServices;

impl FetchServices for NoopServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        panic!("builder must not invoke services")
    }

    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        panic!("builder must not invoke services")
    }

    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        panic!("builder must not invoke services")
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        panic!("builder must not invoke services")
    }

    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        panic!("builder must not invoke services")
    }

    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        panic!("builder must not invoke services")
    }
}

#[test]
fn controller_builder_supports_all_capability_combinations() {
    RequestController::builder(Config::default())
        .build()
        .unwrap();
    RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(NoopServices))
        .build()
        .unwrap();
    RequestController::builder(Config::default())
        .with_scheme_providers(SchemeProviders::new())
        .build()
        .unwrap();
    RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(NoopServices))
        .with_scheme_providers(SchemeProviders::new())
        .build()
        .unwrap();
}

#[test]
fn body_consumption_propagates_a_post_header_transport_error() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nshort\r\n"
            .to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let error = runtime().block_on(async {
        let response = controller.fetch(Request::get(server.url())).await.unwrap();
        response.body.bytes().await.unwrap_err()
    });

    server.join();
    assert!(matches!(error, RequestError::Transport(_)));
}

#[test]
fn unsupported_schemes_are_rejected_without_network_io() {
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let error = runtime()
        .block_on(controller.fetch(Request::get("ftp://example.test/file")))
        .unwrap_err();

    assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
}

#[test]
fn data_urls_are_decoded_without_network_io() {
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let response = runtime()
        .block_on(controller.fetch(Request::get("data:text/plain,hello%20world")))
        .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.url, "data:text/plain,hello%20world");
    assert_eq!(response.headers.get("content-type").unwrap(), "text/plain");
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"hello world"
    );
}

#[test]
fn provider_backed_schemes_fail_with_typed_missing_capabilities() {
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

#[test]
fn non_cacheable_post_requests_are_sent_each_time() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_method("POST").unwrap();
        request
            .set_body(Some(Body::from_bytes(b"payload".to_vec())))
            .unwrap();
        controller.fetch(request.clone()).await.unwrap();
        controller.fetch(request).await.unwrap();
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn browser_context_without_services_fails_before_network_io() {
    let mut request = Request::get("https://remote.test/data");
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("https://client.test/").unwrap()).unwrap(),
    );
    let error = runtime()
        .block_on(
            RequestController::builder(Config::default())
                .build()
                .unwrap()
                .fetch(request),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("browser services")
    ));
}

#[test]
fn cached_browser_response_is_rechecked_before_exposure() {
    struct RejectCached;
    impl FetchServices for RejectCached {
        fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
            Ok(())
        }
        fn check_response(
            &self,
            _request: &Request,
            info: &ResponseInfo,
        ) -> Result<(), RequestError> {
            if info.from_cache {
                Err(RequestError::UnsupportedFeature("cached response policy"))
            } else {
                Ok(())
            }
        }
        fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
            Ok(None)
        }
        fn store_set_cookie(
            &self,
            _request: &Request,
            _values: &[net::HeaderValue],
        ) -> Result<(), RequestError> {
            Ok(())
        }
        fn service_worker(
            &self,
            _request: &Request,
        ) -> Result<ServiceWorkerDecision, RequestError> {
            Ok(ServiceWorkerDecision::Network)
        }
        fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
            Ok("cache-check".into())
        }
    }

    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(std::sync::Arc::new(RejectCached))
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );
    runtime().block_on(async {
        let response = controller.fetch(request.clone()).await.unwrap();
        response.body.bytes().await.unwrap();
    });
    let error = runtime().block_on(controller.fetch(request)).unwrap_err();
    server.join();
    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("cached response policy")
    ));
}
