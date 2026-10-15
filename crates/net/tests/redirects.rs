//! Redirect handling integration tests.

mod common;

use common::{TestServer, collect_body, runtime};
use net::{Config, RedirectMode, Request, RequestController, RequestError};

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
    let controller = RequestController::new(Config::default()).unwrap();

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

    let controller = RequestController::new(Config::default()).unwrap();
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
    let controller = RequestController::new(Config::default()).unwrap();
    let mut request = Request::get(server.url());
    request.set_redirect_mode(RedirectMode::Manual);

    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.response_type, net::ResponseType::OpaqueRedirect);
    assert_eq!(response.status, 0);
    assert!(response.url.is_empty());
}
