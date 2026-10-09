//! Redirect handling integration tests.

mod common;

use common::{TestServer, runtime};
use net::{Config, Request, RequestController, RequestError};

#[test]
fn follow_mode_does_not_follow_a_transport_redirect() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: /\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let error = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::RedirectFailure));
}

#[test]
fn streaming_fetch_rejects_redirect_before_exposing_headers() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: https://remote.test/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });

    let controller = RequestController::new(Config::default()).unwrap();
    let error = runtime()
        .block_on(controller.fetch_stream(Request::get(server.url())))
        .unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::RedirectFailure));
}
