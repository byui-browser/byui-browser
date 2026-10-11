use http::{
    Method,
    header::{CONTENT_TYPE, HeaderName, HeaderValue},
};
use url::Url;

use crate::api::{
    HeaderGuard, Headers, body::Body, cancellation::AbortSignal, error::RequestError,
};

use super::{
    CacheMode, CredentialsMode, FetchContext, FetchEnvironment, InitiatorType, Origin,
    RedirectMode, Referrer, ReferrerPolicy, RequestDestination, RequestMode, ServiceWorkersMode,
};
use crate::api::headers::is_cors_safelisted_header;

/// A browser request before conversion to a transport request.
#[derive(Clone, Debug)]
pub struct Request {
    pub(crate) url: String,
    pub(crate) method: Method,
    pub(crate) headers: Headers,
    pub(crate) body: Option<Body>,
    pub(crate) cache_mode: CacheMode,
    pub(crate) redirect_mode: RedirectMode,
    pub(crate) referrer: Referrer,
    pub(crate) referrer_policy: ReferrerPolicy,
    pub(crate) destination: RequestDestination,
    pub(crate) service_workers: ServiceWorkersMode,
    pub(crate) initiator: InitiatorType,
    pub(crate) integrity: Option<String>,
    pub(crate) keepalive: bool,
    pub(crate) priority: RequestPriority,
    pub(crate) context: FetchContext,
    pub(crate) signal: AbortSignal,
    pub(crate) url_list: Vec<Url>,
    pub(crate) current_url: Option<Url>,
    pub(crate) redirect_count: usize,
    pub(crate) cache_partition: Option<String>,
}

/// Relative importance assigned to a request by the network scheduler.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum RequestPriority {
    /// Background work.
    Low,
    #[default]
    /// Ordinary network work.
    Auto,
    /// User-visible work.
    High,
}

impl Request {
    /// Creates a GET request with Fetch's default request state.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
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
            context: FetchContext::default(),
            signal: AbortSignal::new(),
            url_list: Vec::new(),
            current_url: None,
            redirect_count: 0,
            cache_partition: None,
        }
    }
    /// Creates a request after normalizing and validating its method.
    pub fn new(url: impl Into<String>, method: impl AsRef<str>) -> Result<Self, RequestError> {
        let mut request = Self::get(url);
        request.set_method(method)?;
        Ok(request)
    }
    /// Returns the original URL string supplied for this request.
    pub fn url(&self) -> &str {
        &self.url
    }
    /// Returns the normalized HTTP method.
    pub fn method(&self) -> &Method {
        &self.method
    }
    /// Returns the guarded request header list.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }
    /// Returns this request's optional body.
    pub fn body(&self) -> Option<&Body> {
        self.body.as_ref()
    }
    /// Returns the resolved current URL once request preparation has run.
    pub fn current_url(&self) -> Option<&Url> {
        self.current_url.as_ref()
    }
    /// Returns the configured request mode.
    pub fn mode(&self) -> RequestMode {
        self.context.mode
    }
    /// Returns the configured credentials mode.
    pub fn credentials_mode(&self) -> CredentialsMode {
        self.context.credentials
    }
    /// Returns the client environment used to derive URL and origin state.
    pub fn environment(&self) -> &FetchEnvironment {
        &self.context.environment
    }
    /// Returns all URLs visited by this request, in order.
    /// The supported single-exchange path never follows a redirect.
    pub fn url_list(&self) -> &[Url] {
        &self.url_list
    }
    /// Returns the number of redirects followed so far.
    /// This remains zero until controller-owned redirects are implemented.
    pub fn redirect_count(&self) -> usize {
        self.redirect_count
    }
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
    /// `Manual` and `Error` are rejected before network I/O in this slice.
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
    /// Sets the cancellation signal.
    pub fn set_signal(&mut self, signal: AbortSignal) {
        self.signal = signal;
    }
    /// Requests a Fetch keepalive lifetime.
    /// The engine checks the configured byte limit but currently rejects keepalive execution with `UnsupportedFeature` before network I/O.
    pub fn set_keepalive(&mut self, keepalive: bool) {
        self.keepalive = keepalive;
    }
    /// Returns whether this request method may use the response cache.
    pub(crate) fn is_cacheable_method(&self) -> bool {
        matches!(self.method, Method::GET | Method::HEAD)
    }
    /// Returns whether the credentials mode permits credentials for this URL.
    pub(crate) fn credentials_allowed(&self, url: &Url) -> bool {
        match self.context.credentials {
            CredentialsMode::Omit => false,
            CredentialsMode::Include => true,
            CredentialsMode::SameOrigin => {
                self.context
                    .environment
                    .origin
                    .as_ref()
                    .is_some_and(|origin| {
                        Origin::from_url(url).is_ok_and(|target| origin.is_same_origin(&target))
                    })
            }
        }
    }
    pub(crate) fn requires_cors_preflight(&self) -> bool {
        !matches!(self.method, Method::GET | Method::HEAD | Method::POST)
            || self
                .headers
                .iter_raw()
                .any(|(name, value)| !is_cors_safelisted_header(name, value))
    }
    /// Returns the transport priority corresponding to the Fetch priority.
    pub(crate) fn transport_priority(&self) -> crate::scheduling::RequestPriority {
        match self.priority {
            RequestPriority::Low => crate::scheduling::RequestPriority::Low,
            RequestPriority::Auto => crate::scheduling::RequestPriority::Normal,
            RequestPriority::High => crate::scheduling::RequestPriority::High,
        }
    }
    pub(crate) fn prepare_url(&mut self, url: Url) {
        if self.url_list.is_empty() {
            self.url_list.push(url.clone());
        }
        self.current_url = Some(url);
    }
    /// Builds the internal transport header channel without changing caller headers.
    pub(crate) fn fetch_headers(&self) -> Result<Headers, RequestError> {
        let mut headers = self.headers.clone();
        if let Some(body) = &self.body {
            body.apply_metadata(&mut headers)?;
        }
        let needs_origin = self.context.mode == RequestMode::Cors
            || !matches!(self.method, Method::GET | Method::HEAD);
        if needs_origin && let Some(origin) = &self.context.environment.origin {
            let value = HeaderValue::try_from(origin.as_str())
                .map_err(|_| RequestError::InvalidOrigin(origin.as_str()))?;
            headers.insert_internal_header(HeaderName::from_static("origin"), value)?;
        }
        Ok(headers)
    }
    /// Computes the `Referer` value for a request target.
    pub(crate) fn referrer_value(&self, target: &Url) -> Option<String> {
        let source = match &self.referrer {
            Referrer::NoReferrer => return None,
            Referrer::Client => self.context.environment.referrer.as_ref()?,
            Referrer::Url(source) => source,
        };
        let same_origin = source.origin() == target.origin();
        let downgrade = source.scheme() == "https" && target.scheme() == "http";
        let source_url = source.as_str().to_owned();
        let source_origin = source.origin().ascii_serialization();
        let policy = match self.referrer_policy {
            ReferrerPolicy::Empty => ReferrerPolicy::StrictOriginWhenCrossOrigin,
            policy => policy,
        };
        match policy {
            ReferrerPolicy::UnsafeUrl => Some(source_url),
            ReferrerPolicy::NoReferrer => None,
            ReferrerPolicy::NoReferrerWhenDowngrade if downgrade => None,
            ReferrerPolicy::NoReferrerWhenDowngrade => Some(source_url),
            ReferrerPolicy::Origin => Some(source_origin),
            ReferrerPolicy::OriginWhenCrossOrigin if same_origin => Some(source_url),
            ReferrerPolicy::OriginWhenCrossOrigin => Some(source_origin),
            ReferrerPolicy::SameOrigin if same_origin => Some(source_url),
            ReferrerPolicy::SameOrigin => None,
            ReferrerPolicy::StrictOrigin if downgrade => None,
            ReferrerPolicy::StrictOrigin => Some(source_origin),
            ReferrerPolicy::StrictOriginWhenCrossOrigin if downgrade => None,
            ReferrerPolicy::StrictOriginWhenCrossOrigin if same_origin => Some(source_url),
            ReferrerPolicy::StrictOriginWhenCrossOrigin => Some(source_origin),
            ReferrerPolicy::Empty => Some(source_origin),
        }
    }
    pub(crate) fn validate_body_state(
        &self,
        max_keepalive_body_size: u64,
    ) -> Result<(), RequestError> {
        if self.body.is_some() && matches!(self.method, Method::GET | Method::HEAD) {
            return Err(RequestError::BodyNotAllowed(self.method.to_string()));
        }
        if self.context.mode == RequestMode::NoCors
            && let Some(body) = &self.body
            && let Some(content_type) = body.content_type()
            && !is_cors_safelisted_header(&CONTENT_TYPE, &content_type)
        {
            return Err(RequestError::ForbiddenHeader(
                CONTENT_TYPE.as_str().to_owned(),
            ));
        }
        if self.keepalive {
            let body_size = match &self.body {
                None => Some(0),
                Some(body) => body.length(),
            };
            if body_size.is_none_or(|size| size > max_keepalive_body_size) {
                return Err(RequestError::KeepaliveBodyTooLarge {
                    limit: max_keepalive_body_size,
                });
            }
        }
        Ok(())
    }
}
