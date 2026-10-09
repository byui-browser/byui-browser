//! Request construction and transport-facing integration tests.

mod common;

use common::{TestServer, runtime};
use net::{
    CacheMode, Config, FetchEnvironment, FetchServices, Request, RequestBody, RequestController,
    RequestError, RequestMode, ResponseInfo, ResponseType, ServiceWorkerDecision,
};

struct TestServices;

impl FetchServices for TestServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }
    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        Ok(())
    }
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
    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }
    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("test-partition".into())
    }
}
use reqwest::{
    StatusCode,
    header::{HeaderName, HeaderValue},
};

#[test]
fn fetch_sends_headers_and_body_to_a_local_server() {
    let server = TestServer::start(1, |request| {
        assert!(request.contains("x-test: integration"));
        assert!(request.ends_with("request body"));
        b"HTTP/1.1 201 Created\r\nContent-Length: 7\r\nConnection: close\r\n\r\ncreated".to_vec()
    });

    let mut request = Request::get(server.url());
    request.set_method("POST").unwrap();
    request
        .set_header(
            HeaderName::from_static("x-test"),
            HeaderValue::from_static("integration"),
        )
        .unwrap();
    request
        .set_body(Some(RequestBody::bytes(b"request body".to_vec())))
        .unwrap();
    request.set_cache_mode(CacheMode::NoStore);

    let controller = RequestController::new(Config::default()).unwrap();
    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.body, b"created");
    assert!(!response.from_cache);
}

#[test]
fn configured_user_agent_is_sent_to_the_server() {
    let server = TestServer::start(1, |request| {
        assert!(request.contains("user-agent: test-browser/1.0"));
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });

    let config = Config {
        user_agent: Some(HeaderValue::from_static("test-browser/1.0")),
        ..Config::default()
    };
    let controller = RequestController::new(config).unwrap();
    let response = runtime().block_on(controller.fetch(Request::get(server.url())));

    server.join();
    assert_eq!(response.unwrap().body, b"ok");
}

#[test]
fn fetch_propagates_a_body_transport_error() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nshort\r\n"
            .to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let error = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::Transport(_)));
}

#[test]
fn unsupported_schemes_are_rejected_without_network_io() {
    let controller = RequestController::new(Config::default()).unwrap();
    let error = runtime()
        .block_on(controller.fetch(Request::get("ftp://example.test/file")))
        .unwrap_err();

    assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
}

#[test]
fn non_cacheable_post_requests_are_sent_each_time() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.set_method("POST").unwrap();
        request
            .set_body(Some(RequestBody::bytes(b"payload".to_vec())))
            .unwrap();
        controller.fetch(request.clone()).await.unwrap();
        controller.fetch(request).await.unwrap();
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn cross_origin_fetch_exposes_only_cors_allowed_headers() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Expose-Headers: x-visible\r\nX-Visible: yes\r\nX-Secret: hidden\r\nSet-Cookie: sid=secret\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("https://client.test/").unwrap()).unwrap(),
    );
    let response = runtime()
        .block_on(
            RequestController::with_services(Config::default(), std::sync::Arc::new(TestServices))
                .unwrap()
                .fetch(request),
        )
        .unwrap();
    server.join();
    assert_eq!(response.response_type, ResponseType::Cors);
    assert_eq!(response.status, 200);
    assert_eq!(response.headers.get("x-visible").unwrap(), "yes");
    assert!(!response.headers.has("x-secret"));
    assert!(!response.headers.has("set-cookie"));
    assert_eq!(response.body, b"ok");
}

#[test]
fn cross_origin_no_cors_fetch_is_opaque() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nX-Secret: hidden\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecret".to_vec()
    });
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("https://client.test/").unwrap()).unwrap(),
    );
    request.set_mode(RequestMode::NoCors).unwrap();
    let response = runtime()
        .block_on(
            RequestController::with_services(Config::default(), std::sync::Arc::new(TestServices))
                .unwrap()
                .fetch(request),
        )
        .unwrap();
    server.join();
    assert_eq!(response.response_type, ResponseType::Opaque);
    assert_eq!(response.status, 0);
    assert!(response.url.is_empty());
    assert!(response.headers.is_empty());
    assert!(response.body.is_empty());
}

#[test]
fn browser_context_without_services_fails_before_network_io() {
    let mut request = Request::get("https://remote.test/data");
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("https://client.test/").unwrap()).unwrap(),
    );
    let error = runtime()
        .block_on(
            RequestController::new(Config::default())
                .unwrap()
                .fetch(request),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("browser services")
    ));
}

#[test]
fn browser_services_own_cookie_and_raw_response_header_decisions() {
    use std::sync::{Arc, Mutex};

    struct CookieServices(Arc<Mutex<Vec<String>>>);
    impl FetchServices for CookieServices {
        fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
            Ok(())
        }
        fn check_response(
            &self,
            _request: &Request,
            info: &ResponseInfo,
        ) -> Result<(), RequestError> {
            assert_eq!(info.headers.get("set-cookie").unwrap(), "sid=new");
            Ok(())
        }
        fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
            Ok(Some("sid=old".into()))
        }
        fn store_set_cookie(
            &self,
            _request: &Request,
            values: &[net::HeaderValue],
        ) -> Result<(), RequestError> {
            self.0.lock().unwrap().extend(
                values
                    .iter()
                    .map(|value| value.to_str().unwrap().to_owned()),
            );
            Ok(())
        }
        fn service_worker(
            &self,
            _request: &Request,
        ) -> Result<ServiceWorkerDecision, RequestError> {
            Ok(ServiceWorkerDecision::Network)
        }
        fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
            Ok("cookie-test".into())
        }
    }

    let server = TestServer::start(1, |request| {
        assert!(request.contains("cookie: sid=old"));
        b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=new\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let stored = Arc::new(Mutex::new(Vec::new()));
    let services = Arc::new(CookieServices(stored.clone()));
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );
    request.set_cache_mode(CacheMode::NoStore);
    let response = runtime()
        .block_on(
            RequestController::with_services(Config::default(), services)
                .unwrap()
                .fetch(request),
        )
        .unwrap();
    server.join();
    assert_eq!(response.body, b"ok");
    assert!(!response.headers.has("set-cookie"));
    assert_eq!(&*stored.lock().unwrap(), &["sid=new".to_owned()]);
}

#[test]
fn cached_browser_response_is_rechecked_before_exposure() {
    struct RejectCached;
    impl FetchServices for RejectCached {
        fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
            Ok(())
        }
        fn check_response(
            &self,
            _request: &Request,
            info: &ResponseInfo,
        ) -> Result<(), RequestError> {
            if info.from_cache {
                Err(RequestError::UnsupportedFeature("cached response policy"))
            } else {
                Ok(())
            }
        }
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
        fn service_worker(
            &self,
            _request: &Request,
        ) -> Result<ServiceWorkerDecision, RequestError> {
            Ok(ServiceWorkerDecision::Network)
        }
        fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
            Ok("cache-check".into())
        }
    }

    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller =
        RequestController::with_services(Config::default(), std::sync::Arc::new(RejectCached))
            .unwrap();
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );
    runtime()
        .block_on(controller.fetch(request.clone()))
        .unwrap();
    let error = runtime().block_on(controller.fetch(request)).unwrap_err();
    server.join();
    assert!(matches!(
        error,
        RequestError::UnsupportedFeature("cached response policy")
    ));
}
