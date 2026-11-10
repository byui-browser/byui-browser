//! CORS-preflight request and cache integration tests.

mod common;

use std::sync::{
    Arc,
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
