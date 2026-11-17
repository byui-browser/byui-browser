use std::sync::atomic::{AtomicU64, Ordering};

use http::Method;

use crate::api::{
    HeaderGuard, Headers, body::Body, cancellation::AbortSignal, error::RequestError,
};

use super::{
    CacheMode, FetchContext, InitiatorType, RedirectMode, Referrer, ReferrerPolicy, Request,
    RequestDestination, RequestPriority, ServiceWorkersMode,
};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

impl Request {
    /// Duplicates request metadata and an unused buffered body for an independent send.
    ///
    /// Live stream bodies require the original request to be sent instead of
    /// creating a second request: forking one would impose tee backpressure.
    pub fn try_clone(&self) -> Result<Self, RequestError> {
        if self.body.as_ref().is_some_and(Body::is_streaming_source) {
            return Err(RequestError::BodyNotCloneable);
        }
        let mut cloned = self.clone_without_body();
        cloned.body = self
            .body
            .as_ref()
            .map(Body::try_duplicate_buffered)
            .transpose()?;
        Ok(cloned)
    }

    /// Copies request configuration without transferring or duplicating its body.
    pub(crate) fn clone_without_body(&self) -> Self {
        Self {
            id: self.id,
            url: self.url.clone(),
            method: self.method.clone(),
            headers: self.headers.clone(),
            body: None,
            cache_mode: self.cache_mode,
            redirect_mode: self.redirect_mode,
            referrer: self.referrer.clone(),
            referrer_policy: self.referrer_policy,
            destination: self.destination,
            service_workers: self.service_workers,
            initiator: self.initiator,
            integrity: self.integrity.clone(),
            keepalive: self.keepalive,
            priority: self.priority,
            duplex: self.duplex,
            context: self.context.clone(),
            signal: self.signal.clone(),
            execution: super::state::RequestExecution {
                request_origin: self.execution.request_origin.clone(),
                url_list: self.execution.url_list.clone(),
                current_url: self.execution.current_url.clone(),
                redirect_count: self.execution.redirect_count,
                cache_partition: self.execution.cache_partition.clone(),
            },
        }
    }

    /// Captures request metadata for cache matching after transport finishes.
    pub(crate) fn for_cache(&self) -> Self {
        self.clone_without_body()
    }

    /// Creates a transport attempt, transferring a streaming body once.
    pub(crate) fn for_transport(&mut self) -> Result<Self, RequestError> {
        if self.body.as_ref().is_some_and(Body::is_streaming_source) {
            let mut attempt = self.clone_without_body();
            attempt.body = self.body.take();
            Ok(attempt)
        } else {
            self.try_clone()
        }
    }
    /// Converts a bodyless clone into the internal CORS preflight request.
    pub(crate) fn into_preflight(mut self) -> Self {
        self.method = Method::OPTIONS;
        self.headers = Headers::with_guard(HeaderGuard::None);
        self.body = None;
        self.context.mode = crate::RequestMode::Cors;
        self.context.credentials = crate::CredentialsMode::SameOrigin;
        self.cache_mode = CacheMode::NoStore;
        self.redirect_mode = RedirectMode::Error;
        self.integrity = None;
        self.keepalive = false;
        self
    }
    /// Creates a GET request with Fetch's default request state.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            id: NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
            url: url.into(),
            method: Method::GET,
            headers: Headers::new(),
            body: None,
            cache_mode: CacheMode::Default,
            redirect_mode: RedirectMode::Follow,
            referrer: Referrer::Client,
            referrer_policy: ReferrerPolicy::Empty,
            destination: RequestDestination::Empty,
            service_workers: ServiceWorkersMode::All,
            initiator: InitiatorType::Other,
            integrity: None,
            keepalive: false,
            priority: RequestPriority::Auto,
            duplex: None,
            context: FetchContext::default(),
            signal: AbortSignal::new(),
            execution: super::state::RequestExecution::default(),
        }
    }
    /// Creates a request after normalizing and validating its method.
    pub fn new(url: impl Into<String>, method: impl AsRef<str>) -> Result<Self, RequestError> {
        let mut request = Self::get(url);
        request.set_method(method)?;
        Ok(request)
    }
}
