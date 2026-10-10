//! HTTP transport boundary. Transport performs I/O; it does not make browser policy decisions.

use std::sync::Arc;

use futures_util::StreamExt;
use reqwest::Client;
use reqwest::header::{HeaderValue, REFERER};

use crate::api::{
    config::Config,
    error::RequestError,
    request::PreparedRequest,
    response::{InternalResponse, ResponseBody, ResponseType},
};

/// Performs HTTP I/O behind the networking policy and scheduling layers.
///
/// Implementations must be shareable across controller clones and must return
/// sendable futures because requests may execute concurrently on the async
/// runtime. Browser policy decisions belong to the caller, not this boundary.
pub(crate) trait Transport: Send + Sync {
    /// Executes a request and returns once response headers are available.
    ///
    /// The body remains a stream and is received as the caller polls the
    /// returned internal response.
    fn send(
        &self,
        request: PreparedRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<InternalResponse, RequestError>> + Send>,
    >;
}

/// Builds the reqwest-backed transport used by the networking controller.
///
/// Client construction configures connection pooling and the
/// optional user agent once. The resulting client can then be cheaply cloned
/// for concurrent request futures.
pub(crate) fn reqwest_transport(config: Config) -> Result<Arc<dyn Transport>, reqwest::Error> {
    Ok(Arc::new(ReqwestTransport::new(config)?))
}

/// HTTP transport implementation that owns one reusable reqwest client.
struct ReqwestTransport {
    client: Client,
}

impl ReqwestTransport {
    /// Creates a configured reqwest client without performing network I/O.
    fn new(config: Config) -> Result<Self, reqwest::Error> {
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .pool_idle_timeout(config.pool_idle_timeout)
            .pool_max_idle_per_host(config.pool_max_idle_per_host);
        if let Some(user_agent) = config.user_agent {
            builder = builder.user_agent(user_agent);
        }
        Ok(Self {
            client: builder.build()?,
        })
    }
}

impl Transport for ReqwestTransport {
    fn send(
        &self,
        request: PreparedRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<InternalResponse, RequestError>> + Send>,
    > {
        let client = self.client.clone();
        Box::pin(async move {
            let PreparedRequest {
                request,
                headers: request_headers,
                url,
            } = request;
            if request.signal.is_aborted() {
                return Err(RequestError::Aborted);
            }
            let signal = request.signal.clone();
            let mut headers = request_headers.to_reqwest();
            headers.remove(REFERER);
            if let Some(referrer) = request.referrer_value(&url) {
                if let Ok(value) = HeaderValue::try_from(referrer) {
                    headers.insert(REFERER, value);
                }
            }
            let is_head = request.method == reqwest::Method::HEAD;
            let mut builder = client.request(request.method, url).headers(headers);
            if let Some(body) = request.body {
                builder = builder.body(body.into_reqwest_body()?);
            }
            let response = tokio::select! {
                _ = signal.cancelled() => return Err(RequestError::Aborted),
                response = builder.send() => response.map_err(|error| RequestError::Transport(error.to_string()))?,
            };
            let status = response.status();
            let body_is_null = is_head || matches!(status.as_u16(), 101 | 204 | 205 | 304);
            let headers = response.headers().clone();
            let response_url = response.url().to_string();
            let body = response
                .bytes_stream()
                .map(|chunk| chunk.map_err(|error| RequestError::Transport(error.to_string())));

            Ok(InternalResponse {
                status,
                status_text: status.canonical_reason().unwrap_or_default().to_owned(),
                headers,
                url_list: vec![response_url],
                redirect_count: 0,
                origin: None,
                request_origin: request.context.environment.origin.clone(),
                request_mode: request.context.mode,
                credentials_mode: request.context.credentials,
                response_type: ResponseType::Basic,
                body: ResponseBody::from_stream_with_signal(body, signal),
                body_is_null,
                from_cache: false,
                cookie_headers_processed: false,
            })
        })
    }
}
