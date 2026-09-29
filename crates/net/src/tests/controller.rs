//! Integration tests for the public networking client API.

use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use crate::{CacheMode, Config, Request, RequestController, RequestError, response};
use reqwest::{
    Method, StatusCode,
    header::{HeaderMap, HeaderValue},
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
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let request = Request {
        method: Method::POST,
        url: "https://example.com/api".into(),
        headers,
        body: Some(br#"{"ok":true}"#.to_vec()),
        cache_mode: CacheMode::NoStore,
        context: Default::default(),
    };

    assert_eq!(request.method, Method::POST);
    assert_eq!(request.headers["content-type"], "application/json");
    assert_eq!(request.body.as_deref(), Some(br#"{"ok":true}"#.as_slice()));
    assert_eq!(request.cache_mode, CacheMode::NoStore);
}

#[test]
fn default_config_sets_pool_and_redirect_limits() {
    // These defaults define the initial connection-management behavior used by
    // RequestController::new.
    let config = Config::default();

    assert_eq!(config.max_redirects, 10);
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
fn only_if_cached_reports_a_cache_miss_without_network_access() {
    // OnlyIfCached is useful to callers that must avoid network access entirely;
    // an empty cache should fail immediately rather than attempting the URL.
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let client = RequestController::new(Config::default()).expect("controller should initialize");
    let mut request = Request::get("http://127.0.0.1:1");
    request.cache_mode = CacheMode::OnlyIfCached;

    let error = runtime.block_on(client.fetch(request)).unwrap_err();

    assert!(matches!(error, RequestError::CacheMiss));
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
    assert_eq!(response.body, b"local body");
}
