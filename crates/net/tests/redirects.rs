//! Redirect handling integration tests.

mod common;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use common::{TestServer, collect_body, runtime};
use net::{Body, Config, RedirectMode, Request, RequestController, RequestError};

#[test]
fn follow_mode_follows_redirect_and_exposes_final_response() {
    let server = TestServer::start(2, |request| {
        if request.starts_with("GET / HTTP") {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        } else {
            b"HTTP/1.1 302 Found\r\nLocation: /\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let response = runtime()
        .block_on(controller.fetch(Request::get(format!("{}/start", server.url()))))
        .unwrap();

    server.join();
    assert_eq!(response.status, 200);
    assert!(response.redirected);
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
}

#[test]
fn error_mode_rejects_redirect_before_exposing_headers() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: https://remote.test/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });

    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_redirect_mode(RedirectMode::Error);
    let error = runtime().block_on(controller.fetch(request)).unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::RedirectFailure));
}

#[test]
fn manual_mode_returns_an_opaque_redirect() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_redirect_mode(RedirectMode::Manual);

    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.response_type, net::ResponseType::OpaqueRedirect);
    assert_eq!(response.status, 0);
    assert!(response.url.is_empty());
}

#[test]
fn post_redirect_rewrites_to_get_and_removes_body_headers() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let server = TestServer::start(2, move |request| {
        observed.lock().unwrap().push(request.to_owned());
        if request.starts_with("POST /start HTTP") {
            b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::new(format!("{}/start", server.url()), "POST").unwrap();
    request.set_body(Some(Body::from_text("payload"))).unwrap();

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
    server.join();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("POST /start HTTP"));
    assert!(requests[0].contains("content-type: text/plain;charset=UTF-8"));
    assert!(requests[1].starts_with("GET /next HTTP"));
    assert!(!requests[1].contains("content-type:"));
    assert!(!requests[1].contains("content-length:"));
    assert!(!requests[1].ends_with("payload"));
}

#[test]
fn cross_origin_redirect_strips_sensitive_headers() {
    let destination_requests = Arc::new(Mutex::new(Vec::new()));
    let observed_destination = Arc::clone(&destination_requests);
    let destination = TestServer::start(1, move |request| {
        observed_destination
            .lock()
            .unwrap()
            .push(request.to_owned());
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let location = format!("{}/next", destination.url());
    let source = TestServer::start(1, move |_| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::get(source.url());
    request
        .set_header(
            net::HeaderName::from_static("authorization"),
            net::HeaderValue::from_static("Bearer secret"),
        )
        .unwrap();

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
    source.join();
    destination.join();

    let requests = destination_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].contains("authorization:"));
}

#[test]
fn non_replayable_body_cannot_follow_307_redirect() {
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&requests);
    let server = TestServer::start(1, move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::new(server.url(), "POST").unwrap();
    request
        .set_body(Some(Body::one_shot_bytes(b"payload".to_vec())))
        .unwrap();

    let error = runtime().block_on(controller.fetch(request)).unwrap_err();
    server.join();

    assert!(matches!(error, RequestError::RedirectFailure));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test]
fn redirect_limit_fails_before_a_twenty_second_follow_up() {
    let server = TestServer::start(21, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let error = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap_err();
    server.join();

    assert!(matches!(error, RequestError::RedirectFailure));
}
