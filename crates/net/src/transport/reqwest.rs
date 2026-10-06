//! HTTP transport boundary. Transport performs I/O; it does not make browser policy decisions.

use std::sync::Arc;

use futures_util::StreamExt;
use reqwest::Client;

use crate::{
    config::Config,
    error::RequestError,
    request::Request,
    response::{ResponseBody, StreamingResponse},
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
    /// returned [`StreamingResponse`].
    fn send(
        &self,
        request: Request,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<StreamingResponse, RequestError>> + Send>,
    >;
}

/// Builds the reqwest-backed transport used by the networking controller.
///
/// Client construction configures connection pooling, redirects, and the
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
            .redirect(reqwest::redirect::Policy::limited(config.max_redirects))
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
        request: Request,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<StreamingResponse, RequestError>> + Send>,
    > {
        let client = self.client.clone();
        Box::pin(async move {
            let url = reqwest::Url::parse(&request.url)
                .map_err(|_| RequestError::InvalidUrl(request.url.clone()))?;
            let mut builder = client.request(request.method, url).headers(request.headers);
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            let response = builder.send().await?;
            let status = response.status();
            let headers = response.headers().clone();
            let response_url = response.url().to_string();
            let body = response
                .bytes_stream()
                .map(|chunk| chunk.map_err(RequestError::from));

            Ok(StreamingResponse {
                status,
                headers,
                url: response_url,
                body: ResponseBody::from_stream(body),
                from_cache: false,
            })
        })
    }
}
