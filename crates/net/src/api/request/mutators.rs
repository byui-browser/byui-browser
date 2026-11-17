use http::{
    Method,
    header::{HeaderName, HeaderValue},
};

use crate::api::{HeaderGuard, body::Body, cancellation::AbortSignal, error::RequestError};

use super::{
    CacheMode, CredentialsMode, FetchEnvironment, InitiatorType, RedirectMode, Referrer,
    ReferrerPolicy, Request, RequestDestination, RequestDuplex, RequestMode, RequestPriority,
    ServiceWorkersMode,
};

impl Request {
    /// Replaces the method after Fetch token and forbidden-method validation.
    pub fn set_method(&mut self, method: impl AsRef<str>) -> Result<(), RequestError> {
        let method = method.as_ref();
        let parsed = Method::from_bytes(method.as_bytes())
            .map_err(|_| RequestError::InvalidMethod(method.to_owned()))?;
        let normalized = match parsed.as_str().to_ascii_uppercase().as_str() {
            "DELETE" => Method::DELETE,
            "GET" => Method::GET,
            "HEAD" => Method::HEAD,
            "OPTIONS" => Method::OPTIONS,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            _ => parsed,
        };
        if matches!(normalized.as_str(), "CONNECT" | "TRACE" | "TRACK") {
            log::debug!(
                "forbidden method {normalized} rejected for request {}",
                self.id
            );
            return Err(RequestError::ForbiddenMethod(normalized.to_string()));
        }
        if self.context.mode == RequestMode::NoCors
            && !matches!(normalized, Method::GET | Method::HEAD | Method::POST)
        {
            return Err(RequestError::NoCorsMethod(normalized.to_string()));
        }
        self.method = normalized;
        Ok(())
    }
    /// Appends a caller-controlled request header under the active Fetch guard.
    pub fn append_header(
        &mut self,
        name: HeaderName,
        value: HeaderValue,
    ) -> Result<(), RequestError> {
        self.headers.append(name, value)
    }
    /// Replaces caller-controlled values for a header under the active Fetch guard.
    pub fn set_header(&mut self, name: HeaderName, value: HeaderValue) -> Result<(), RequestError> {
        self.headers.insert(name, value)
    }
    /// Changes request mode and applies the corresponding request-header guard.
    pub fn set_mode(&mut self, mode: RequestMode) -> Result<(), RequestError> {
        if mode == RequestMode::NoCors
            && !matches!(self.method, Method::GET | Method::HEAD | Method::POST)
        {
            return Err(RequestError::NoCorsMethod(self.method.to_string()));
        }
        self.headers.set_guard(if mode == RequestMode::NoCors {
            HeaderGuard::RequestNoCors
        } else {
            HeaderGuard::Request
        })?;
        self.context.mode = mode;
        Ok(())
    }
    /// Sets the credentials mode used by cookie and authentication policy.
    pub fn set_credentials_mode(&mut self, mode: CredentialsMode) {
        self.context.credentials = mode;
    }
    /// Sets the initiating client environment.
    pub fn set_environment(&mut self, environment: FetchEnvironment) {
        self.context.environment = environment;
    }
    /// Sets or clears the body and derives its content type when absent.
    pub fn set_body(&mut self, body: Option<Body>) -> Result<(), RequestError> {
        if body.is_some() && matches!(self.method, Method::GET | Method::HEAD) {
            log::debug!("body rejected for {} request {}", self.method, self.id);
            return Err(RequestError::BodyNotAllowed(self.method.to_string()));
        }
        self.body = body;
        Ok(())
    }
    /// Sets the Fetch cache mode.
    pub fn set_cache_mode(&mut self, mode: CacheMode) {
        self.cache_mode = mode;
    }
    /// Sets the Fetch redirect mode.
    ///
    /// `Follow` executes approved HTTP(S) hops, `Error` fails after receiving a
    /// redirect response, and `Manual` returns an opaque-redirect view.
    pub fn set_redirect_mode(&mut self, mode: RedirectMode) {
        self.redirect_mode = mode;
    }
    /// Sets the referrer source and reduction policy.
    pub fn set_referrer(&mut self, referrer: Referrer, policy: ReferrerPolicy) {
        self.referrer = referrer;
        self.referrer_policy = policy;
    }
    /// Sets the request destination used by browser policy integrations.
    pub fn set_destination(&mut self, destination: RequestDestination) {
        self.destination = destination;
    }
    /// Sets whether service workers may intercept this request.
    pub fn set_service_workers_mode(&mut self, mode: ServiceWorkersMode) {
        self.service_workers = mode;
    }
    /// Sets the source that initiated this request.
    pub fn set_initiator(&mut self, initiator: InitiatorType) {
        self.initiator = initiator;
    }
    /// Sets optional subresource integrity metadata.
    pub fn set_integrity(&mut self, integrity: Option<String>) {
        self.integrity = integrity;
    }
    /// Sets the scheduler priority for this request.
    pub fn set_priority(&mut self, priority: RequestPriority) {
        self.priority = priority;
    }
    /// Selects the request streaming mode required for a streaming body.
    pub fn set_duplex(&mut self, duplex: Option<RequestDuplex>) {
        self.duplex = duplex;
    }
    /// Sets the cancellation signal.
    pub fn set_signal(&mut self, signal: AbortSignal) {
        self.signal = signal;
    }
    /// Requests a Fetch keepalive lifetime.
    /// The engine checks the configured byte limit but currently rejects keepalive execution with `UnsupportedFeature` before network I/O.
    pub fn set_keepalive(&mut self, keepalive: bool) {
        self.keepalive = keepalive;
    }
}
