//! Shared fakes and controller builders for scheme integration tests.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use net::{
    BlobProvider, CachePartitionService, Config, CookieService, FileProvider, Origin, PageProvider,
    ProviderFuture, Request, RequestController, RequestError, RequestPolicyService, ResponseInfo,
    ResponsePolicyService, SchemeProviders, ServiceWorkerDecision, ServiceWorkerService,
};

pub(crate) use crate::common::runtime;
pub(crate) use net::{ProviderErrorKind, ResponseType};

#[derive(Default)]
pub(crate) struct RecordingServices {
    pub(crate) request_checks: AtomicUsize,
    pub(crate) response_checks: AtomicUsize,
    pub(crate) cookie_lookups: AtomicUsize,
    pub(crate) cookie_stores: AtomicUsize,
    pub(crate) cache_partitions: AtomicUsize,
    pub(crate) worker_checks: AtomicUsize,
    pub(crate) response_origins: Mutex<Vec<Option<Origin>>>,
}

pub(crate) enum FailurePoint {
    Request,
    Response,
    Worker,
}

pub(crate) struct FailingServices(pub(crate) FailurePoint);

impl RequestPolicyService for FailingServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        if matches!(self.0, FailurePoint::Request) {
            Err(RequestError::InvalidResponse("request denied".into()))
        } else {
            Ok(())
        }
    }
}

impl ResponsePolicyService for FailingServices {
    fn check_response(&self, _request: &Request, _info: &ResponseInfo) -> Result<(), RequestError> {
        if matches!(self.0, FailurePoint::Response) {
            Err(RequestError::InvalidResponse("response denied".into()))
        } else {
            Ok(())
        }
    }
}

impl CookieService for FailingServices {
    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        panic!("local fetches must not request cookies")
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        panic!("local fetches must not store cookies")
    }
}

impl ServiceWorkerService for FailingServices {
    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        if matches!(self.0, FailurePoint::Worker) {
            Ok(ServiceWorkerDecision::Intercept)
        } else {
            Ok(ServiceWorkerDecision::Network)
        }
    }
}

impl CachePartitionService for FailingServices {
    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        panic!("local fetches must not request a cache partition")
    }
}

impl RequestPolicyService for RecordingServices {
    fn check_request(&self, _request: &Request) -> Result<(), RequestError> {
        self.request_checks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl ResponsePolicyService for RecordingServices {
    fn check_response(&self, _request: &Request, info: &ResponseInfo) -> Result<(), RequestError> {
        self.response_checks.fetch_add(1, Ordering::SeqCst);
        self.response_origins
            .lock()
            .unwrap()
            .push(info.response_origin.clone());
        Ok(())
    }
}

impl CookieService for RecordingServices {
    fn cookie_header(&self, _request: &Request) -> Result<Option<String>, RequestError> {
        self.cookie_lookups.fetch_add(1, Ordering::SeqCst);
        Ok(Some("session=unexpected".into()))
    }

    fn store_set_cookie(
        &self,
        _request: &Request,
        _values: &[net::HeaderValue],
    ) -> Result<(), RequestError> {
        self.cookie_stores.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl ServiceWorkerService for RecordingServices {
    fn service_worker(&self, _request: &Request) -> Result<ServiceWorkerDecision, RequestError> {
        self.worker_checks.fetch_add(1, Ordering::SeqCst);
        Ok(ServiceWorkerDecision::Network)
    }
}

impl CachePartitionService for RecordingServices {
    fn cache_partition(&self, _request: &Request) -> Result<String, RequestError> {
        self.cache_partitions.fetch_add(1, Ordering::SeqCst);
        Ok("local-scheme-test".into())
    }
}

/// A provider whose methods panic if dispatch reaches an unrelated scheme.
pub(crate) struct UnreachableProvider;

impl BlobProvider for UnreachableProvider {
    fn resolve<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        panic!("unexpected blob provider call")
    }
}

impl FileProvider for UnreachableProvider {
    fn open<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        panic!("unexpected file provider call")
    }
}

impl PageProvider for UnreachableProvider {
    fn resolve<'a>(&'a self, _request: &'a Request, _url: &'a net::Url) -> ProviderFuture<'a> {
        panic!("unexpected page provider call")
    }
}

pub(crate) fn controller_with_services_and_providers() -> (RequestController, Arc<RecordingServices>)
{
    let services = Arc::new(RecordingServices::default());
    let providers = SchemeProviders::new()
        .with_blob(Arc::new(UnreachableProvider))
        .with_file(Arc::new(UnreachableProvider))
        .with_page(Arc::new(UnreachableProvider));
    let controller = RequestController::builder(Config::default())
        .with_browser_capabilities(services.clone())
        .with_scheme_providers(providers)
        .build()
        .expect("controller should initialize");
    (controller, services)
}
