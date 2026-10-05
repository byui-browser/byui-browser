//! Cache policy and cache lifecycle integration tests.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use common::{TestServer, collect_body, runtime};
use net::{CacheMode, Config, Request, RequestController};

#[test]
fn empty_stream_body_can_be_consumed_and_cached() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 204 No Content\r\nCache-Control: max-age=60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let (first, second) = runtime().block_on(async {
        let first = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let first_body = collect_body(first).await.unwrap();
        let second = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let second_from_cache = second.from_cache;
        let second_body = collect_body(second).await.unwrap();
        ((first_body, second_from_cache), second_body)
    });

    server.join();
    assert!(first.0.is_empty());
    assert!(first.1);
    assert!(second.is_empty());
}

#[test]
fn fully_consumed_stream_is_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(1, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let response = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        assert_eq!(collect_body(response).await.unwrap(), b"cached");
        let cached = controller.fetch(Request::get(server.url())).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.body, b"cached");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[test]
fn no_store_streams_are_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfresh".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.cache_mode = CacheMode::NoStore;
        let first = controller.fetch_stream(request.clone()).await.unwrap();
        let second = controller.fetch_stream(request).await.unwrap();
        assert!(!first.from_cache);
        assert!(!second.from_cache);
        assert_eq!(collect_body(first).await.unwrap(), b"fresh");
        assert_eq!(collect_body(second).await.unwrap(), b"fresh");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn reload_stream_bypasses_cache_and_caches_the_refresh() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        let body = if observed_hits.fetch_add(1, Ordering::SeqCst) == 0 {
            "first"
        } else {
            "second"
        };
        format!("HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_bytes()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let (first, refreshed, cached) = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = collect_body(controller.fetch_stream(request.clone()).await.unwrap())
            .await
            .unwrap();
        let mut reload = request;
        reload.cache_mode = CacheMode::Reload;
        let refreshed_response = controller.fetch_stream(reload).await.unwrap();
        let refreshed = (
            refreshed_response.from_cache,
            collect_body(refreshed_response).await.unwrap(),
        );
        let cached_response = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let cached = (
            cached_response.from_cache,
            collect_body(cached_response).await.unwrap(),
        );
        (first, refreshed, cached)
    });

    server.join();
    assert_eq!(first, b"first");
    assert!(!refreshed.0);
    assert_eq!(refreshed.1, b"second");
    assert!(cached.0);
    assert_eq!(cached.1, b"second");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn only_if_cached_returns_a_streaming_cache_hit() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nX-Cache-Test: yes\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let (from_cache, body, header) = runtime().block_on(async {
        let request = Request::get(server.url());
        collect_body(controller.fetch_stream(request.clone()).await.unwrap())
            .await
            .unwrap();
        let mut only_cached = request;
        only_cached.cache_mode = CacheMode::OnlyIfCached;
        let response = controller.fetch_stream(only_cached).await.unwrap();
        let from_cache = response.from_cache;
        let header = response.headers["x-cache-test"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = collect_body(response).await.unwrap();
        (from_cache, body, header)
    });

    server.join();
    assert!(from_cache);
    assert_eq!(body, b"cached");
    assert_eq!(header, "yes");
}

#[test]
fn no_store_requests_are_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfresh".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.cache_mode = CacheMode::NoStore;
        controller.fetch(request.clone()).await.unwrap();
        controller.fetch(request).await.unwrap();
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn clear_cache_forces_a_new_network_request() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let request = Request::get(server.url());
        assert!(!controller.fetch(request.clone()).await.unwrap().from_cache);
        assert!(controller.fetch(request.clone()).await.unwrap().from_cache);
        controller.clear_cache();
        assert!(!controller.fetch(request).await.unwrap().from_cache);
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn reload_bypasses_cached_response_and_caches_the_refresh() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        let body = if observed_hits.fetch_add(1, Ordering::SeqCst) == 0 {
            "first"
        } else {
            "second"
        };
        format!("HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_bytes()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let response = runtime().block_on(async {
        let request = Request::get(server.url());
        controller.fetch(request.clone()).await.unwrap();
        let mut reload = request;
        reload.cache_mode = CacheMode::Reload;
        (
            controller.fetch(reload).await.unwrap(),
            controller.fetch(Request::get(server.url())).await.unwrap(),
        )
    });

    server.join();
    assert_eq!(response.0.body, b"second");
    assert!(!response.0.from_cache);
    assert_eq!(response.1.body, b"second");
    assert!(response.1.from_cache);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn only_if_cached_returns_a_cached_response_without_network_io() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let cached = runtime().block_on(async {
        let request = Request::get(server.url());
        controller.fetch(request.clone()).await.unwrap();
        let mut only_cached = request;
        only_cached.cache_mode = CacheMode::OnlyIfCached;
        controller.fetch(only_cached).await.unwrap()
    });

    server.join();
    assert!(cached.from_cache);
    assert_eq!(cached.body, b"cached");
}

#[test]
fn cloned_controllers_share_the_response_cache() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\nshared".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();
    let clone = controller.clone();

    let (first, second) = runtime().block_on(async {
        let request = Request::get(server.url());
        (
            controller.fetch(request.clone()).await.unwrap(),
            clone.fetch(request).await.unwrap(),
        )
    });

    server.join();
    assert!(!first.from_cache);
    assert!(second.from_cache);
}
