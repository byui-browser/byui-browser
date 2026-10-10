//! Request construction and transport-facing integration tests.

mod common;

use common::{TestServer, runtime};
use net::{
    Config, FetchEnvironment, FetchServices, Request, RequestBody, RequestController, RequestError,
    ResponseInfo, ServiceWorkerDecision,
};

#[test]
fn fetch_propagates_a_body_transport_error() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nshort\r\n"
            .to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let error = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::Transport(_)));
}

#[test]
fn unsupported_schemes_are_rejected_without_network_io() {
    let controller = RequestController::new(Config::default()).unwrap();
    let error = runtime()
        .block_on(controller.fetch(Request::get("ftp://example.test/file")))
        .unwrap_err();

    assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
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
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_method("POST").unwrap();
        request
            .set_body(Some(RequestBody::bytes(b"payload".to_vec())))
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
            RequestController::new(Config::default())
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
    let controller =
        RequestController::with_services(Config::default(), std::sync::Arc::new(RejectCached))
            .unwrap();
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );
    runtime()
        .block_on(controller.fetch(request.clone()))
        .unwrap();
    let error = runtime().block_on(controller.fetch(request)).unwrap_err();
    server.join();
    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("cached response policy")
    ));
}
