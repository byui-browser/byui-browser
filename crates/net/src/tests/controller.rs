//! Integration tests for the public networking client API.

use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use crate::{AbortController, Body, CacheMode, Config, Request, RequestController, RequestError};
use futures_util::StreamExt;
use reqwest::{
    Method, StatusCode,
    header::{HeaderName, HeaderValue},
};

#[test]
fn get_request_has_expected_defaults() {
    // The convenience constructor should create the most common browser request
    // without requiring callers to initialize every field manually.
    let request = Request::get("https://example.com");

    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url, "https://example.com");
    assert!(request.headers.is_empty());
    assert!(request.body.is_none());
    assert_eq!(request.cache_mode, CacheMode::Default);
}

#[test]
fn custom_request_preserves_headers_and_body() {
    // Callers must also be able to construct non-GET requests with arbitrary
    // headers and binary request bodies.
    let mut headers = crate::Headers::new();
    headers
        .append(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        )
        .unwrap();
    let request = Request {
        method: Method::POST,
        url: "https://example.com/api".into(),
        headers,
        body: Some(Body::from_bytes(br#"{"ok":true}"#.to_vec())),
        cache_mode: CacheMode::NoStore,
        ..Request::get("https://example.com/api")
    };

    assert_eq!(request.method, Method::POST);
    assert_eq!(request.headers.iter().next().unwrap().1, "application/json");
    assert_eq!(
        request.body,
        Some(Body::from_bytes(br#"{"ok":true}"#.to_vec()))
    );
    assert_eq!(request.cache_mode, CacheMode::NoStore);
}

#[test]
fn default_config_sets_pool_and_identity() {
    // These defaults define the initial connection-management behavior used by
    // RequestController::new.
    let config = Config::default();

    assert_eq!(config.pool_max_idle_per_host, 8);
    assert!(config.pool_idle_timeout.is_some());
    assert_eq!(config.user_agent.unwrap(), "byui-browser/0.1");
}

#[test]
fn invalid_urls_return_typed_errors() {
    // URL validation happens before reqwest is asked to perform network I/O,
    // so malformed input should produce a stable crate-level error variant.
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let client = RequestController::new(Config::default()).expect("controller should initialize");

    let error = runtime
        .block_on(client.fetch(Request::get("not a URL")))
        .unwrap_err();

    assert!(matches!(error, RequestError::InvalidUrl(url) if url == "not a URL"));
}

#[test]
fn an_aborted_request_fails_before_network_work() {
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let client = RequestController::new(Config::default()).expect("controller should initialize");
    let controller = AbortController::new();
    let mut request = Request::get("http://127.0.0.1:1");
    request.signal = controller.signal();
    controller.abort();

    let error = runtime.block_on(client.fetch(request)).unwrap_err();

    assert!(matches!(error, RequestError::Aborted));
}

#[test]
fn only_if_cached_reports_a_cache_miss_without_network_access() {
    // OnlyIfCached is useful to callers that must avoid network access entirely;
    // an empty cache should fail immediately rather than attempting the URL.
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let client = RequestController::new(Config::default()).expect("controller should initialize");
    let mut request = Request::get("http://127.0.0.1:1");
    request.cache_mode = CacheMode::OnlyIfCached;

    let error = runtime.block_on(client.fetch(request)).unwrap_err();

    assert!(matches!(error, RequestError::NetworkError));
}

#[test]
fn can_fetch_resource_from_local_server() {
    // Keep the test deterministic and independent of DNS, internet access, and
    // external service availability.
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let address = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("test server address should be available")
    );
    let server = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test server should receive a request");
        let mut request = [0; 4096];
        let _ = stream
            .read(&mut request)
            .expect("test server should read the request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nlocal body",
            )
            .expect("test server should write the response");
    });

    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let client = RequestController::new(Config::default()).expect("controller should initialize");

    let response = runtime
        .block_on(client.fetch(Request::get(&address)))
        .expect("request should succeed");
    server.join().expect("test server should exit");

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        runtime.block_on(response.body.bytes()).unwrap(),
        b"local body"
    );
}

#[test]
fn fetch_returns_headers_before_body_and_forwards_body_bytes() {
    // The server pauses after sending headers. A successful fetch call
    // therefore proves callers can inspect response metadata without waiting
    // for the complete body.
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let address = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("test server address should be available")
    );
    let (headers_sent, headers_received) = mpsc::channel();
    let (send_body, receive_body) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test server should receive a request");
        let mut request = [0; 4096];
        let _ = stream
            .read(&mut request)
            .expect("test server should read the request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nX-Stream: yes\r\nConnection: close\r\n\r\n",
            )
            .expect("test server should write response headers");
        stream.flush().expect("test server should flush headers");
        headers_sent
            .send(())
            .expect("test should still be listening");
        receive_body.recv().expect("test should release the body");
        stream
            .write_all(b"hello world")
            .expect("test server should write response body");
    });

    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let client = RequestController::new(Config::default()).expect("controller should initialize");
    let mut response = runtime
        .block_on(client.fetch(Request::get(&address)))
        .expect("streaming request should succeed");

    headers_received
        .recv()
        .expect("server should have sent response headers");
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.headers.get("x-stream").unwrap(), "yes");
    assert!(!response.from_cache);

    send_body
        .send(())
        .expect("test server should receive release");
    let body = runtime.block_on(async {
        let mut body = Vec::new();
        while let Some(chunk) = response.body.next().await {
            body.extend_from_slice(&chunk.expect("body chunk should be valid"));
        }
        body
    });

    server.join().expect("test server should exit");
    assert_eq!(body, b"hello world");
}

#[test]
fn fetch_is_cached_only_after_body_completion() {
    // The first response is streamed and fully consumed. The following
    // streaming request should then use the completed response from cache.
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let address = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("test server address should be available")
    );
    let server = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test server should receive one request");
        let mut request = [0; 4096];
        let _ = stream
            .read(&mut request)
            .expect("test server should read the request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\nstreamed response",
            )
            .expect("test server should write response");
    });

    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let client = RequestController::new(Config::default()).expect("controller should initialize");
    let (first, second) = runtime.block_on(async {
        let mut first = client
            .fetch(Request::get(&address))
            .await
            .expect("first streaming request should succeed");
        let mut first_body = Vec::new();
        while let Some(chunk) = first.body.next().await {
            first_body.extend_from_slice(&chunk.expect("first body chunk should be valid"));
        }

        let second = client
            .fetch(Request::get(&address))
            .await
            .expect("cached streaming request should succeed");
        (first_body, second)
    });

    server.join().expect("test server should exit");
    assert_eq!(first, b"streamed response");
    assert!(second.from_cache);
    let second_body = runtime.block_on(async {
        let mut body = Vec::new();
        let mut second = second;
        while let Some(chunk) = second.body.next().await {
            body.extend_from_slice(&chunk.expect("cached body chunk should be valid"));
        }
        body
    });
    assert_eq!(second_body, b"streamed response");
}
