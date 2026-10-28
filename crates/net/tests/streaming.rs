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
use net::{CacheMode, Config, Request, RequestController, RequestError};

#[test]
fn fetch_yields_body_chunks_before_response_completion() {
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

    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let (declared_length, first_chunk) = runtime().block_on(async {
        let mut response = controller.fetch(Request::get(url)).await.unwrap();
        let length = response.body.length();
        let chunk = response.body.next().await.unwrap().unwrap().to_vec();
        (length, chunk)
    });

    server.join().unwrap();
    assert_eq!(declared_length, Some(11));
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
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let (partial, retry) = runtime().block_on(async {
        let mut response = controller.fetch(Request::get(server.url())).await.unwrap();
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
    assert_eq!(runtime().block_on(retry.1.body.bytes()).unwrap(), b"valid");
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
    let controller = RequestController::builder(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .build()
    .unwrap();

    runtime().block_on(async {
        let mut response = controller.fetch(Request::get(server.url())).await.unwrap();
        assert_eq!(
            response.body.next().await.unwrap().unwrap().as_ref(),
            b"body"
        );
        drop(response);
        let retry = controller.fetch(Request::get(server.url())).await.unwrap();
        assert!(!retry.from_cache);
        assert_eq!(retry.body.bytes().await.unwrap(), b"body");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn cloned_response_holds_one_permit_until_both_branches_are_dropped() {
    let server = TestServer::start(2, |_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody".to_vec()
    });
    let controller = RequestController::builder(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .build()
    .unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_cache_mode(CacheMode::NoStore);
        let mut first = controller.fetch(request.clone()).await.unwrap();
        let clone = first.try_clone().unwrap();
        drop(first);

        let second = controller.fetch(request);
        tokio::pin!(second);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut second)
                .await
                .is_err()
        );

        drop(clone);
        let response = tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .expect("dropping both branches should release the permit")
            .unwrap();
        assert_eq!(response.body.bytes().await.unwrap(), b"body");
    });

    server.join();
}
