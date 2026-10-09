//! Public request orchestration boundary.

use std::sync::Arc;

use futures_util::{StreamExt, stream};

use crate::{
    cache::{ResponseCache, StoredResponse},
    config::Config,
    cors::CorsChecker,
    error::RequestError,
    policy::RequestPolicy,
    request::{
        CacheMode, CredentialsMode, InitiatorType, PreparedRequest, Request, RequestDestination,
        ServiceWorkersMode,
    },
    response::{InternalResponse, Response, ResponseBody, StreamingResponse},
    scheduler::RequestScheduler,
    services::{FetchServices, ResponseInfo, ServiceWorkerDecision},
    transport::reqwest_transport,
};

/// Shared, stateful entry point for browser network requests.
///
/// A controller owns the process-local cache and scheduler while sharing the
/// connection pool held by its transport. Cloning a controller is therefore
/// inexpensive and preserves cache and connection reuse. Use `with_services`
/// for requests carrying a browser client origin; `new` is for standalone
/// requests without browser-owned policy or storage context.
#[derive(Clone)]
pub struct RequestController {
    inner: Arc<ControllerInner>,
}

struct ControllerInner {
    /// Maximum accepted body size for keepalive requests.
    max_keepalive_body_size: u64,
    /// Process-local response cache shared by controller clones.
    cache: ResponseCache,
    /// Browser-owned policy, credential, cache-partition, and worker decisions.
    services: Option<Arc<dyn FetchServices>>,
    /// Cross-origin response validation boundary.
    cors: CorsChecker,
    /// Request and response policy validation boundary.
    policy: RequestPolicy,
    /// Shared transport admission and concurrency controller.
    scheduler: RequestScheduler,
}

impl std::fmt::Debug for RequestController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestController").finish_non_exhaustive()
    }
}

impl RequestController {
    /// Creates a controller with the supplied transport, cache, and scheduling
    /// configuration.
    ///
    /// The controller does not perform network I/O during construction. A
    /// [`RequestError::Initialization`] is returned if the underlying HTTP
    /// client cannot be initialized.
    pub fn new(config: Config) -> Result<Self, RequestError> {
        Self::build(config, None)
    }

    /// Creates a controller with browser-owned service decisions.
    ///
    /// The service is shared by controller clones and must own any durable
    /// storage or policy data outside this process-local Fetch engine.
    pub fn with_services(
        config: Config,
        services: Arc<dyn FetchServices>,
    ) -> Result<Self, RequestError> {
        Self::build(config, Some(services))
    }

    fn build(
        config: Config,
        services: Option<Arc<dyn FetchServices>>,
    ) -> Result<Self, RequestError> {
        let max_in_flight = config.max_in_flight;
        let max_keepalive_body_size = config.max_keepalive_body_size;
        let transport = reqwest_transport(config.clone())
            .map_err(|error| RequestError::Initialization(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(ControllerInner {
                max_keepalive_body_size,
                cache: ResponseCache::default(),
                services,
                cors: CorsChecker,
                policy: RequestPolicy,
                scheduler: RequestScheduler::new(transport, max_in_flight),
            }),
        })
    }

    /// Validates and executes one browser request.
    ///
    /// Cache hits return before a scheduler permit is acquired. Network-bound
    /// requests pass through policy validation, cookie attachment, scheduling,
    /// transport, response validation, cookie processing, and cache insertion.
    pub async fn fetch(&self, request: Request) -> Result<Response, RequestError> {
        // Reuses the streaming fetch implementation by streaming the response and then buffering it
        // into a fully collected response.
        let mut streaming = self.fetch_stream(request).await?;
        let mut body = Vec::new();
        while let Some(chunk) = streaming.body.next().await {
            body.extend_from_slice(&chunk?);
        }

        Ok(Response {
            response_type: streaming.response_type,
            status: streaming.status,
            status_text: streaming.status_text,
            headers: streaming.headers,
            url: streaming.url,
            redirected: streaming.redirected,
            body,
            from_cache: streaming.from_cache,
        })
    }

    /// Validates and executes one browser request without buffering its body.
    ///
    /// The returned headers are available before the body is consumed. Body
    /// chunks are delivered as the transport receives them. A network response
    /// with bytes is inserted into the cache only after the caller consumes the
    /// stream to completion; dropped or failed streams are not cached. Null
    /// bodies may be cached as soon as headers arrive.
    pub async fn fetch_stream(
        &self,
        mut request: Request,
    ) -> Result<StreamingResponse, RequestError> {
        if request.signal.is_aborted() {
            return Err(RequestError::Aborted);
        }
        let url = self
            .inner
            .policy
            .validate_request(&request, self.inner.max_keepalive_body_size)?;
        request.prepare_url(url.clone());

        if let Some(services) = &self.inner.services {
            services.check_request(&request)?;
            if request.service_workers == ServiceWorkersMode::All
                && services.service_worker(&request)? == ServiceWorkerDecision::Intercept
            {
                return Err(RequestError::UnsupportedFeature(
                    "service-worker interception",
                ));
            }
        } else if request.context.environment.origin.is_some()
            || request.context.credentials == CredentialsMode::Include
            || request.destination != RequestDestination::Empty
            || request.initiator != InitiatorType::Other
        {
            return Err(RequestError::UnsupportedFeature("browser services"));
        }

        let cacheable = request.is_cacheable_method();
        if cacheable && request.cache_mode != CacheMode::NoStore {
            if let Some(services) = &self.inner.services {
                request.cache_partition = Some(services.cache_partition(&request)?);
            }
        }
        let cache_lookup = match request.cache_mode {
            CacheMode::Default | CacheMode::OnlyIfCached => Some(false),
            CacheMode::ForceCache => Some(true),
            CacheMode::NoStore | CacheMode::Reload | CacheMode::NoCache => None,
        };
        if cacheable {
            if let Some(allow_stale) = cache_lookup {
                if let Some(response) =
                    self.inner
                        .cache
                        .get_with_staleness(&request, &url, allow_stale)
                {
                    let origin = reqwest::Url::parse(&response.url)
                        .ok()
                        .and_then(|url| crate::Origin::from_url(&url).ok());
                    // Return information about the response to the devtools and other observers, but do not allow the body to be consumed until the caller polls it.
                    let mut internal = InternalResponse {
                        status: response.status,
                        status_text: response
                            .status
                            .canonical_reason()
                            .unwrap_or_default()
                            .to_owned(),
                        headers: response.headers,
                        url_list: vec![response.url],
                        redirect_count: 0,
                        origin,
                        request_origin: request.context.environment.origin.clone(),
                        request_mode: request.context.mode,
                        response_type: crate::response::ResponseType::Basic,
                        body: ResponseBody::once_with_signal(response.body, request.signal.clone()),
                        body_is_null: request.method() == reqwest::Method::HEAD
                            || matches!(response.status.as_u16(), 101 | 204 | 205 | 304),
                        from_cache: true,
                        cookie_headers_processed: response.cookie_headers_processed,
                    };
                    internal.response_type = self.inner.cors.response_type(&request, &internal)?;
                    self.approve_response(&request, &internal)?;
                    return Ok(internal.expose());
                }
            }
            if request.cache_mode == CacheMode::OnlyIfCached {
                return Err(RequestError::NetworkError);
            }
        }

        // Cookie and policy modules operate before transport sees the request;
        // this keeps browser behavior out of the low-level HTTP implementation.
        let mut request = request;
        request.apply_fetch_headers()?;
        if request.credentials_allowed(&url) {
            if let Some(services) = &self.inner.services {
                if let Some(cookie) = services.cookie_header(&request)? {
                    let value = http::HeaderValue::from_str(&cookie)
                        .map_err(|_| RequestError::ForbiddenHeader("cookie".into()))?;
                    request
                        .headers
                        .insert_internal_header(http::header::COOKIE, value);
                }
            }
        }
        // The scheduler owns concurrency admission. Transport remains focused
        // on HTTP I/O and connection pooling.
        let mut response = self
            .inner
            .scheduler
            .submit(
                PreparedRequest {
                    request: request.clone(),
                    url: url.clone(),
                },
                request.transport_priority(),
            )
            .await?;

        if matches!(response.status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Err(RequestError::RedirectFailure);
        }
        response.origin = response
            .url_list
            .last()
            .and_then(|url| reqwest::Url::parse(url).ok())
            .and_then(|url| crate::Origin::from_url(&url).ok());
        response.response_type = self.inner.cors.response_type(&request, &response)?;
        self.inner.policy.validate_response(&request, &response)?;
        self.approve_response(&request, &response)?;
        if request.credentials_allowed(&url) {
            if let Some(services) = &self.inner.services {
                let values: Vec<http::HeaderValue> = response
                    .headers
                    .get_all(http::header::SET_COOKIE)
                    .iter()
                    .cloned()
                    .collect();
                services.store_set_cookie(&request, &values)?;
                response.cookie_headers_processed = true;
            }
        }

        if cacheable && request.cache_mode != CacheMode::NoStore {
            if response.body_is_null {
                self.inner.cache.insert(
                    &request,
                    &url,
                    &StoredResponse {
                        status: response.status,
                        headers: response.headers.clone(),
                        url: response.url_list.last().cloned().unwrap_or_default(),
                        body: Vec::new(),
                        cookie_headers_processed: response.cookie_headers_processed,
                    },
                );
                return Ok(response.expose());
            }
            response = with_cache_capture(response, self.inner.cache.clone(), request.clone(), url);
        }
        Ok(response.expose())
    }

    fn approve_response(
        &self,
        request: &Request,
        response: &InternalResponse,
    ) -> Result<(), RequestError> {
        if let Some(services) = &self.inner.services {
            let info = ResponseInfo {
                status: response.status.as_u16(),
                status_text: response.status_text.clone(),
                url: response.url_list.last().cloned().unwrap_or_default(),
                headers: crate::HeaderList::internal_response(&response.headers),
                response_origin: response.origin.clone(),
                request_origin: response.request_origin.clone(),
                request_mode: response.request_mode,
                response_type: response.response_type,
                redirect_count: response.redirect_count,
                from_cache: response.from_cache,
            };
            services.check_response(request, &info)?;
            services.response_headers(request, &info);
        }
        Ok(())
    }

    /// Removes all currently stored responses from this controller's cache.
    pub fn clear_cache(&self) {
        self.inner.cache.clear();
    }
}

struct CacheCapture {
    /// Cache receiving the body after successful end-of-stream.
    cache: ResponseCache,
    /// Original request used to compute the cache key.
    request: Request,
    /// Parsed request URL used to compute the cache key.
    request_url: reqwest::Url,
    /// Response metadata retained while the body is consumed.
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    url: String,
    cookie_headers_processed: bool,
}

/// Wraps a response body so completed network responses are copied into cache.
///
/// Bytes are forwarded immediately to the caller while a second copy is
/// collected. Any body error abandons the capture, and only a clean end of
/// stream inserts the complete response.
fn with_cache_capture(
    mut response: InternalResponse,
    cache: ResponseCache,
    request: Request,
    request_url: reqwest::Url,
) -> InternalResponse {
    let status = response.status;
    let headers = response.headers.clone();
    let url = response.url_list.last().cloned().unwrap_or_default();
    let (body, permit) = response.body.into_parts();
    let capture = CacheCapture {
        cache,
        request,
        request_url,
        status,
        headers,
        url,
        cookie_headers_processed: response.cookie_headers_processed,
    };
    let body = stream::unfold(
        (body, Vec::new(), Some(capture)),
        |(mut body, mut collected, capture)| async move {
            match body.next().await {
                Some(Ok(chunk)) => {
                    collected.extend_from_slice(&chunk);
                    Some((Ok(chunk), (body, collected, capture)))
                }
                Some(Err(error)) => Some((Err(error), (body, collected, None))),
                None => {
                    if let Some(capture) = capture {
                        capture.cache.insert(
                            &capture.request,
                            &capture.request_url,
                            &StoredResponse {
                                status: capture.status,
                                headers: capture.headers,
                                url: capture.url,
                                body: collected,
                                cookie_headers_processed: capture.cookie_headers_processed,
                            },
                        );
                    }
                    None
                }
            }
        },
    );
    let mut body = ResponseBody::from_stream(body);
    if let Some(permit) = permit {
        body.attach_permit(permit);
    }
    response.body = body;
    response
}
