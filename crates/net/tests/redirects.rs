//! Redirect handling integration tests.

mod common;

use std::{io::Write, net::TcpListener, thread};

use common::{TestServer, collect_body, read_request, runtime};
use net::{Config, Request, RequestController, RequestError};

#[test]
fn redirect_limit_is_enforced() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: /\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });
    let controller = RequestController::new(Config {
        max_redirects: 0,
        ..Config::default()
    })
    .unwrap();

    let error = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::Transport(_)));
}

#[test]
fn streamed_redirect_exposes_the_final_url_and_response() {
    let target_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_url = format!("http://{}", target_listener.local_addr().unwrap());
    let target_server = thread::spawn(move || {
        let (mut stream, _) = target_listener.accept().unwrap();
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfinal")
            .unwrap();
    });

    let redirect_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let redirect_url = format!("http://{}", redirect_listener.local_addr().unwrap());
    let redirect_target = target_url.clone();
    let redirect_server = thread::spawn(move || {
        let (mut stream, _) = redirect_listener.accept().unwrap();
        read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {redirect_target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let controller = RequestController::new(Config::default()).unwrap();
    let response = runtime().block_on(async {
        controller
            .fetch_stream(Request::get(redirect_url))
            .await
            .unwrap()
    });
    let final_url = response.url.clone();
    let body = runtime().block_on(collect_body(response)).unwrap();

    redirect_server.join().unwrap();
    target_server.join().unwrap();
    assert_eq!(final_url, format!("{target_url}/"));
    assert_eq!(body, b"final");
}
