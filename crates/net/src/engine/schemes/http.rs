//! HTTP(S)-specific Fetch execution paths.

use crate::{
    api::{
        body::Body,
        error::RequestError,
        request::{CacheMode, PreparedRequest, Request},
        response::{InternalResponse, Response, ResponseType, has_null_body},
    },
    cache::StoredResponse,
    engine::{RequestController, cache_capture::with_cache_capture},
};

impl RequestController {
    /// Executes the existing HTTP(S) provider path after scheme dispatch.
    pub(crate) async fn fetch_http(
        &self,
        request: Request,
        url: reqwest::Url,
        cacheable: bool,
    ) -> Result<Response, RequestError> {
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
                    // Return information about the devtools and other observers, but do not allow the body to be consumed until the caller polls it.
                    let mut body = if has_null_body(request.method(), response.status) {
                        Body::null()
                    } else {
                        Body::once_with_signal(response.body, request.signal.clone())
                    };
                    body.set_response_metadata(&response.headers, None);
                    let mut internal = InternalResponse::from_http_parts(
                        response.status,
                        response.headers,
                        response.url,
                        &request.context,
                        body,
                        true,
                        response.cookie_headers_processed,
                    );
                    self.finalize_response(&request, &mut internal)?;
                    return Ok(internal.expose());
                }
            }
            if request.cache_mode == CacheMode::OnlyIfCached {
                return Err(RequestError::NetworkError);
            }
        }

        let mut request = request;
        let mut url = url;
        let mut response;
        loop {
            if request.signal.is_aborted() {
                return Err(RequestError::Aborted);
            }
            if let Some(services) = &self.inner.services {
                services.check_request(&request)?;
            }

            // Cookie and policy modules operate before transport sees the
            // request; this keeps browser behavior out of the low-level HTTP
            // implementation.
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

            // The scheduler owns concurrency admission. Transport remains
            // focused on HTTP I/O and connection pooling.
            let mut candidate = self
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
            let status = candidate.status.as_u16();
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                response = candidate;
                break;
            }

            let location = candidate
                .headers
                .get(http::header::LOCATION)
                .ok_or(RequestError::RedirectFailure)?
                .to_str()
                .map_err(|_| RequestError::RedirectFailure)?;
            let next_url = url
                .join(location)
                .map_err(|_| RequestError::RedirectFailure)?;
            self.inner
                .policy
                .validate_redirect_target(&request, &next_url)?;
            match request.redirect_mode {
                crate::RedirectMode::Error => return Err(RequestError::RedirectFailure),
                crate::RedirectMode::Manual => {
                    candidate.url_list = request.url_list.iter().map(ToString::to_string).collect();
                    candidate.redirect_count = request.redirect_count + 1;
                    candidate.response_type = ResponseType::OpaqueRedirect;
                    candidate.body = Body::null();
                    return Ok(candidate.expose());
                }
                crate::RedirectMode::Follow => {}
            }
            if request.redirect_count >= 20 {
                return Err(RequestError::RedirectFailure);
            }
            if matches!(status, 307 | 308)
                && request
                    .body
                    .as_ref()
                    .is_some_and(|body| !body.is_replayable())
            {
                return Err(RequestError::RedirectFailure);
            }
            if (matches!(status, 301 | 302) && request.method() == http::Method::POST)
                || (status == 303
                    && !matches!(request.method(), &http::Method::GET | &http::Method::HEAD))
            {
                request.rewrite_for_redirect()?;
            }
            let cross_origin = request.current_url().is_some_and(|current| {
                current.scheme() != next_url.scheme() || current.origin() != next_url.origin()
            });
            if cross_origin {
                request.strip_sensitive_redirect_headers();
            }
            request.follow_url(next_url.clone());
            url = next_url;
        }
        response.url_list = request.url_list.iter().map(ToString::to_string).collect();
        response.redirect_count = request.redirect_count;
        response.origin = response
            .url_list
            .last()
            .and_then(|url| reqwest::Url::parse(url).ok())
            .and_then(|url| crate::Origin::from_url(&url).ok());
        self.finalize_response(&request, &mut response)?;
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
            if response.body.is_null() {
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
            response = with_cache_capture(
                response,
                self.inner.cache.clone(),
                request.clone(),
                url,
                self.inner.max_cache_body_size,
            );
        }
        Ok(response.expose())
    }
}
