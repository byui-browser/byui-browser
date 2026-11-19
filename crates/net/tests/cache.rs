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
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let (first, second) = runtime().block_on(async {
        let first = controller.fetch(Request::get(server.url())).await.unwrap();
        let first_body = collect_body(first).await.unwrap();
        let second = controller.fetch(Request::get(server.url())).await.unwrap();
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
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    runtime().block_on(async {
        let response = controller.fetch(Request::get(server.url())).await.unwrap();
        assert_eq!(response.body.length(), Some(6));
        assert_eq!(collect_body(response).await.unwrap(), b"cached");
        let cached = controller.fetch(Request::get(server.url())).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.body.length(), Some(6));
        assert_eq!(cached.body.bytes().await.unwrap(), b"cached");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[test]
fn no_cache_revalidates_with_etag_and_reuses_the_merged_representation() {
    let server = TestServer::start(2, |request| {
        if request.contains("if-none-match: \"version-1\"") {
            b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nETag: \"version-2\"\r\nX-Revalidated: yes\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nETag: \"version-1\"\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let (revalidated, cached) = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"cached");
        let mut no_cache = request;
        no_cache.set_cache_mode(CacheMode::NoCache);
        let revalidated = controller.fetch(no_cache).await.unwrap();
        let revalidated = (
            revalidated.from_cache,
            revalidated.headers.get("x-revalidated"),
            collect_body(revalidated).await.unwrap(),
        );
        let cached = controller.fetch(Request::get(server.url())).await.unwrap();
        let cached = (cached.from_cache, collect_body(cached).await.unwrap());
        (revalidated, cached)
    });

    server.join();
    assert!(revalidated.0);
    assert_eq!(revalidated.1.as_deref(), Some("yes"));
    assert_eq!(revalidated.2, b"cached");
    assert!(cached.0);
    assert_eq!(cached.1, b"cached");
}

#[test]
fn bodies_larger_than_the_capture_limit_are_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nlarge".to_vec()
    });
    let controller = RequestController::builder(Config {
        max_cache_body_size: 4,
        ..Config::default()
    })
    .build()
    .unwrap();

    runtime().block_on(async {
        for _ in 0..2 {
            let response = controller.fetch(Request::get(server.url())).await.unwrap();
            assert!(!response.from_cache);
            assert_eq!(response.body.bytes().await.unwrap(), b"large");
        }
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn no_store_responses_are_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfresh".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_cache_mode(CacheMode::NoStore);
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        let second = controller.fetch(request).await.unwrap();
        assert!(!first.from_cache);
        assert!(!second.from_cache);
        assert_eq!(collect_body(first).await.unwrap(), b"fresh");
        assert_eq!(collect_body(second).await.unwrap(), b"fresh");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn reload_bypasses_cache_and_caches_the_refresh_when_consumed() {
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
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let (first, refreshed, cached) = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = collect_body(
            controller
                .fetch(request.try_clone().unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        let mut reload = request;
        reload.set_cache_mode(CacheMode::Reload);
        let refreshed_response = controller.fetch(reload).await.unwrap();
        let refreshed = (
            refreshed_response.from_cache,
            collect_body(refreshed_response).await.unwrap(),
        );
        let cached_response = controller.fetch(Request::get(server.url())).await.unwrap();
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
fn unsafe_redirect_invalidates_its_same_origin_location_target() {
    let target_hits = Arc::new(AtomicUsize::new(0));
    let observed_target_hits = Arc::clone(&target_hits);
    let server = TestServer::start(3, move |request| {
        if request.starts_with("POST /mutate ") {
            b"HTTP/1.1 302 Found\r\nLocation: /target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else if request.starts_with("GET /target ") {
            let body = if observed_target_hits.fetch_add(1, Ordering::SeqCst) == 0 {
                "old"
            } else {
                "new"
            };
            format!("HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
        } else {
            panic!("unexpected request: {request}");
        }
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let target = format!("{}/target", server.url());
    let mutation = format!("{}/mutate", server.url());

    let final_response = runtime().block_on(async {
        let initial = controller.fetch(Request::get(&target)).await.unwrap();
        assert_eq!(collect_body(initial).await.unwrap(), b"old");

        let mutation = controller
            .fetch(Request::new(&mutation, "POST").unwrap())
            .await
            .unwrap();
        (mutation.from_cache, collect_body(mutation).await.unwrap())
    });

    server.join();
    assert!(!final_response.0);
    assert_eq!(final_response.1, b"new");
}

#[test]
fn only_if_cached_returns_a_cache_hit() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nX-Cache-Test: yes\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let (from_cache, body, header) = runtime().block_on(async {
        let request = Request::get(server.url());
        collect_body(
            controller
                .fetch(request.try_clone().unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        let mut only_cached = request;
        only_cached.set_cache_mode(CacheMode::OnlyIfCached);
        let response = controller.fetch(only_cached).await.unwrap();
        let from_cache = response.from_cache;
        let header = response.headers.get("x-cache-test").unwrap().to_owned();
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
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_cache_mode(CacheMode::NoStore);
        controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
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
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert!(!first.from_cache);
        collect_body(first).await.unwrap();
        assert!(
            controller
                .fetch(request.try_clone().unwrap())
                .await
                .unwrap()
                .from_cache
        );
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
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let response = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        collect_body(first).await.unwrap();
        let mut reload = request;
        reload.set_cache_mode(CacheMode::Reload);
        let refreshed = controller.fetch(reload).await.unwrap();
        let refreshed_from_cache = refreshed.from_cache;
        let refreshed_body = collect_body(refreshed).await.unwrap();
        let cached = controller.fetch(Request::get(server.url())).await.unwrap();
        (refreshed_from_cache, refreshed_body, cached)
    });

    server.join();
    assert_eq!(response.1, b"second");
    assert!(!response.0);
    assert_eq!(
        runtime().block_on(response.2.body.bytes()).unwrap(),
        b"second"
    );
    assert!(response.2.from_cache);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn only_if_cached_returns_a_cached_response_without_network_io() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let cached = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        collect_body(first).await.unwrap();
        let mut only_cached = request;
        only_cached.set_cache_mode(CacheMode::OnlyIfCached);
        controller.fetch(only_cached).await.unwrap()
    });

    server.join();
    assert!(cached.from_cache);
    assert_eq!(runtime().block_on(cached.body.bytes()).unwrap(), b"cached");
}

#[test]
fn cloned_controllers_share_the_response_cache() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\nshared".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let clone = controller.clone();

    let (first, second) = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        let first_from_cache = first.from_cache;
        collect_body(first).await.unwrap();
        let second = clone.fetch(request).await.unwrap();
        (first_from_cache, second)
    });

    server.join();
    assert!(!first);
    assert!(second.from_cache);
}
