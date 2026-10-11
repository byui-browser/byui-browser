//! Header behavior and transport-boundary integration tests.

mod common;

use std::sync::{Arc, Mutex};

use common::{TestServer, runtime};
use net::{
    Body, CacheMode, Config, CredentialsMode, FetchEnvironment, FetchServices, Referrer,
    ReferrerPolicy, Request, RequestController, RequestError, RequestMode, ResponseInfo,
    ResponseType, ServiceWorkerDecision,
};
use reqwest::{
    StatusCode,
    header::{HeaderName, HeaderValue},
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
        .set_body(Some(Body::from_bytes(b"request body".to_vec())))
        .unwrap();
    request.set_cache_mode(CacheMode::NoStore);

    let controller = RequestController::new(Config::default()).unwrap();
    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(
        runtime().block_on(response.body.bytes()).unwrap(),
        b"created"
    );
    assert!(!response.from_cache);
}

#[test]
fn generated_request_header_channels_do_not_mutate_caller_headers() {
    struct ChannelServices;

    impl FetchServices for ChannelServices {
        fn check_request(&self, request: &Request) -> Result<(), RequestError> {
            assert!(!request.headers().has("origin"));
            assert!(!request.headers().has("referer"));
            assert!(!request.headers().has("cookie"));
            assert!(!request.headers().has("content-type"));
            Ok(())
        }

        fn check_response(
            &self,
            _request: &Request,
            _info: &ResponseInfo,
        ) -> Result<(), RequestError> {
            Ok(())
        }

        fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
            Ok(Some("sid=internal".into()))
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
            Ok("header-channel-test".into())
        }
    }

    let server = TestServer::start(1, |wire_request| {
        assert!(wire_request.contains("origin: http://client.test"));
        assert!(wire_request.contains("referer: http://client.test/source"));
        assert!(wire_request.contains("cookie: sid=internal"));
        assert!(wire_request.contains("content-type: text/plain;charset=UTF-8"));
        b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: http://client.test\r\nAccess-Control-Allow-Credentials: true\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let mut request = Request::new(server.url(), "POST").unwrap();
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("http://client.test/source").unwrap())
            .unwrap(),
    );
    request.set_credentials_mode(CredentialsMode::Include);
    request.set_referrer(
        Referrer::Url(reqwest::Url::parse("http://client.test/source").unwrap()),
        ReferrerPolicy::UnsafeUrl,
    );
    request
        .set_body(Some(Body::from_text("request body")))
        .unwrap();
    request.set_cache_mode(CacheMode::NoStore);
    assert!(request.headers().is_empty());

    let controller =
        RequestController::with_services(Config::default(), Arc::new(ChannelServices)).unwrap();
    let response = runtime()
        .block_on(controller.fetch(request.clone()))
        .unwrap();
    server.join();

    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    assert!(request.headers().is_empty());
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
    assert_eq!(
        runtime().block_on(response.unwrap().body.bytes()).unwrap(),
        b"ok"
    );
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
            RequestController::with_services(Config::default(), Arc::new(TestServices))
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
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
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
            RequestController::with_services(Config::default(), Arc::new(TestServices))
                .unwrap()
                .fetch(request),
        )
        .unwrap();
    server.join();
    assert_eq!(response.response_type, ResponseType::Opaque);
    assert_eq!(response.status, 0);
    assert!(response.url.is_empty());
    assert!(response.headers.is_empty());
    assert!(response.body.is_null());
}

#[test]
fn browser_services_own_cookie_and_raw_response_header_decisions() {
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
            assert_eq!(info.headers.get_set_cookie(), ["sid=new", "theme=dark"]);
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
        b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=new\r\nSet-Cookie: theme=dark\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
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
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    assert!(!response.headers.has("set-cookie"));
    assert_eq!(
        &*stored.lock().unwrap(),
        &["sid=new".to_owned(), "theme=dark".to_owned()]
    );
}
