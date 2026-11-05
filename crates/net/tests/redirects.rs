//! Redirect handling integration tests.

mod common;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use common::{TestServer, collect_body, runtime};
use net::{
    Body, Config, CredentialsMode, FetchEnvironment, FetchServices, RedirectMode, Request,
    RequestController, RequestError, ResponseInfo, ResponseType, ServiceWorkerDecision,
};

#[derive(Default)]
struct RedirectServices {
    request_urls: Mutex<Vec<String>>,
    responses: Mutex<Vec<(u16, ResponseType, usize)>>,
    stored_cookies: Mutex<Vec<Vec<String>>>,
    worker_checks: AtomicUsize,
}

impl FetchServices for RedirectServices {
    fn check_request(&self, request: &Request) -> Result<(), RequestError> {
        self.request_urls
            .lock()
            .unwrap()
            .push(request.active_url().to_owned());
        Ok(())
    }

    fn check_response(&self, _request: &Request, info: &ResponseInfo) -> Result<(), RequestError> {
        self.responses
            .lock()
            .unwrap()
            .push((info.status, info.response_type, info.redirect_count));
        Ok(())
    }

    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        Ok(None)
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        self.stored_cookies.lock().unwrap().push(
            values
                .iter()
                .map(|value| value.to_str().unwrap().to_owned())
                .collect(),
        );
        Ok(())
    }

    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        self.worker_checks.fetch_add(1, Ordering::SeqCst);
        Ok(ServiceWorkerDecision::Network)
    }

    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("redirect-tests".into())
    }
}

fn request_has_header(request: &str, name: &str) -> bool {
    request
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .any(|(header, _)| header.eq_ignore_ascii_case(name))
}

fn request_body(request: &str) -> &str {
    request.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}

#[test]
fn follow_mode_follows_redirect_and_exposes_final_response() {
    let server = TestServer::start(2, |request| {
        if request.starts_with("GET / HTTP") {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        } else {
            b"HTTP/1.1 302 Found\r\nLocation: /\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let response = runtime()
        .block_on(controller.fetch(Request::get(format!("{}/start", server.url()))))
        .unwrap();

    server.join();
    assert_eq!(response.status, 200);
    assert!(response.redirected);
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
}

#[test]
fn error_mode_rejects_redirect_before_exposing_headers() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: https://remote.test/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });

    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_redirect_mode(RedirectMode::Error);
    let error = runtime().block_on(controller.fetch(request)).unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::RedirectFailure));
}

#[test]
fn manual_mode_returns_an_opaque_redirect() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_redirect_mode(RedirectMode::Manual);

    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.response_type, net::ResponseType::OpaqueRedirect);
    assert_eq!(response.status, 0);
    assert!(response.url.is_empty());
}

#[test]
fn manual_mode_hides_a_non_http_redirect_target() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: data:text/plain,hidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_redirect_mode(RedirectMode::Manual);

    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.response_type, ResponseType::OpaqueRedirect);
    assert_eq!(response.status, 0);
    assert!(response.url.is_empty());
}

#[test]
fn redirect_status_without_location_is_an_ordinary_response() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let response = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap();

    server.join();
    assert_eq!(response.response_type, ResponseType::Basic);
    assert_eq!(response.status, 302);
    assert!(!response.redirected);
}

#[test]
fn post_redirect_rewrites_to_get_and_removes_body_headers() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let server = TestServer::start(2, move |request| {
        observed.lock().unwrap().push(request.to_owned());
        if request.starts_with("POST /start HTTP") {
            b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::new(format!("{}/start", server.url()), "POST").unwrap();
    request.set_body(Some(Body::from_text("payload"))).unwrap();

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
    server.join();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("POST /start HTTP"));
    assert!(request_has_header(&requests[0], "content-type"));
    assert_eq!(request_body(&requests[0]), "payload");
    assert!(requests[1].starts_with("GET /next HTTP"));
    assert!(!request_has_header(&requests[1], "content-type"));
    assert!(!request_has_header(&requests[1], "content-length"));
    assert!(request_body(&requests[1]).is_empty());
}

#[test]
fn cross_origin_redirect_strips_sensitive_headers() {
    let destination_requests = Arc::new(Mutex::new(Vec::new()));
    let observed_destination = Arc::clone(&destination_requests);
    let destination = TestServer::start(1, move |request| {
        observed_destination
            .lock()
            .unwrap()
            .push(request.to_owned());
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let location = format!("{}/next", destination.url());
    let source = TestServer::start(1, move |_| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::get(source.url());
    request
        .set_header(
            net::HeaderName::from_static("authorization"),
            net::HeaderValue::from_static("Bearer secret"),
        )
        .unwrap();

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
    source.join();
    destination.join();

    let requests = destination_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!request_has_header(&requests[0], "authorization"));
}

#[test]
fn replayable_body_follows_a_307_redirect_unchanged() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let server = TestServer::start(2, move |request| {
        observed.lock().unwrap().push(request.to_owned());
        if request.starts_with("POST /start HTTP") {
            b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::new(format!("{}/start", server.url()), "POST").unwrap();
    request.set_body(Some(Body::from_text("payload"))).unwrap();

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
    server.join();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].starts_with("POST /next HTTP"));
    assert_eq!(request_body(&requests[1]), "payload");
}

#[test]
fn redirect_statuses_apply_their_method_and_body_rules() {
    for (status, reason, method, expected_method, preserves_body) in [
        (301, "Moved Permanently", "POST", "GET", false),
        (303, "See Other", "PUT", "GET", false),
        (303, "See Other", "HEAD", "HEAD", false),
        (308, "Permanent Redirect", "POST", "POST", true),
    ] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        let server = TestServer::start(2, move |request| {
            observed.lock().unwrap().push(request.to_owned());
            if request.contains(" /start HTTP") {
                format!(
                    "HTTP/1.1 {status} {reason}\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .into_bytes()
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            }
        });
        let controller = RequestController::builder(Config::default())
            .build()
            .unwrap();
        let mut request = Request::new(format!("{}/start", server.url()), method).unwrap();
        if method != "HEAD" {
            request.set_body(Some(Body::from_text("payload"))).unwrap();
        }

        let response = runtime().block_on(controller.fetch(request)).unwrap();
        assert!(response.redirected, "status {status}");
        server.join();

        let requests = requests.lock().unwrap();
        assert!(
            requests[1].starts_with(&format!("{expected_method} /next HTTP")),
            "status {status}: {}",
            requests[1]
        );
        assert_eq!(
            request_body(&requests[1]) == "payload",
            preserves_body,
            "status {status}"
        );
        assert_eq!(
            request_has_header(&requests[1], "content-type"),
            preserves_body,
            "status {status}"
        );
    }
}

#[test]
fn non_replayable_body_cannot_follow_307_redirect() {
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&requests);
    let server = TestServer::start(1, move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::new(server.url(), "POST").unwrap();
    request
        .set_body(Some(Body::one_shot_bytes(b"payload".to_vec())))
        .unwrap();

    let error = runtime().block_on(controller.fetch(request)).unwrap_err();
    server.join();

    assert!(matches!(error, RequestError::RedirectFailure));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test]
fn non_replayable_body_cannot_follow_method_preserving_301_redirect() {
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&requests);
    let server = TestServer::start(1, move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();
    let mut request = Request::new(server.url(), "PUT").unwrap();
    request
        .set_body(Some(Body::one_shot_bytes(b"payload".to_vec())))
        .unwrap();

    let error = runtime().block_on(controller.fetch(request)).unwrap_err();
    server.join();

    assert!(matches!(error, RequestError::RedirectFailure));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test]
fn redirect_limit_fails_before_a_twenty_second_follow_up() {
    let server = TestServer::start(21, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let error = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap_err();
    server.join();

    assert!(matches!(error, RequestError::RedirectFailure));
}

#[test]
fn redirect_rejects_a_non_http_target_before_dispatch() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: data:text/plain,blocked\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let error = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::RedirectFailure));
}

#[test]
fn redirect_rejects_embedded_credentials_before_following() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: http://user:secret@127.0.0.1:9/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let error = runtime()
        .block_on(controller.fetch(Request::get(server.url())))
        .unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::RedirectFailure));
}

#[test]
fn each_redirect_response_runs_policy_and_cookie_processing() {
    let server = TestServer::start(2, |request| {
        if request.starts_with("GET /start HTTP") {
            b"HTTP/1.1 302 Found\r\nLocation: /next\r\nSet-Cookie: first=one\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nSet-Cookie: second=two\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let services = Arc::new(RedirectServices::default());
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(services.clone())
        .build()
        .unwrap();
    let mut request = Request::get(format!("{}/start", server.url()));
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );
    request.set_credentials_mode(CredentialsMode::Include);
    let expected_urls = vec![
        format!("{}/start", server.url()),
        format!("{}/next", server.url()),
    ];

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
    server.join();

    assert_eq!(
        *services.responses.lock().unwrap(),
        vec![(302, ResponseType::Basic, 0), (200, ResponseType::Basic, 1)]
    );
    assert_eq!(
        *services.stored_cookies.lock().unwrap(),
        vec![vec!["first=one".to_owned()], vec!["second=two".to_owned()]]
    );
    assert_eq!(services.request_urls.lock().unwrap().len(), 2);
    assert_eq!(*services.request_urls.lock().unwrap(), expected_urls);
    assert_eq!(services.worker_checks.load(Ordering::SeqCst), 1);
}

#[test]
fn manual_redirect_runs_common_approval_as_opaque_redirect() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });
    let services = Arc::new(RedirectServices::default());
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(services.clone())
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );
    request.set_redirect_mode(RedirectMode::Manual);

    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.response_type, ResponseType::OpaqueRedirect);
    assert_eq!(
        *services.responses.lock().unwrap(),
        vec![(302, ResponseType::OpaqueRedirect, 1)]
    );
}

#[test]
fn manual_cross_origin_redirect_is_opaque_without_cors_approval() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: https://remote.test/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let services = Arc::new(RedirectServices::default());
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(services.clone())
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("https://client.test/").unwrap()).unwrap(),
    );
    request.set_redirect_mode(RedirectMode::Manual);

    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.response_type, ResponseType::OpaqueRedirect);
    assert_eq!(
        *services.responses.lock().unwrap(),
        vec![(302, ResponseType::OpaqueRedirect, 1)]
    );
}

#[test]
fn error_redirect_does_not_run_response_policy_or_store_cookies() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: /next\r\nSet-Cookie: ignored=yes\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let services = Arc::new(RedirectServices::default());
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(services.clone())
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );
    request.set_credentials_mode(CredentialsMode::Include);
    request.set_redirect_mode(RedirectMode::Error);

    let error = runtime().block_on(controller.fetch(request)).unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::RedirectFailure));
    assert!(services.responses.lock().unwrap().is_empty());
    assert!(services.stored_cookies.lock().unwrap().is_empty());
}

#[test]
fn cors_rejects_an_unapproved_redirect_response_before_following() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(RedirectServices::default()))
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("https://client.test/").unwrap()).unwrap(),
    );

    let error = runtime().block_on(controller.fetch(request)).unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::CorsDenied));
}

#[test]
fn first_cross_origin_redirect_preserves_the_request_origin() {
    let destination = TestServer::start(1, |request| {
        assert!(request_has_header(request, "origin"));
        assert!(request.lines().any(|line| {
            line.to_ascii_lowercase()
                .starts_with("origin: http://127.0.0.1:")
        }));
        b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let location = format!("{}/next", destination.url());
    let source = TestServer::start(1, move |_| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(RedirectServices::default()))
        .build()
        .unwrap();
    let mut request = Request::get(source.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&source.url()).unwrap()).unwrap(),
    );

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");

    source.join();
    destination.join();
}

#[test]
fn redirect_taint_serializes_the_origin_as_null() {
    let destination = TestServer::start(1, |request| {
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("origin: null"))
        );
        b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: null\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let location = format!("{}/next", destination.url());
    let source = TestServer::start(1, move |_| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nAccess-Control-Allow-Origin: https://client.test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(RedirectServices::default()))
        .build()
        .unwrap();
    let mut request = Request::get(source.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse("https://client.test/").unwrap()).unwrap(),
    );

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");

    source.join();
    destination.join();
}

#[test]
fn redirect_response_referrer_policy_applies_to_the_next_hop() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let server = TestServer::start(2, move |request| {
        observed.lock().unwrap().push(request.to_owned());
        if request.starts_with("GET /start HTTP") {
            b"HTTP/1.1 302 Found\r\nLocation: /next\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(RedirectServices::default()))
        .build()
        .unwrap();
    let mut request = Request::get(format!("{}/start", server.url()));
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");
    server.join();

    let requests = requests.lock().unwrap();
    assert!(request_has_header(&requests[0], "referer"));
    assert!(!request_has_header(&requests[1], "referer"));
}

#[test]
fn post_to_get_redirect_reuses_the_target_cache_entry() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let server = TestServer::start(2, move |request| {
        observed.lock().unwrap().push(request.to_owned());
        if request.starts_with("GET /next HTTP") {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
        } else {
            b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    });
    let controller = RequestController::builder(Config::default())
        .build()
        .unwrap();

    let primed = runtime()
        .block_on(controller.fetch(Request::get(format!("{}/next", server.url()))))
        .unwrap();
    assert_eq!(runtime().block_on(collect_body(primed)).unwrap(), b"cached");
    let redirected = runtime()
        .block_on(
            controller.fetch(Request::new(format!("{}/start", server.url()), "POST").unwrap()),
        )
        .unwrap();
    assert!(redirected.redirected);
    assert!(redirected.from_cache);
    assert_eq!(
        runtime().block_on(collect_body(redirected)).unwrap(),
        b"cached"
    );

    server.join();
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[test]
fn same_origin_mode_blocks_a_cross_origin_redirect_before_transport() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(Arc::new(RedirectServices::default()))
        .build()
        .unwrap();
    let mut request = Request::get(server.url());
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );
    request.set_mode(net::RequestMode::SameOrigin).unwrap();

    let error = runtime().block_on(controller.fetch(request)).unwrap_err();

    server.join();
    assert!(matches!(error, RequestError::SameOriginViolation));
}

#[test]
fn network_redirects_bypass_service_worker_reinterception() {
    struct InterceptSecond(AtomicUsize);

    impl FetchServices for InterceptSecond {
        fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
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
            let call = self.0.fetch_add(1, Ordering::SeqCst);
            Ok(if call == 0 {
                ServiceWorkerDecision::Network
            } else {
                ServiceWorkerDecision::Intercept
            })
        }

        fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
            Ok("worker-redirect".into())
        }
    }

    let server = TestServer::start(2, |request| {
        if request.starts_with("GET /start HTTP") {
            b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        }
    });
    let services = Arc::new(InterceptSecond(AtomicUsize::new(0)));
    let controller = RequestController::builder(Config::default())
        .with_fetch_services(services.clone())
        .build()
        .unwrap();
    let mut request = Request::get(format!("{}/start", server.url()));
    request.set_environment(
        FetchEnvironment::from_url(reqwest::Url::parse(&server.url()).unwrap()).unwrap(),
    );

    let response = runtime().block_on(controller.fetch(request)).unwrap();
    assert_eq!(runtime().block_on(collect_body(response)).unwrap(), b"ok");

    server.join();
    assert_eq!(services.0.load(Ordering::SeqCst), 1);
}
