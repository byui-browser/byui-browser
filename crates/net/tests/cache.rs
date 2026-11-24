//! Cache policy and cache lifecycle integration tests.

mod common;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use common::{TestServer, collect_body, runtime};
use http::{HeaderName, HeaderValue};
use net::{
    CacheMode, CachePartitionService, Config, CookieService, FetchEnvironment, Request,
    RequestController, RequestError, RequestMode, RequestPolicyService, ResponseInfo,
    ResponsePolicyService, ServiceWorkerDecision, ServiceWorkerService,
};

struct SameOriginCacheServices;

struct StandaloneCachePartition;

impl CachePartitionService for StandaloneCachePartition {
    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("standalone-cache-test".into())
    }
}

fn cache_controller() -> RequestController {
    RequestController::builder(Config::default())
        .with_cache_partition_service(Arc::new(StandaloneCachePartition))
        .build()
        .unwrap()
}

/// Waits for detached body capture by retrying a cache-preferred probe.
async fn wait_for_cached(controller: &RequestController, request: &Request) -> net::Response {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut probe = request
                .try_clone()
                .expect("cache probe request should be cloneable");
            probe.set_cache_mode(CacheMode::ForceCache);
            match controller.fetch(probe).await {
                Ok(response) if response.from_cache => break response,
                Ok(response) => {
                    let _ = response.body.bytes().await;
                    tokio::task::yield_now().await;
                }
                Err(RequestError::NetworkError | RequestError::Transport(_)) => {
                    tokio::task::yield_now().await
                }
                Err(error) => panic!("cache probe failed unexpectedly: {error}"),
            }
        }
    })
    .await
    .expect("background body capture should finish within five seconds")
}

impl RequestPolicyService for SameOriginCacheServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }
}

impl ResponsePolicyService for SameOriginCacheServices {
    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        Ok(())
    }
}

impl CookieService for SameOriginCacheServices {
    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        Ok(None)
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        Ok(())
    }
}

impl ServiceWorkerService for SameOriginCacheServices {
    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }
}

impl CachePartitionService for SameOriginCacheServices {
    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("same-origin-cache-test".into())
    }
}

struct RecordingCookieCacheServices {
    stored: Arc<Mutex<Vec<String>>>,
    validation_effects: Arc<Mutex<Vec<String>>>,
}

struct EffectiveVaryServices;

impl RequestPolicyService for EffectiveVaryServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }

    fn generated_request_headers(
        &self,
        request: &Request,
        _target: &net::Url,
    ) -> Result<net::Headers, RequestError> {
        let mut headers = net::Headers::generated_request_headers();
        headers
            .set_str(
                "sec-fetch-site",
                &request.headers().get("x-cache-variant").unwrap(),
            )
            .unwrap();
        Ok(headers)
    }
}

impl ResponsePolicyService for EffectiveVaryServices {
    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        Ok(())
    }
}

impl CookieService for EffectiveVaryServices {
    fn cookie_header(&self, request: &Request) -> Result<Option<String>, RequestError> {
        Ok(Some(format!(
            "session={}",
            request.headers().get("x-cache-variant").unwrap()
        )))
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        Ok(())
    }
}

impl ServiceWorkerService for EffectiveVaryServices {
    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }
}

impl CachePartitionService for EffectiveVaryServices {
    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("effective-vary-test".into())
    }
}

#[test]
fn vary_matches_generated_and_cookie_headers_from_the_outgoing_request() {
    let server = TestServer::start(4, |request| {
        let vary = if request.starts_with("GET /generated ") {
            "Sec-Fetch-Site"
        } else if request.starts_with("GET /cookie ") {
            "Cookie"
        } else {
            panic!("unexpected request: {request}");
        };
        format!(
            "HTTP/1.1 204 No Content\r\nCache-Control: max-age=60\r\nVary: {vary}\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });
    let controller = RequestController::builder(Config::default())
        .with_browser_capabilities(Arc::new(EffectiveVaryServices))
        .build()
        .unwrap();

    runtime().block_on(async {
        for path in ["generated", "cookie"] {
            for variant in ["one", "two"] {
                let mut request = Request::get(format!("{}/{path}", server.url()));
                request.set_credentials_mode(net::CredentialsMode::Include);
                request.set_environment(
                    FetchEnvironment::from_url(reqwest::Url::parse(request.url()).unwrap())
                        .unwrap(),
                );
                request
                    .set_header(
                        HeaderName::from_static("x-cache-variant"),
                        HeaderValue::from_static(variant),
                    )
                    .unwrap();
                let response = controller.fetch(request).await.unwrap();
                assert!(
                    !response.from_cache,
                    "{path} variant {variant} reused another variant"
                );
            }
        }

        let mut repeated = Request::get(format!("{}/generated", server.url()));
        repeated.set_credentials_mode(net::CredentialsMode::Include);
        repeated.set_environment(
            FetchEnvironment::from_url(reqwest::Url::parse(repeated.url()).unwrap()).unwrap(),
        );
        repeated
            .set_header(
                HeaderName::from_static("x-cache-variant"),
                HeaderValue::from_static("two"),
            )
            .unwrap();
        assert!(controller.fetch(repeated).await.unwrap().from_cache);
    });

    server.join();
}

impl RequestPolicyService for RecordingCookieCacheServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }
}

impl ResponsePolicyService for RecordingCookieCacheServices {
    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        Ok(())
    }

    fn validation_response_headers(&self, _request: &Request, headers: net::Headers) {
        if let Some(effect) = headers.get("x-validation-effect") {
            self.validation_effects.lock().unwrap().push(effect);
        }
    }
}

impl CookieService for RecordingCookieCacheServices {
    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        Ok(None)
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        self.stored.lock().unwrap().extend(
            values
                .iter()
                .map(|value| value.to_str().unwrap().to_owned()),
        );
        Ok(())
    }
}

impl ServiceWorkerService for RecordingCookieCacheServices {
    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }
}

impl CachePartitionService for RecordingCookieCacheServices {
    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("recording-cookie-cache-test".into())
    }
}

#[test]
fn empty_stream_body_can_be_consumed_and_cached() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 204 No Content\r\nCache-Control: max-age=60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = cache_controller();

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
        .with_browser_capabilities(Arc::new(SameOriginCacheServices))
        .build()
        .unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_mode(RequestMode::SameOrigin).unwrap();
        request.set_environment(
            FetchEnvironment::from_url(reqwest::Url::parse(request.url()).unwrap()).unwrap(),
        );
        let response = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(response.body.length(), Some(6));
        assert_eq!(collect_body(response).await.unwrap(), b"cached");
        let mut cached_request = request;
        cached_request.set_cache_mode(CacheMode::OnlyIfCached);
        let cached = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match controller.fetch(cached_request.try_clone().unwrap()).await {
                    Ok(response) if response.from_cache => break response,
                    Err(RequestError::NetworkError) => tokio::task::yield_now().await,
                    Ok(_) => panic!("only-if-cached response must come from the cache"),
                    Err(error) => panic!("cache probe failed unexpectedly: {error}"),
                }
            }
        })
        .await
        .expect("fully consumed body should be cached within five seconds");
        assert!(cached.from_cache);
        assert_eq!(cached.body.length(), Some(6));
        assert_eq!(cached.body.bytes().await.unwrap(), b"cached");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[test]
fn no_cache_revalidates_with_all_validators_and_reuses_the_merged_representation() {
    let server = TestServer::start(2, |request| {
        let request = request.to_ascii_lowercase();
        if request.contains("if-none-match: \"version-1\"") {
            assert!(request.contains("cache-control: max-age=0"));
            assert!(request.contains("if-modified-since: sun, 06 nov 1994 08:49:37 gmt"));
            b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nETag: \"version-2\"\r\nX-Revalidated: yes\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nETag: \"version-1\"\r\nLast-Modified: Sun, 06 Nov 1994 08:49:37 GMT\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
        }
    });
    let controller = cache_controller();

    let (revalidated, cached) = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"cached");
        for _ in 0..4_096 {
            tokio::task::yield_now().await;
        }
        let mut no_cache = request;
        no_cache.set_cache_mode(CacheMode::NoCache);
        let revalidated = controller.fetch(no_cache).await.unwrap();
        let revalidated = (
            revalidated.from_cache,
            revalidated.headers.get("x-revalidated"),
            collect_body(revalidated).await.unwrap(),
        );
        let cached = wait_for_cached(&controller, &Request::get(server.url())).await;
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
fn revalidation_processes_304_cookies_once_without_caching_them() {
    let server = TestServer::start(2, |request| {
        if request
            .to_ascii_lowercase()
            .contains("if-none-match: \"version-1\"")
        {
            b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nETag: \"version-2\"\r\nSet-Cookie: refreshed=two\r\nX-Validation-Effect: revalidated\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nETag: \"version-1\"\r\nSet-Cookie: initial=one\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
        }
    });
    let stored = Arc::new(Mutex::new(Vec::new()));
    let validation_effects = Arc::new(Mutex::new(Vec::new()));
    let controller = RequestController::builder(Config::default())
        .with_browser_capabilities(Arc::new(RecordingCookieCacheServices {
            stored: Arc::clone(&stored),
            validation_effects: Arc::clone(&validation_effects),
        }))
        .build()
        .unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_mode(RequestMode::SameOrigin).unwrap();
        request.set_environment(
            FetchEnvironment::from_url(reqwest::Url::parse(request.url()).unwrap()).unwrap(),
        );
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"cached");
        let cached = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut probe = request.try_clone().unwrap();
                probe.set_cache_mode(CacheMode::OnlyIfCached);
                match controller.fetch(probe).await {
                    Ok(response) if response.from_cache => break response,
                    Err(RequestError::NetworkError) => tokio::task::yield_now().await,
                    Ok(_) => panic!("only-if-cached response must come from the cache"),
                    Err(error) => panic!("cache probe failed unexpectedly: {error}"),
                }
            }
        })
        .await
        .expect("initial body capture should finish within five seconds");
        drop(cached);

        let mut revalidate = request.try_clone().unwrap();
        revalidate.set_cache_mode(CacheMode::NoCache);
        let revalidated = controller.fetch(revalidate).await.unwrap();
        assert!(revalidated.from_cache);
        assert_eq!(collect_body(revalidated).await.unwrap(), b"cached");

        let cached = controller.fetch(request).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(collect_body(cached).await.unwrap(), b"cached");
    });

    server.join();
    assert_eq!(
        stored.lock().unwrap().as_slice(),
        ["initial=one", "refreshed=two"]
    );
    assert_eq!(
        validation_effects.lock().unwrap().as_slice(),
        ["revalidated"]
    );
}

#[test]
fn no_cache_without_a_validator_replaces_the_cached_representation() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |request| {
        assert!(!request.to_ascii_lowercase().contains("if-none-match:"));
        if observed_hits.fetch_add(1, Ordering::SeqCst) == 0 {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfirst".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecond".to_vec()
        }
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"first");
        wait_for_cached(&controller, &request).await;

        let mut revalidate = request.try_clone().unwrap();
        revalidate.set_cache_mode(CacheMode::NoCache);
        let replacement = controller.fetch(revalidate).await.unwrap();
        assert!(!replacement.from_cache);
        assert_eq!(collect_body(replacement).await.unwrap(), b"second");

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut probe = request.try_clone().unwrap();
                probe.set_cache_mode(CacheMode::ForceCache);
                let cached = controller.fetch(probe).await.unwrap();
                if cached.from_cache && collect_body(cached).await.unwrap() == b"second" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("replacement body capture should finish within five seconds");
    });

    server.join();
}

#[test]
fn failed_revalidation_keeps_the_existing_representation() {
    let server = TestServer::start(2, |request| {
        if request
            .to_ascii_lowercase()
            .contains("if-none-match: \"version-1\"")
        {
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfail".to_vec()
        } else {
            b"HTTP/1.1 204 No Content\r\nCache-Control: max-age=0\r\nETag: \"version-1\"\r\nConnection: close\r\n\r\n".to_vec()
        }
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"");

        let failure = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert!(!failure.from_cache);
        assert_eq!(collect_body(failure).await.unwrap(), b"fail");

        let mut cached_request = request;
        cached_request.set_cache_mode(CacheMode::ForceCache);
        let cached = controller.fetch(cached_request).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(collect_body(cached).await.unwrap(), b"");
    });

    server.join();
}

#[test]
fn transport_failed_revalidation_keeps_the_existing_representation() {
    let server = TestServer::start(2, |request| {
        if request
            .to_ascii_lowercase()
            .contains("if-none-match: \"version-1\"")
        {
            // Closing a response before writing HTTP headers makes reqwest
            // report a transport failure rather than a cacheable response.
            Vec::new()
        } else {
            b"HTTP/1.1 204 No Content\r\nCache-Control: max-age=0\r\nETag: \"version-1\"\r\nConnection: close\r\n\r\n".to_vec()
        }
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"");

        assert!(matches!(
            controller.fetch(request.try_clone().unwrap()).await,
            Err(RequestError::NetworkError | RequestError::Transport(_))
        ));

        let mut force_cache = request;
        force_cache.set_cache_mode(CacheMode::ForceCache);
        let cached = controller.fetch(force_cache).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(collect_body(cached).await.unwrap(), b"");
    });

    server.join();
}

#[test]
fn stale_while_revalidate_serves_stale_and_refreshes_once_in_the_background() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |request| {
        let hit = observed_hits.fetch_add(1, Ordering::SeqCst);
        if hit == 0 {
            b"HTTP/1.1 204 No Content\r\nCache-Control: max-age=0, stale-while-revalidate=60\r\nETag: \"stale\"\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("if-none-match: \"stale\"")
            );
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfresh".to_vec()
        }
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let request = Request::get(server.url());
        let initial = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(initial).await.unwrap(), b"");

        let stale = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert!(stale.from_cache);
        assert_eq!(collect_body(stale).await.unwrap(), b"");

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut probe = request.try_clone().unwrap();
                probe.set_cache_mode(CacheMode::ForceCache);
                let response = controller.fetch(probe).await.unwrap();
                if response.from_cache && collect_body(response).await.unwrap() == b"fresh" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("background refresh should complete");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn stale_if_error_returns_a_cached_representation_for_server_failures() {
    let server = TestServer::start(2, |request| {
        if request
            .to_ascii_lowercase()
            .contains("if-none-match: \"stale\"")
        {
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfail".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=0, stale-if-error=60\r\nETag: \"stale\"\r\nContent-Length: 5\r\nConnection: close\r\n\r\nstale".to_vec()
        }
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let request = Request::get(server.url());
        let initial = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(initial).await.unwrap(), b"stale");
        wait_for_cached(&controller, &request).await;

        let fallback = controller.fetch(request).await.unwrap();
        assert!(fallback.from_cache);
        assert_eq!(collect_body(fallback).await.unwrap(), b"stale");
    });

    server.join();
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
    let controller = cache_controller();

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
fn force_cache_serves_a_permitted_stale_representation_without_network_io() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=0\r\nContent-Length: 5\r\nConnection: close\r\n\r\nstale".to_vec()
    });
    let controller = cache_controller();

    let cached = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"stale");
        wait_for_cached(&controller, &request).await;

        let mut force_cache = request;
        force_cache.set_cache_mode(CacheMode::ForceCache);
        controller.fetch(force_cache).await.unwrap()
    });

    server.join();
    assert!(cached.from_cache);
    assert_eq!(runtime().block_on(cached.body.bytes()).unwrap(), b"stale");
}

#[test]
fn force_cache_and_only_if_cached_serve_entries_that_require_revalidation() {
    let server = TestServer::start(2, |request| {
        if request.starts_with("GET /force ") {
            b"HTTP/1.1 200 OK\r\nCache-Control: no-cache, max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nforce".to_vec()
        } else if request.starts_with("GET /only ") {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=0, must-revalidate\r\nContent-Length: 4\r\nConnection: close\r\n\r\nonly".to_vec()
        } else {
            panic!("unexpected request: {request}");
        }
    });
    let controller = RequestController::builder(Config::default())
        .with_browser_capabilities(Arc::new(SameOriginCacheServices))
        .build()
        .unwrap();

    let (force, only_if_cached) = runtime().block_on(async {
        let force_url = format!("{}/force", server.url());
        let force_request = Request::get(&force_url);
        let first = controller
            .fetch(force_request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"force");
        let force = wait_for_cached(&controller, &force_request).await;

        let only_url = format!("{}/only", server.url());
        let mut only_request = Request::get(&only_url);
        only_request.set_mode(RequestMode::SameOrigin).unwrap();
        only_request.set_environment(
            FetchEnvironment::from_url(url::Url::parse(only_request.url()).unwrap()).unwrap(),
        );
        let first = controller
            .fetch(only_request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(first).await.unwrap(), b"only");
        wait_for_cached(&controller, &only_request).await;
        only_request.set_cache_mode(CacheMode::OnlyIfCached);
        let only_if_cached = controller.fetch(only_request).await.unwrap();

        (force, only_if_cached)
    });

    server.join();
    assert!(force.from_cache);
    assert_eq!(runtime().block_on(force.body.bytes()).unwrap(), b"force");
    assert!(only_if_cached.from_cache);
    assert_eq!(
        runtime().block_on(only_if_cached.body.bytes()).unwrap(),
        b"only"
    );
}

#[test]
fn reload_bypasses_cache_and_caches_the_refresh_when_consumed() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |request| {
        let hit = observed_hits.fetch_add(1, Ordering::SeqCst);
        if hit == 1 {
            assert!(request.contains("cache-control: no-cache"));
            assert!(request.contains("pragma: no-cache"));
        }
        let body = if hit == 0 { "first" } else { "second" };
        format!("HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_bytes()
    });
    let controller = cache_controller();

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
        let cached = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut probe = Request::get(server.url());
                probe.set_cache_mode(CacheMode::ForceCache);
                let response = controller.fetch(probe).await.unwrap();
                let cached = (response.from_cache, collect_body(response).await.unwrap());
                if cached.0 && cached.1 == b"second" {
                    break cached;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("refreshed background body capture should finish within five seconds");
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
fn cache_mode_directives_preserve_caller_headers() {
    let server = TestServer::start(3, |request| {
        if request.starts_with("GET /reload ") {
            assert!(request.contains("cache-control: max-age=120"));
            assert!(request.contains("pragma: client-cache-rule"));
            assert!(!request.contains("cache-control: no-cache"));
            assert!(!request.contains("pragma: no-cache"));
        } else if request.starts_with("GET /no-cache ") {
            assert!(request.contains("cache-control: max-age=120"));
            assert!(!request.contains("cache-control: max-age=0"));
        } else if request.starts_with("GET /no-store ") {
            assert!(request.contains("cache-control: max-age=120"));
            assert!(request.contains("pragma: client-cache-rule"));
            assert!(!request.contains("cache-control: no-cache"));
            assert!(!request.contains("pragma: no-cache"));
        } else {
            panic!("unexpected request: {request}");
        }
        b"HTTP/1.1 200 OK\r\nCache-Control: no-store\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let mut reload = Request::get(format!("{}/reload", server.url()));
        reload.set_cache_mode(CacheMode::Reload);
        reload
            .set_header(
                HeaderName::from_static("cache-control"),
                HeaderValue::from_static("max-age=120"),
            )
            .unwrap();
        reload
            .set_header(
                HeaderName::from_static("pragma"),
                HeaderValue::from_static("client-cache-rule"),
            )
            .unwrap();
        assert_eq!(
            collect_body(controller.fetch(reload).await.unwrap())
                .await
                .unwrap(),
            b"ok"
        );

        let mut no_cache = Request::get(format!("{}/no-cache", server.url()));
        no_cache.set_cache_mode(CacheMode::NoCache);
        no_cache
            .set_header(
                HeaderName::from_static("cache-control"),
                HeaderValue::from_static("max-age=120"),
            )
            .unwrap();
        assert_eq!(
            collect_body(controller.fetch(no_cache).await.unwrap())
                .await
                .unwrap(),
            b"ok"
        );

        let mut no_store = Request::get(format!("{}/no-store", server.url()));
        no_store.set_cache_mode(CacheMode::NoStore);
        no_store
            .set_header(
                HeaderName::from_static("cache-control"),
                HeaderValue::from_static("max-age=120"),
            )
            .unwrap();
        no_store
            .set_header(
                HeaderName::from_static("pragma"),
                HeaderValue::from_static("client-cache-rule"),
            )
            .unwrap();
        assert_eq!(
            collect_body(controller.fetch(no_store).await.unwrap())
                .await
                .unwrap(),
            b"ok"
        );
    });

    server.join();
}

#[test]
fn no_store_sends_cache_bypass_directives_when_the_caller_omits_them() {
    let server = TestServer::start(1, |request| {
        assert!(request.contains("pragma: no-cache"));
        assert!(request.contains("cache-control: no-cache"));
        b"HTTP/1.1 200 OK\r\nCache-Control: no-store\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_cache_mode(CacheMode::NoStore);
        assert_eq!(
            collect_body(controller.fetch(request).await.unwrap())
                .await
                .unwrap(),
            b"ok"
        );
    });

    server.join();
}

#[test]
fn default_request_with_a_caller_validator_bypasses_and_preserves_the_cache() {
    let server = TestServer::start(2, |request| {
        if request.contains("if-none-match: \"caller\"") {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 9\r\nConnection: close\r\n\r\nvalidated".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
        }
    });
    let controller = cache_controller();
    let url = server.url().to_owned();

    runtime().block_on(async {
        let request = Request::get(&url);
        assert_eq!(
            collect_body(
                controller
                    .fetch(request.try_clone().unwrap())
                    .await
                    .unwrap()
            )
            .await
            .unwrap(),
            b"cached"
        );
        wait_for_cached(&controller, &request).await;

        let mut conditional = request;
        conditional
            .set_header(
                HeaderName::from_static("if-none-match"),
                HeaderValue::from_static("\"caller\""),
            )
            .unwrap();
        assert_eq!(
            collect_body(controller.fetch(conditional).await.unwrap())
                .await
                .unwrap(),
            b"validated"
        );

        let cached = wait_for_cached(&controller, &Request::get(&url)).await;
        assert_eq!(collect_body(cached).await.unwrap(), b"cached");
    });

    server.join();
}

#[test]
fn caller_no_cache_directive_revalidates_a_fresh_entry() {
    let server = TestServer::start(2, |request| {
        if request.contains("if-none-match: \"cached\"") {
            b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\n"
                .to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nETag: \"cached\"\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
        }
    });
    let controller = cache_controller();
    let url = server.url().to_owned();

    runtime().block_on(async {
        let first = Request::get(&url);
        assert_eq!(
            collect_body(controller.fetch(first.try_clone().unwrap()).await.unwrap())
                .await
                .unwrap(),
            b"cached"
        );
        drop(wait_for_cached(&controller, &first).await);

        let mut revalidate = Request::get(&url);
        revalidate
            .set_header(
                HeaderName::from_static("cache-control"),
                HeaderValue::from_static("no-cache"),
            )
            .unwrap();
        let response = controller.fetch(revalidate).await.unwrap();
        assert!(response.from_cache);
        assert_eq!(collect_body(response).await.unwrap(), b"cached");
    });

    server.join();
}

#[test]
fn missing_cache_partition_uses_no_store_behavior() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 7\r\nConnection: close\r\n\r\nnetwork".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    runtime().block_on(async {
        for _ in 0..2 {
            let response = controller.fetch(Request::get(server.url())).await.unwrap();
            assert!(!response.from_cache);
            assert_eq!(collect_body(response).await.unwrap(), b"network");
        }
    });

    server.join();
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
    let controller = cache_controller();
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
fn unsuccessful_unsafe_response_preserves_its_request_target() {
    let server = TestServer::start(2, move |request| {
        if request.starts_with("POST /resource ") {
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec()
        } else if request.starts_with("GET /resource ") {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 3\r\nConnection: close\r\n\r\nold".to_vec()
        } else {
            panic!("unexpected request: {request}");
        }
    });
    let controller = cache_controller();
    let resource = format!("{}/resource", server.url());

    runtime().block_on(async {
        let initial = controller.fetch(Request::get(&resource)).await.unwrap();
        assert_eq!(collect_body(initial).await.unwrap(), b"old");
        wait_for_cached(&controller, &Request::get(&resource)).await;

        let mutation = controller
            .fetch(Request::new(&resource, "POST").unwrap())
            .await
            .unwrap();
        assert_eq!(mutation.status, 500);

        let cached = controller.fetch(Request::get(&resource)).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(collect_body(cached).await.unwrap(), b"old");
    });

    server.join();
}

#[test]
fn unsuccessful_unsafe_response_does_not_invalidate_its_location_target() {
    let server = TestServer::start(2, |request| {
        if request.starts_with("POST /mutate ") {
            b"HTTP/1.1 404 Not Found\r\nLocation: /target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec()
        } else if request.starts_with("GET /target ") {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
        } else {
            panic!("unexpected request: {request}");
        }
    });
    let controller = cache_controller();
    let target = format!("{}/target", server.url());
    let mutation = format!("{}/mutate", server.url());

    runtime().block_on(async {
        let initial = controller.fetch(Request::get(&target)).await.unwrap();
        assert_eq!(collect_body(initial).await.unwrap(), b"cached");
        wait_for_cached(&controller, &Request::get(&target)).await;

        let response = controller
            .fetch(Request::new(&mutation, "POST").unwrap())
            .await
            .unwrap();
        assert_eq!(response.status, 404);

        let cached = wait_for_cached(&controller, &Request::get(&target)).await;
        assert_eq!(collect_body(cached).await.unwrap(), b"cached");
    });

    server.join();
}

#[test]
fn unsafe_response_cannot_invalidate_a_cross_origin_location_target() {
    let target_server = TestServer::start(1, |request| {
        assert!(request.starts_with("GET /target "));
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let cross_origin_target = format!("{}/target", target_server.url());
    let location = cross_origin_target.clone();
    let mutation_server = TestServer::start(1, move |request| {
        assert!(request.starts_with("POST /mutate "));
        format!("HTTP/1.1 201 Created\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let initial = controller
            .fetch(Request::get(&cross_origin_target))
            .await
            .unwrap();
        assert_eq!(collect_body(initial).await.unwrap(), b"cached");
        wait_for_cached(&controller, &Request::get(&cross_origin_target)).await;

        let mutation = Request::new(format!("{}/mutate", mutation_server.url()), "POST").unwrap();
        let response = controller.fetch(mutation).await.unwrap();
        assert_eq!(response.status, 201);

        let cached = wait_for_cached(&controller, &Request::get(&cross_origin_target)).await;
        assert_eq!(collect_body(cached).await.unwrap(), b"cached");
    });

    target_server.join();
    mutation_server.join();
}

#[test]
fn only_if_cached_returns_a_cache_hit() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nX-Cache-Test: yes\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .with_browser_capabilities(Arc::new(SameOriginCacheServices))
        .build()
        .unwrap();

    let (from_cache, body, header) = runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_mode(RequestMode::SameOrigin).unwrap();
        request.set_environment(
            FetchEnvironment::from_url(url::Url::parse(request.url()).unwrap()).unwrap(),
        );
        collect_body(
            controller
                .fetch(request.try_clone().unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        let response = wait_for_cached(&controller, &request).await;
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
    let controller = cache_controller();

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
        .with_browser_capabilities(Arc::new(SameOriginCacheServices))
        .build()
        .unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_mode(RequestMode::SameOrigin).unwrap();
        request.set_environment(
            FetchEnvironment::from_url(reqwest::Url::parse(request.url()).unwrap()).unwrap(),
        );
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert!(!first.from_cache);
        collect_body(first).await.unwrap();
        let cached = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut probe = request.try_clone().unwrap();
                probe.set_cache_mode(CacheMode::OnlyIfCached);
                match controller.fetch(probe).await {
                    Ok(response) if response.from_cache => break response,
                    Err(RequestError::NetworkError) => tokio::task::yield_now().await,
                    Ok(_) => panic!("only-if-cached response must come from the cache"),
                    Err(error) => panic!("cache probe failed unexpectedly: {error}"),
                }
            }
        })
        .await
        .expect("initial body capture should finish within five seconds");
        drop(cached);
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
    let server = TestServer::start(2, move |request| {
        let hit = observed_hits.fetch_add(1, Ordering::SeqCst);
        if hit == 1 {
            assert!(request.contains("cache-control: no-cache"));
        }
        let body = if hit == 0 { "first" } else { "second" };
        format!("HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_bytes()
    });
    let controller = cache_controller();

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
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut probe = Request::get(server.url());
                probe.set_cache_mode(CacheMode::ForceCache);
                match controller.fetch(probe).await {
                    Ok(response) => {
                        if response.body.bytes().await.unwrap() == b"second" {
                            break;
                        }
                    }
                    Err(RequestError::NetworkError | RequestError::Transport(_)) => {}
                    Err(error) => panic!("cache probe failed unexpectedly: {error}"),
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("refreshed background body capture should finish within five seconds");
        let cached = wait_for_cached(&controller, &Request::get(server.url())).await;
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
        .with_browser_capabilities(Arc::new(SameOriginCacheServices))
        .build()
        .unwrap();

    let cached = runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_mode(RequestMode::SameOrigin).unwrap();
        request.set_environment(
            FetchEnvironment::from_url(url::Url::parse(request.url()).unwrap()).unwrap(),
        );
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        collect_body(first).await.unwrap();
        wait_for_cached(&controller, &request).await
    });

    server.join();
    assert!(cached.from_cache);
    assert_eq!(runtime().block_on(cached.body.bytes()).unwrap(), b"cached");
}

#[test]
fn only_if_cached_follows_a_cached_redirect_without_network_io() {
    let server = TestServer::start(2, |request| {
        if request.starts_with("GET /start ") {
            b"HTTP/1.1 302 Found\r\nLocation: /target\r\nCache-Control: max-age=60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else if request.starts_with("GET /target ") {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
        } else {
            panic!("unexpected request: {request}");
        }
    });
    let controller = RequestController::builder(Config::default())
        .with_browser_capabilities(Arc::new(SameOriginCacheServices))
        .build()
        .unwrap();
    let start = format!("{}/start", server.url());
    let target = format!("{}/target", server.url());

    let response = runtime().block_on(async {
        let mut request = Request::get(&start);
        request.set_mode(RequestMode::SameOrigin).unwrap();
        request
            .set_environment(FetchEnvironment::from_url(url::Url::parse(&start).unwrap()).unwrap());
        assert_eq!(
            collect_body(
                controller
                    .fetch(request.try_clone().unwrap())
                    .await
                    .unwrap()
            )
            .await
            .unwrap(),
            b"cached"
        );

        let mut target_request = Request::get(&target);
        target_request.set_mode(RequestMode::SameOrigin).unwrap();
        target_request
            .set_environment(FetchEnvironment::from_url(url::Url::parse(&start).unwrap()).unwrap());
        let cached_target = wait_for_cached(&controller, &target_request).await;
        assert!(cached_target.from_cache);
        drop(cached_target);

        request.set_cache_mode(CacheMode::OnlyIfCached);
        controller.fetch(request).await.unwrap()
    });

    server.join();
    assert!(response.from_cache);
    assert!(response.redirected);
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"cached"
    );
}

#[test]
fn only_if_cached_requires_same_origin_mode_without_network_access() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = cache_controller();

    runtime().block_on(async {
        let request = Request::get(server.url());
        let response = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        assert_eq!(collect_body(response).await.unwrap(), b"cached");

        let mut only_cached = request;
        only_cached.set_cache_mode(CacheMode::OnlyIfCached);
        assert!(matches!(
            controller.fetch(only_cached).await,
            Err(RequestError::NetworkError)
        ));
    });

    server.join();
}

#[test]
fn cloned_controllers_share_the_response_cache() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\nshared".to_vec()
    });
    let controller = cache_controller();
    let clone = controller.clone();

    let (first, second) = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller
            .fetch(request.try_clone().unwrap())
            .await
            .unwrap();
        let first_from_cache = first.from_cache;
        collect_body(first).await.unwrap();
        let second = wait_for_cached(&clone, &request).await;
        (first_from_cache, second)
    });

    server.join();
    assert!(!first);
    assert!(second.from_cache);
}
