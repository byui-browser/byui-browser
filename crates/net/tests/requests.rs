//! Request construction and transport-facing integration tests.

mod common;

use common::{TestServer, runtime};
use net::{CacheMode, Config, Request, RequestController, RequestError};
use reqwest::{Method, StatusCode, header::HeaderValue};

#[test]
fn fetch_sends_headers_and_body_to_a_local_server() {
    let server = TestServer::start(1, |request| {
        assert!(request.contains("x-test: integration"));
        assert!(request.ends_with("request body"));
        b"HTTP/1.1 201 Created\r\nContent-Length: 7\r\nConnection: close\r\n\r\ncreated".to_vec()
    });

    let mut request = Request::get(server.url());
    request.method = Method::POST;
    request
        .headers
        .insert("x-test", HeaderValue::from_static("integration"));
    request.body = Some(b"request body".to_vec());
    request.cache_mode = CacheMode::NoStore;

    let controller = RequestController::new(Config::default()).unwrap();
    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.body, b"created");
    assert!(!response.from_cache);
}

#[test]
fn configured_user_agent_is_sent_to_the_server() {
    let server = TestServer::start(1, |request| {
        assert!(request.contains("user-agent: test-browser/1.0"));
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });

    let config = Config {
        user_agent: Some(HeaderValue::from_static("test-browser/1.0")),
        ..Config::default()
    };
    let controller = RequestController::new(config).unwrap();
    let response = runtime().block_on(controller.fetch(Request::get(server.url())));

    server.join();
    assert_eq!(response.unwrap().body, b"ok");
}

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
    assert!(matches!(error, RequestError::Decode(_)));
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
        request.method = Method::POST;
        request.body = Some(b"payload".to_vec());
        controller.fetch(request.clone()).await.unwrap();
        controller.fetch(request).await.unwrap();
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
