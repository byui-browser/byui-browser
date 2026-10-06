//! Streaming response and body-lifecycle integration tests.

mod common;

use std::{
    io::Write,
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use common::{TestServer, read_request, runtime};
use futures_util::StreamExt;
use net::{Config, Request, RequestController, RequestError};

#[test]
fn fetch_stream_yields_body_chunks_before_response_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello")
            .unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(100));
        let _ = stream.write_all(b" world");
    });

    let controller = RequestController::new(Config::default()).unwrap();
    let first_chunk = runtime().block_on(async {
        let mut response = controller.fetch_stream(Request::get(url)).await.unwrap();
        response.body.next().await.unwrap().unwrap().to_vec()
    });

    server.join().unwrap();
    assert_eq!(first_chunk, b"hello");
}

#[test]
fn incomplete_stream_forwards_error_and_is_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        if observed_hits.fetch_add(1, Ordering::SeqCst) == 0 {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 11\r\nConnection: close\r\n\r\nshort".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nvalid".to_vec()
        }
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let (partial, retry) = runtime().block_on(async {
        let mut response = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let mut partial = Vec::new();
        let mut error = None;
        while let Some(chunk) = response.body.next().await {
            match chunk {
                Ok(chunk) => partial.extend_from_slice(&chunk),
                Err(error_value) => {
                    error = Some(error_value);
                    break;
                }
            }
        }
        let retry = controller.fetch(Request::get(server.url())).await.unwrap();
        (partial, (error, retry))
    });

    server.join();
    assert_eq!(partial, b"short");
    assert!(matches!(retry.0, Some(RequestError::Transport(_))));
    assert!(!retry.1.from_cache);
    assert_eq!(retry.1.body, b"valid");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn dropping_a_stream_releases_its_scheduler_permit_and_does_not_cache() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody".to_vec()
    });
    let controller = RequestController::new(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .unwrap();

    runtime().block_on(async {
        let mut response = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        assert_eq!(
            response.body.next().await.unwrap().unwrap().as_ref(),
            b"body"
        );
        drop(response);
        let retry = controller.fetch(Request::get(server.url())).await.unwrap();
        assert!(!retry.from_cache);
        assert_eq!(retry.body, b"body");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
