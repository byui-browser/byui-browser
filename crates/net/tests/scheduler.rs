//! Request scheduling and concurrency integration tests.

mod common;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use common::{TestServer, runtime};
use net::{
    AbortController, CachePartitionService, Config, CookieService, NetworkEvent, NetworkObserver,
    NetworkPhase, Request, RequestController, RequestError, RequestPolicyService, ResponseInfo,
    ResponsePolicyService, ServiceWorkerDecision, ServiceWorkerService,
};

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

struct AdmissionObserver {
    checks: AtomicUsize,
    notified: tokio::sync::Notify,
}

impl AdmissionObserver {
    fn new() -> Self {
        Self {
            checks: AtomicUsize::new(0),
            notified: tokio::sync::Notify::new(),
        }
    }
}

impl RequestPolicyService for AdmissionObserver {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.notified.notify_one();
        Ok(())
    }
}

impl ResponsePolicyService for AdmissionObserver {
    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        Ok(())
    }
}

impl CookieService for AdmissionObserver {
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

impl ServiceWorkerService for AdmissionObserver {
    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        Ok(ServiceWorkerDecision::Network)
    }
}

impl CachePartitionService for AdmissionObserver {
    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        Ok("test".into())
    }
}

#[test]
fn zero_in_flight_limit_still_allows_a_request() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller = RequestController::builder(Config {
        max_in_flight: 0,
        ..Config::default()
    })
    .build()
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
    let accepted = Arc::new(AtomicUsize::new(0));
    let (first_started_tx, first_started_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let release_first_rx = Arc::new(Mutex::new(release_first_rx));
    let observed_active = Arc::clone(&active);
    let observed_maximum = Arc::clone(&maximum);
    let observed_accepted = Arc::clone(&accepted);
    let server = TestServer::start(2, move |_| {
        let now = observed_active.fetch_add(1, Ordering::SeqCst) + 1;
        observed_maximum.fetch_max(now, Ordering::SeqCst);
        if observed_accepted.fetch_add(1, Ordering::SeqCst) == 0 {
            first_started_tx.send(()).unwrap();
            release_first_rx.lock().unwrap().recv().unwrap();
        }
        observed_active.fetch_sub(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
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
        let first_controller = controller.clone();
        let second_controller = controller.clone();
        let first_url = server.url();
        let second_url = server.url();
        let first = tokio::spawn(async move {
            let response = first_controller.fetch(Request::get(first_url)).await?;
            response.body.bytes().await
        });
        tokio::task::spawn_blocking(move || {
            first_started_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("first transport should reach the server")
        })
        .await
        .unwrap();

        let second_request = Request::get(second_url);
        let second_id = second_request.id();
        let second = tokio::spawn(async move {
            let response = second_controller.fetch(second_request).await?;
            response.body.bytes().await
        });
        observer
            .wait_for(second_id, NetworkPhase::QueueStarted)
            .await;
        tokio::task::yield_now().await;
        assert!(!observer.saw(second_id, NetworkPhase::QueueAdmitted));

        release_first_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            first.await.unwrap().unwrap();
            second.await.unwrap().unwrap();
        })
        .await
        .expect("both scheduled requests should complete");
    });

    server.join();
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
}

#[test]
fn aborting_a_queued_request_releases_admission_for_the_next_request() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let observer = Arc::new(AdmissionObserver::new());
    let controller = RequestController::builder(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .with_browser_capabilities(observer.clone())
    .build()
    .unwrap();

    runtime().block_on(async {
        let first = controller.fetch(Request::get(server.url())).await.unwrap();
        let abort = AbortController::new();
        let mut queued_request = Request::get(server.url());
        queued_request.set_signal(abort.signal());
        let queued_controller = controller.clone();
        let queued = tokio::spawn(async move { queued_controller.fetch(queued_request).await });

        while observer.checks.load(Ordering::SeqCst) < 2 {
            observer.notified.notified().await;
        }
        abort.abort();
        assert!(matches!(queued.await.unwrap(), Err(RequestError::Aborted)));

        drop(first);
        let response = controller.fetch(Request::get(server.url())).await.unwrap();
        assert_eq!(response.body.bytes().await.unwrap(), b"ok");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
