//! Fetch execution and response delivery.

use futures_util::StreamExt;

use crate::{
    api::{
        error::RequestError,
        request::{CacheMode, PreparedRequest, Request},
        response::{InternalResponse, Response, ResponseBody, ResponseType, StreamingResponse},
    },
    cache::StoredResponse,
    engine::{RequestController, cache_capture::with_cache_capture},
};

impl RequestController {
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
    pub async fn fetch_stream(&self, request: Request) -> Result<StreamingResponse, RequestError> {
        let (request, url, cacheable) = self.prepare_request(request)?;
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
                        credentials_mode: request.context.credentials,
                        response_type: ResponseType::Basic,
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
        let request = request;
        let mut transport_headers = request.fetch_headers()?;
        if request.credentials_allowed(&url) {
            if let Some(services) = &self.inner.services {
                if let Some(cookie) = services.cookie_header(&request)? {
                    let value = http::HeaderValue::from_str(&cookie)
                        .map_err(|_| RequestError::InvalidHeaderValue("cookie".into()))?;
                    transport_headers.insert_internal_header(http::header::COOKIE, value)?;
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
                    headers: transport_headers,
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
        crate::policy::validate_response(&request, &response)?;
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
}
