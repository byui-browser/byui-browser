//! Request scheduling and concurrency integration tests.

mod common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use common::{TestServer, runtime};
use net::{Config, Request, RequestController};

#[test]
fn zero_in_flight_limit_still_allows_a_request() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller = RequestController::new(Config {
        max_in_flight: 0,
        ..Config::default()
    })
    .unwrap();

    let response = runtime().block_on(controller.fetch(Request::get(server.url())));

    server.join();
    assert_eq!(
        runtime().block_on(response.unwrap().body.bytes()).unwrap(),
        b"ok"
    );
}

#[test]
fn scheduler_limits_active_transport_requests() {
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let observed_active = Arc::clone(&active);
    let observed_maximum = Arc::clone(&maximum);
    let server = TestServer::start(2, move |_| {
        let now = observed_active.fetch_add(1, Ordering::SeqCst) + 1;
        observed_maximum.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(50));
        observed_active.fetch_sub(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });

    let controller = RequestController::new(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .unwrap();
    runtime().block_on(async {
        let first_controller = controller.clone();
        let second_controller = controller.clone();
        let first_url = server.url();
        let second_url = server.url();
        // Each task reads its own body. The scheduler permit stays with a
        // response body until it is consumed, so an unread response parked in
        // one join handle would starve the other request, whichever task the
        // runtime happens to admit first.
        let first = tokio::spawn(async move {
            let response = first_controller.fetch(Request::get(first_url)).await?;
            response.body.bytes().await
        });
        let second = tokio::spawn(async move {
            let response = second_controller.fetch(Request::get(second_url)).await?;
            response.body.bytes().await
        });
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
    });

    server.join();
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
}
