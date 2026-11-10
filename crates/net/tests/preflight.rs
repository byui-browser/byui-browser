//! CORS-preflight request and cache integration tests.

mod common;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use common::{TestServer, runtime};
use net::{
    Body, Config, FetchEnvironment, FetchServices, HeaderName, HeaderValue, Request,
    RequestController, RequestError, ResponseInfo, ServiceWorkerDecision,
};

/// Browser services used to supply an isolated cache partition without credentials.
struct TestServices;

impl FetchServices for TestServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }

    fn check_response(
        &self,
        _request: &Request,
        _response: &ResponseInfo,
    ) -> Result<(), RequestError> {
        Ok(())
    }

    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        Ok(None)
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[HeaderValue],
    ) -> Result<(), RequestError> {
        Ok(())
    }

    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }

    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("preflight-tests".into())
    }
}

/// Services that prove cookies are reserved for the original credentialed request.
struct CookieServices;

impl FetchServices for CookieServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }

    fn check_response(
        &self,
        _request: &Request,
        _response: &ResponseInfo,
    ) -> Result<(), RequestError> {
        Ok(())
    }

    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        Ok(Some("session=secret".into()))
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[HeaderValue],
    ) -> Result<(), RequestError> {
        Ok(())
    }

    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }

    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("credentialed-preflight".into())
    }
}

/// Services that record whether a cross-origin request reaches cookie policy.
struct RecordingCookieServices {
    reads: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
}

impl FetchServices for RecordingCookieServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        Ok(())
    }

    fn check_response(
        &self,
        _request: &Request,
        _response: &ResponseInfo,
    ) -> Result<(), RequestError> {
        Ok(())
    }

    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(Some("session=must-not-leak".into()))
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[HeaderValue],
    ) -> Result<(), RequestError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }

    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("cross-origin-cookie-tests".into())
    }
}

/// Builds a cross-origin browser request whose method and headers need preflight.
fn request(url: String) -> Request {
    let mut request = Request::new(url, "PUT").unwrap();
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("https://client.test/page").unwrap())
            .unwrap(),
    );
    request
        .set_header(
            HeaderName::from_static("x-example"),
            HeaderValue::from_static("one"),
        )
        .unwrap();
    request
        .set_body(Some(Body::from_bytes_with_content_type(
            br#"{"value":1}"#.to_vec(),
            Some(HeaderValue::from_static("application/json")),
        )))
        .unwrap();
    request
}

/// Builds a controller with the browser context required by cross-origin requests.
fn controller() -> RequestController {
    RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(TestServices))
        .build()
        .unwrap()
}

#[test]
fn unsafe_request_sends_preflight_before_the_original_request() {
    let sequence = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&sequence);
    let server = TestServer::start(2, move |request| {
        let index = observed.fetch_add(1, Ordering::SeqCst);
        if index == 0 {
            assert!(request.starts_with("OPTIONS / HTTP"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("origin: https://client.test")
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("access-control-request-method: put")
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("access-control-request-headers: content-type,x-example")
            );
            assert!(!request.contains(r#"{"value":1}"#));
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nAccess-Control-Max-Age: 60\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            assert!(request.starts_with("PUT / HTTP"));
            assert!(request.contains(r#"{"value":1}"#));
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: https://client.test\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });

    let response = runtime()
        .block_on(controller().fetch(request(server.url())))
        .unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    server.join();
    assert_eq!(sequence.load(Ordering::SeqCst), 2);
}

#[test]
fn successful_preflight_is_reused_until_the_controller_cache_is_cleared() {
    let preflights = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&preflights);
    let server = TestServer::start(5, move |request| {
        if request.starts_with("OPTIONS ") {
            observed.fetch_add(1, Ordering::SeqCst);
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nAccess-Control-Max-Age: 60\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: https://client.test\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let controller = controller();
    let original = request(server.url());

    runtime().block_on(async {
        for _ in 0..2 {
            controller
                .fetch(original.try_clone().unwrap())
                .await
                .unwrap()
                .body
                .bytes()
                .await
                .unwrap();
        }
        controller.clear_cache();
        controller
            .fetch(original)
            .await
            .unwrap()
            .body
            .bytes()
            .await
            .unwrap();
    });

    server.join();
    assert_eq!(preflights.load(Ordering::SeqCst), 2);
}

#[test]
fn denied_preflight_does_not_send_the_original_request() {
    let server = TestServer::start(1, |request| {
        assert!(request.starts_with("OPTIONS / HTTP"));
        b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Allow-Methods: DELETE\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
    });

    let error = runtime()
        .block_on(controller().fetch(request(server.url())))
        .unwrap_err();
    server.join();
    assert!(matches!(error, RequestError::CorsDenied));
}

#[test]
fn credentialed_preflight_requires_credential_approval_but_sends_no_cookie() {
    let server = TestServer::start(2, |request| {
        if request.starts_with("OPTIONS ") {
            assert!(!request.to_ascii_lowercase().contains("cookie:"));
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("cookie: session=secret")
            );
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Allow-Credentials: true\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(CookieServices))
        .build()
        .unwrap();
    let mut request = request(server.url());
    request.set_credentials_mode(net::CredentialsMode::Include);

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    server.join();
}

#[test]
fn opaque_origins_preflight_and_fetch_with_a_null_origin() {
    let server = TestServer::start(2, |request| {
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("origin: null"))
        );
        if request.starts_with("OPTIONS ") {
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: null\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: null\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let mut request = request(server.url());
    request.set_environment(FetchEnvironment {
        origin: Some(net::Origin::opaque()),
        ..FetchEnvironment::default()
    });

    let response = runtime().block_on(controller().fetch(request)).unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    server.join();
}

#[test]
fn noncredentialed_wildcards_authorize_unsafe_preflight_permissions() {
    let server = TestServer::start(2, |request| {
        if request.starts_with("OPTIONS ") {
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: *\r\nAccess-Control-Allow-Headers: *\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });

    let response = runtime()
        .block_on(controller().fetch(request(server.url())))
        .unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    server.join();
}

#[test]
fn authorization_cannot_be_approved_by_a_preflight_wildcard() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: *\r\nAccess-Control-Allow-Headers: *\r\nConnection: close\r\n\r\n".to_vec()
    });
    let mut request = request(server.url());
    request
        .set_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer test"),
        )
        .unwrap();

    let error = runtime().block_on(controller().fetch(request)).unwrap_err();
    server.join();
    assert!(matches!(error, RequestError::CorsDenied));
}

#[test]
fn cross_origin_same_origin_credentials_never_reach_cookie_services() {
    let server = TestServer::start(2, |request| {
        assert!(!request.to_ascii_lowercase().contains("cookie:"));
        if request.starts_with("OPTIONS ") {
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nSet-Cookie: ignored=preflight\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: https://client.test\r\nSet-Cookie: ignored=actual\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let reads = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(RecordingCookieServices {
            reads: Arc::clone(&reads),
            writes: Arc::clone(&writes),
        }))
        .build()
        .unwrap();

    let response = runtime()
        .block_on(controller.fetch(request(server.url())))
        .unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    server.join();
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}

#[test]
fn unsafe_redirect_target_receives_its_own_preflight() {
    let target = TestServer::start(2, |request| {
        if request.starts_with("OPTIONS ") {
            assert!(request.to_ascii_lowercase().contains("origin: null"));
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: null\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            assert!(request.starts_with("PUT / HTTP"));
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: null\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let target_url = target.url();
    let source = TestServer::start(2, move |request| {
        if request.starts_with("OPTIONS ") {
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_url}\r\nAccess-Control-Allow-Origin: https://client.test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .into_bytes()
        }
    });

    let response = runtime()
        .block_on(controller().fetch(request(source.url())))
        .unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    source.join();
    target.join();
}

#[test]
fn a_tainted_redirect_chain_preflights_each_target_with_null_origin() {
    let final_target = TestServer::start(2, |request| {
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("origin: null"))
        );
        if request.starts_with("OPTIONS ") {
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: null\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: null\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let final_url = final_target.url();
    let middle = TestServer::start(2, move |request| {
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("origin: null"))
        );
        if request.starts_with("OPTIONS ") {
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: null\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {final_url}\r\nAccess-Control-Allow-Origin: null\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .into_bytes()
        }
    });
    let middle_url = middle.url();
    let source = TestServer::start(2, move |request| {
        if request.starts_with("OPTIONS ") {
            assert!(
                request
                    .lines()
                    .any(|line| line.eq_ignore_ascii_case("origin: https://client.test"))
            );
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: https://client.test\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {middle_url}\r\nAccess-Control-Allow-Origin: https://client.test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .into_bytes()
        }
    });

    let response = runtime()
        .block_on(controller().fetch(request(source.url())))
        .unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    source.join();
    middle.join();
    final_target.join();
}

#[test]
fn returning_to_the_request_origin_remains_tainted_and_preflighted() {
    let source_state = Arc::new(Mutex::new(None::<(String, String)>));
    let middle_state = Arc::clone(&source_state);
    let middle = TestServer::start(2, move |request| {
        let (source_origin, final_url) = middle_state.lock().unwrap().clone().unwrap();
        assert!(
            request
                .lines()
                .any(|line| { line.eq_ignore_ascii_case(&format!("origin: {source_origin}")) })
        );
        if request.starts_with("OPTIONS ") {
            format!(
                "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {source_origin}\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n"
            )
            .into_bytes()
        } else {
            format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {final_url}\r\nAccess-Control-Allow-Origin: {source_origin}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .into_bytes()
        }
    });
    let middle_url = middle.url();
    let source = TestServer::start(3, move |request| {
        if request.starts_with("PUT /start ") {
            format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {middle_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .into_bytes()
        } else {
            assert!(
                request
                    .lines()
                    .any(|line| line.eq_ignore_ascii_case("origin: null"))
            );
            if request.starts_with("OPTIONS ") {
                b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: null\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: content-type, x-example\r\nConnection: close\r\n\r\n".to_vec()
            } else {
                b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: null\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
            }
        }
    });
    let source_origin = source.url();
    *source_state.lock().unwrap() = Some((source_origin.clone(), format!("{source_origin}/final")));
    let mut request = request(format!("{source_origin}/start"));
    request.set_environment(
        net::FetchEnvironment::from_url(reqwest::Url::parse(&source_origin).unwrap()).unwrap(),
    );

    let response = runtime().block_on(controller().fetch(request)).unwrap();
    assert_eq!(runtime().block_on(response.body.bytes()).unwrap(), b"ok");
    source.join();
    middle.join();
}
