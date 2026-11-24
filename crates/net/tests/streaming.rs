//! Streaming response and body-lifecycle integration tests.

mod common;

use std::{
    io::Write,
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use common::{TestServer, read_request, runtime};
use futures_util::StreamExt;
use net::{
    AbortController, CacheMode, Config, NetworkEvent, NetworkObserver, NetworkPhase, Request,
    RequestController, RequestError,
};

const ASYNC_TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct QueueObserver {
    phases: Mutex<Vec<(u64, NetworkPhase)>>,
    notified: tokio::sync::Notify,
}

impl QueueObserver {
    fn saw(&self, request_id: u64, phase: NetworkPhase) -> bool {
        self.phases.lock().unwrap().contains(&(request_id, phase))
    }

    async fn wait_for(&self, request_id: u64, phase: NetworkPhase) {
        loop {
            let notified = self.notified.notified();
            if self.saw(request_id, phase) {
                return;
            }
            notified.await;
        }
    }
}

impl NetworkObserver for QueueObserver {
    fn observe(&self, event: NetworkEvent) {
        self.phases
            .lock()
            .unwrap()
            .push((event.request_id, event.phase));
        self.notified.notify_waiters();
    }
}

#[test]
fn fetch_yields_body_chunks_before_response_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello")
            .unwrap();
        stream.flush().unwrap();
        release_rx
            .recv_timeout(ASYNC_TEST_TIMEOUT)
            .expect("test should release the remaining response bytes");
        let _ = stream.write_all(b" world");
    });

    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let (declared_length, first_chunk) = runtime().block_on(async {
        let mut response = controller.fetch(Request::get(url)).await.unwrap();
        let length = response.body.length();
        let chunk = response.body.next().await.unwrap().unwrap().to_vec();
        release_tx.send(()).unwrap();
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
    let controller = RequestController::builder(Config {
        max_in_flight: 1,
        ..Config::default()
    })
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
        let retry = tokio::time::timeout(
            ASYNC_TEST_TIMEOUT,
            controller.fetch(Request::get(server.url())),
        )
        .await
        .expect("errored stream should release its scheduler permit")
        .unwrap();
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
fn failed_cloned_response_releases_permit_and_is_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        if observed_hits.fetch_add(1, Ordering::SeqCst) == 0 {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 10\r\nConnection: close\r\n\r\nbroken".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nvalid".to_vec()
        }
    });
    let controller = RequestController::builder(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .build()
    .unwrap();

    runtime().block_on(async {
        let mut first = controller.fetch(Request::get(server.url())).await.unwrap();
        let clone = first.try_clone().unwrap();
        let (original, cloned) = tokio::join!(first.body.bytes(), clone.body.bytes());
        assert!(matches!(original, Err(RequestError::Transport(_))));
        assert!(matches!(cloned, Err(RequestError::Transport(_))));

        let retry = tokio::time::timeout(
            ASYNC_TEST_TIMEOUT,
            controller.fetch(Request::get(server.url())),
        )
        .await
        .expect("failed tee should release its scheduler permit")
        .unwrap();
        assert!(!retry.from_cache);
        assert_eq!(retry.body.bytes().await.unwrap(), b"valid");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn dropping_a_stream_releases_its_scheduler_permit() {
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
        let mut retry_request = Request::get(server.url());
        retry_request.set_cache_mode(CacheMode::Reload);
        let retry = tokio::time::timeout(ASYNC_TEST_TIMEOUT, controller.fetch(retry_request))
            .await
            .expect("dropped stream should release its scheduler permit")
            .unwrap();
        assert!(!retry.from_cache);
        assert_eq!(retry.body.bytes().await.unwrap(), b"body");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn aborting_after_headers_releases_the_permit_and_does_not_cache() {
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
        let abort = AbortController::new();
        let mut request = Request::get(server.url());
        request.set_signal(abort.signal());
        let response = controller.fetch(request).await.unwrap();
        abort.abort();
        assert!(matches!(
            response.body.bytes().await,
            Err(RequestError::Aborted)
        ));

        let retry = tokio::time::timeout(
            ASYNC_TEST_TIMEOUT,
            controller.fetch(Request::get(server.url())),
        )
        .await
        .expect("aborted stream should release its scheduler permit")
        .unwrap();
        assert!(!retry.from_cache);
        assert_eq!(retry.body.bytes().await.unwrap(), b"body");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn aborting_cloned_response_releases_permit_and_prevents_cache_capture() {
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
        let abort = AbortController::new();
        let mut request = Request::get(server.url());
        request.set_signal(abort.signal());
        let mut response = controller.fetch(request).await.unwrap();
        let clone = response.try_clone().unwrap();
        abort.abort();
        let (original, cloned) = tokio::join!(response.body.bytes(), clone.body.bytes());
        assert!(matches!(original, Err(RequestError::Aborted)));
        assert!(matches!(cloned, Err(RequestError::Aborted)));

        let retry = tokio::time::timeout(
            ASYNC_TEST_TIMEOUT,
            controller.fetch(Request::get(server.url())),
        )
        .await
        .expect("aborted tee should release its scheduler permit")
        .unwrap();
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
    let observer = Arc::new(QueueObserver::default());
    let controller = RequestController::builder(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .with_network_observer(observer.clone())
    .build()
    .unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_cache_mode(CacheMode::NoStore);
        let mut first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        let clone = first.try_clone().unwrap();
        drop(first);

        let mut second_request = Request::get(server.url());
        second_request.set_cache_mode(CacheMode::NoStore);
        let second_id = second_request.id();
        let second_controller = controller.clone();
        let second = tokio::spawn(async move { second_controller.fetch(second_request).await });
        observer
            .wait_for(second_id, NetworkPhase::QueueStarted)
            .await;
        tokio::task::yield_now().await;
        assert!(!observer.saw(second_id, NetworkPhase::QueueAdmitted));

        drop(clone);
        let response = tokio::time::timeout(ASYNC_TEST_TIMEOUT, second)
            .await
            .expect("dropping both branches should release the permit")
            .unwrap()
            .unwrap();
        assert_eq!(response.body.bytes().await.unwrap(), b"body");
    });

    server.join();
}
