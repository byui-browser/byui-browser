//! Public request orchestration boundary.

use std::sync::Arc;

use futures_util::{StreamExt, stream};

use crate::{
    cache::ResponseCache,
    config::Config,
    cookies::CookieStore,
    cors::CorsChecker,
    error::RequestError,
    policy::RequestPolicy,
    request::{CacheMode, PreparedRequest, Request},
    response::{Response, ResponseBody, StreamingResponse},
    scheduler::{RequestPriority, RequestScheduler},
    transport::reqwest_transport,
};

/// Shared, stateful entry point for browser network requests.
///
/// A controller owns the process-local cache and scheduler while sharing the
/// connection pool held by its transport. Cloning a controller is therefore
/// inexpensive and preserves cache and connection reuse.
#[derive(Clone)]
pub struct RequestController {
    inner: Arc<ControllerInner>,
}

struct ControllerInner {
    /// Process-local response cache shared by controller clones.
    cache: ResponseCache,
    /// Cookie attachment and response processing boundary.
    cookies: CookieStore,
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
    /// transport-construction error is returned if the underlying HTTP client
    /// cannot be initialized.
    pub fn new(config: Config) -> Result<Self, reqwest::Error> {
        let transport = reqwest_transport(config.clone())?;
        Ok(Self {
            inner: Arc::new(ControllerInner {
                cache: ResponseCache::default(),
                cookies: CookieStore,
                cors: CorsChecker,
                policy: RequestPolicy,
                scheduler: RequestScheduler::new(transport, config.max_in_flight),
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
            status: streaming.status,
            headers: streaming.headers,
            url: streaming.url,
            body,
            from_cache: streaming.from_cache,
        })
    }

    /// Validates and executes one browser request without buffering its body.
    ///
    /// The returned headers are available before the body is consumed. Body
    /// chunks are delivered as the transport receives them. A network response
    /// is inserted into the cache only after the caller consumes the stream to
    /// completion; dropped or failed streams are not cached.
    pub async fn fetch_stream(&self, request: Request) -> Result<StreamingResponse, RequestError> {
        let url = self.inner.policy.validate_request(&request)?;

        let cacheable = request.is_cacheable_method();
        if cacheable
            && matches!(
                request.cache_mode,
                CacheMode::Default | CacheMode::OnlyIfCached
            )
        {
            if let Some(response) = self.inner.cache.get(&request, &url) {
                return Ok(StreamingResponse {
                    status: response.status,
                    headers: response.headers,
                    url: response.url,
                    body: ResponseBody::once(response.body),
                    from_cache: true,
                });
            }
            if request.cache_mode == CacheMode::OnlyIfCached {
                return Err(RequestError::CacheMiss);
            }
        }

        // Cookie and policy modules operate before transport sees the request;
        // this keeps browser behavior out of the low-level HTTP implementation.
        let mut request = request;
        self.inner.cookies.attach(&mut request)?;
        // The scheduler owns concurrency admission. Transport remains focused
        // on HTTP I/O and connection pooling.
        let response = self
            .inner
            .scheduler
            .submit(
                PreparedRequest {
                    request: request.clone(),
                    url: url.clone(),
                },
                RequestPriority::Normal,
            )
            .await?;

        let response_metadata = Response {
            status: response.status,
            headers: response.headers.clone(),
            url: response.url.clone(),
            body: Vec::new(),
            from_cache: false,
        };
        self.inner.cors.validate(&request, &response_metadata)?;
        self.inner
            .policy
            .validate_response(&request, &response_metadata)?;
        self.inner
            .cookies
            .process_response(&request, &response.headers)?;

        if cacheable && request.cache_mode != CacheMode::NoStore {
            return Ok(with_cache_capture(
                response,
                self.inner.cache.clone(),
                request,
                url,
            ));
        }
        Ok(response)
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
}

/// Wraps a response body so completed network responses are copied into cache.
///
/// Bytes are forwarded immediately to the caller while a second copy is
/// collected. Any body error abandons the capture, and only a clean end of
/// stream inserts the complete response.
fn with_cache_capture(
    mut response: StreamingResponse,
    cache: ResponseCache,
    request: Request,
    request_url: reqwest::Url,
) -> StreamingResponse {
    let status = response.status;
    let headers = response.headers.clone();
    let url = response.url.clone();
    let (body, permit) = response.body.into_parts();
    let capture = CacheCapture {
        cache,
        request,
        request_url,
        status,
        headers,
        url,
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
                            &Response {
                                status: capture.status,
                                headers: capture.headers,
                                url: capture.url,
                                body: collected,
                                from_cache: false,
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
