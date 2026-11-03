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
                    log::debug!(
                        "cache hit: {} {} -> {} ({} bytes, allow_stale={allow_stale})",
                        request.method(),
                        crate::log_url::loggable(url.as_str()),
                        response.status,
                        response.body.len()
                    );
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
            if cache_lookup.is_some() {
                log::debug!(
                    "cache miss: {} {}",
                    request.method(),
                    crate::log_url::loggable(url.as_str())
                );
            }
            if request.cache_mode == CacheMode::OnlyIfCached {
                log::debug!(
                    "only-if-cached request has no cache entry: {}",
                    crate::log_url::loggable(url.as_str())
                );
                return Err(RequestError::NetworkError);
            }
        }

        let mut request = request;
        let mut url = url;
        let mut response;
        loop {
            if request.signal.is_aborted() {
                log::debug!(
                    "request aborted before transport: {}",
                    crate::log_url::loggable(url.as_str())
                );
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
                        let value = http::HeaderValue::from_str(&cookie).map_err(|_| {
                            log::warn!(
                                "cookie header from services is invalid; failing {}",
                                crate::log_url::loggable(url.as_str())
                            );
                            RequestError::InvalidHeaderValue("cookie".into())
                        })?;
                        transport_headers.insert_internal_header(http::header::COOKIE, value)?;
                    }
                }
            }

            // The scheduler owns concurrency admission. Transport remains
            // focused on HTTP I/O and connection pooling.
            let had_nonreplayable_body = request
                .body
                .as_ref()
                .is_some_and(|body| !body.is_replayable());
            let attempt = request.for_transport()?;
            let mut candidate = self
                .inner
                .scheduler
                .submit(
                    PreparedRequest {
                        request: attempt,
                        headers: transport_headers,
                        url: url.clone(),
                    },
                    request.transport_priority(),
                )
                .await?;
            let status = candidate.status.as_u16();
            log::debug!(
                "transport response: {} {} -> {status}",
                request.method(),
                crate::log_url::loggable(url.as_str())
            );
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                response = candidate;
                break;
            }

            let location = candidate
                .headers
                .get(http::header::LOCATION)
                .ok_or_else(|| {
                    log::warn!(
                        "redirect {status} from {} has no location header",
                        crate::log_url::loggable(url.as_str())
                    );
                    RequestError::RedirectFailure
                })?
                .to_str()
                .map_err(|_| {
                    log::warn!(
                        "redirect {status} from {} has a non-text location header",
                        crate::log_url::loggable(url.as_str())
                    );
                    RequestError::RedirectFailure
                })?;
            let next_url = url.join(location).map_err(|_| {
                log::warn!(
                    "redirect {status} from {} has an invalid location",
                    crate::log_url::loggable(url.as_str())
                );
                RequestError::RedirectFailure
            })?;
            self.inner
                .policy
                .validate_redirect_target(&request, &next_url)
                .inspect_err(|error| {
                    log::warn!(
                        "redirect target blocked: {} -> {} ({})",
                        crate::log_url::loggable(url.as_str()),
                        crate::log_url::loggable(next_url.as_str()),
                        crate::log_url::loggable_error(error)
                    );
                })?;
            match request.redirect_mode {
                crate::RedirectMode::Error => {
                    log::warn!(
                        "redirect {status} to {} refused by redirect mode error",
                        crate::log_url::loggable(next_url.as_str())
                    );
                    return Err(RequestError::RedirectFailure);
                }
                crate::RedirectMode::Manual => {
                    log::debug!(
                        "manual redirect {status}: {} -> {}",
                        crate::log_url::loggable(url.as_str()),
                        crate::log_url::loggable(next_url.as_str())
                    );
                    candidate.url_list = request.url_list.iter().map(ToString::to_string).collect();
                    candidate.redirect_count = request.redirect_count + 1;
                    candidate.response_type = ResponseType::OpaqueRedirect;
                    candidate.body = Body::null();
                    return Ok(candidate.expose());
                }
                crate::RedirectMode::Follow => {}
            }
            if request.redirect_count >= 20 {
                log::warn!(
                    "redirect limit reached at {}",
                    crate::log_url::loggable(url.as_str())
                );
                return Err(RequestError::RedirectFailure);
            }
            let rewrite_to_get = (matches!(status, 301 | 302)
                && request.method() == http::Method::POST)
                || (status == 303
                    && !matches!(request.method(), &http::Method::GET | &http::Method::HEAD));
            if had_nonreplayable_body && !rewrite_to_get {
                log::warn!(
                    "cannot replay request body for {status} redirect from {}",
                    crate::log_url::loggable(url.as_str())
                );
                return Err(RequestError::RedirectFailure);
            }
            if rewrite_to_get {
                request.rewrite_for_redirect()?;
            }
            let cross_origin = request.current_url().is_some_and(|current| {
                current.scheme() != next_url.scheme() || current.origin() != next_url.origin()
            });
            log::debug!(
                "following redirect {status}: {} -> {} (hop {})",
                crate::log_url::loggable(url.as_str()),
                crate::log_url::loggable(next_url.as_str()),
                request.redirect_count + 1
            );
            if cross_origin {
                log::trace!("cross-origin redirect, stripping sensitive headers");
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
                log::trace!(
                    "storing null-body response in cache: {}",
                    crate::log_url::loggable(url.as_str())
                );
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
                request.for_cache(),
                url,
                self.inner.max_cache_body_size,
            )?;
        }
        Ok(response.expose())
    }
}
