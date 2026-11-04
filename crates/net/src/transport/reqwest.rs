//! HTTP transport boundary. Transport performs I/O; it does not make browser policy decisions.

use std::sync::Arc;

use futures_util::StreamExt;
use reqwest::Client;
use reqwest::header::{HeaderValue, REFERER};

use crate::api::{
    body::Body,
    config::Config,
    error::RequestError,
    request::PreparedRequest,
    response::{InternalResponse, has_null_body},
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
        log::debug!(
            "building http client: pool_max_idle_per_host={}, custom user agent={}",
            config.pool_max_idle_per_host,
            config.user_agent.is_some()
        );
        if let Some(user_agent) = config.user_agent {
            builder = builder.user_agent(user_agent);
        }
        Ok(Self {
            client: builder.build()?,
        })
    }
}

impl Transport for ReqwestTransport {
    /// Sends the prepared request and exposes its response body as a live stream.
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
            let context = request.context.clone();
            let mut headers = request_headers.to_reqwest();
            headers.remove(REFERER);
            if let Some(referrer) = request.referrer_value(&url) {
                match HeaderValue::try_from(referrer) {
                    Ok(value) => {
                        headers.insert(REFERER, value);
                    }
                    Err(_) => {
                        log::debug!(
                            "referrer is not a valid header value; sent {} without it",
                            crate::log_url::loggable(url.as_str())
                        )
                    }
                }
            }
            let method = request.method.clone();
            let mut builder = client.request(method.clone(), url).headers(headers);
            if let Some(body) = request.body {
                builder = builder.body(body.into_reqwest_body()?);
            }
            let response = tokio::select! {
                _ = signal.cancelled() => return Err(RequestError::Aborted),
                response = builder.send() => response.map_err(|error| {
                    let message = error.to_string();
                    // reqwest's Display embeds the request URL; keep it out of logs.
                    log::debug!("transport send failed: {}", error.without_url());
                    RequestError::Transport(message)
                })?,
            };
            let status = response.status();
            log::trace!(
                "transport headers received: {status}, content-length {:?}",
                response.content_length()
            );
            let headers = response.headers().clone();
            let response_url = response.url().to_string();
            let mut body = if has_null_body(&method, status) {
                Body::null()
            } else {
                let body = response
                    .bytes_stream()
                    .map(|chunk| chunk.map_err(|error| RequestError::Transport(error.to_string())));
                Body::from_stream_with_signal(body, signal)
            };
            body.set_response_metadata(&headers, None);

            Ok(InternalResponse::from_http_parts(
                status,
                headers,
                response_url,
                &context,
                body,
                false,
                false,
            ))
        })
    }
}
