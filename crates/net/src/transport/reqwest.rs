//! HTTP transport boundary. Transport performs I/O; it does not make browser policy decisions.

use std::sync::Arc;

use reqwest::Client;

use crate::{config::Config, error::RequestError, request::Request, response::Response};

pub(crate) trait Transport: Send + Sync {
    fn send(
        &self,
        request: Request,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, RequestError>> + Send>>;
}

pub(crate) fn reqwest_transport(config: Config) -> Result<Arc<dyn Transport>, reqwest::Error> {
    Ok(Arc::new(ReqwestTransport::new(config)?))
}

struct ReqwestTransport {
    client: Client,
}

impl ReqwestTransport {
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, RequestError>> + Send>>
    {
        let client = self.client.clone();
        Box::pin(async move {
            let url = reqwest::Url::parse(&request.url)
                .map_err(|_| RequestError::InvalidUrl(request.url.clone()))?;
            let mut builder = client.request(request.method, url).headers(request.headers);
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            let response = builder.send().await?;
            Ok(Response {
                status: response.status(),
                headers: response.headers().clone(),
                url: response.url().to_string(),
                body: response.bytes().await?.to_vec(),
                from_cache: false,
            })
        })
    }
}
