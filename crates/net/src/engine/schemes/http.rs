//! HTTP(S)-specific Fetch execution paths.

use crate::{
    api::{
        body::Body,
        error::RequestError,
        request::{CacheMode, PreparedRequest, Request},
        response::{InternalResponse, Response, has_null_body},
    },
    cache::StoredResponse,
    engine::{RequestController, cache_capture::with_cache_capture},
};

impl RequestController {
    /// Executes HTTP(S) exchanges and controller-owned redirects.
    pub(crate) async fn fetch_http(
        &self,
        mut request: Request,
        mut url: reqwest::Url,
    ) -> Result<Response, RequestError> {
        let mut response;
        loop {
            if request.signal.is_aborted() {
                log::debug!(
                    "request aborted before transport: {}",
                    crate::log_url::loggable(url.as_str())
                );
                return Err(RequestError::Aborted);
            }
            if request.redirect_count > 0 {
                self.authorize_redirect_hop(&request)?;
            }
            request.reduce_referrer_for_target(&url);
            let (mut transport_headers, generated_headers) =
                self.transport_headers(&request, &url)?;
            self.ensure_cors_preflight(&request, &url, &generated_headers)
                .await?;

            let cacheable = request.is_cacheable_method();
            let cache_lookup = match request.cache_mode {
                CacheMode::Default | CacheMode::OnlyIfCached => Some(false),
                CacheMode::ForceCache => Some(true),
                CacheMode::NoStore | CacheMode::Reload | CacheMode::NoCache => None,
            };
            if cacheable {
                if let Some(allow_stale) = cache_lookup
                    && let Some(cached) =
                        self.inner
                            .cache
                            .get_with_staleness(&request, &url, allow_stale)
                {
                    log::debug!(
                        "cache hit: {} {} -> {} ({} bytes, allow_stale={allow_stale})",
                        request.method(),
                        crate::log_url::loggable(url.as_str()),
                        cached.status,
                        cached.body.len()
                    );
                    let mut body = if has_null_body(request.method(), cached.status) {
                        Body::null()
                    } else {
                        Body::once_with_signal(cached.body, request.signal.clone())
                    };
                    body.set_response_metadata(&cached.headers, None);
                    let mut internal = InternalResponse::from_http_parts(
                        cached.status,
                        cached.headers,
                        cached.url,
                        &request.context,
                        body,
                        true,
                        cached.cookie_headers_processed,
                    );
                    Self::set_response_redirect_state(&request, &mut internal);
                    self.finalize_response(&request, &mut internal)
                        .inspect_err(|_| {
                            self.clear_preflight_cache_after_failure(&request, &url);
                        })?;
                    return Ok(internal.expose());
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

            if request.credentials_allowed(&url)
                && let Some(services) = &self.inner.services
                && let Some(cookie) = services.cookie_header(&request)?
            {
                let value = http::HeaderValue::from_str(&cookie).map_err(|_| {
                    log::warn!(
                        "cookie header from services is invalid; failing {}",
                        crate::log_url::loggable(url.as_str())
                    );
                    RequestError::InvalidHeaderValue("cookie".into())
                })?;
                transport_headers.insert_internal_header(http::header::COOKIE, value)?;
            }

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
                .await
                .inspect_err(|_| {
                    self.clear_preflight_cache_after_failure(&request, &url);
                })?;
            Self::set_response_redirect_state(&request, &mut candidate);
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

            let Some(location) = candidate.headers.get(http::header::LOCATION) else {
                // A redirect status without Location is an ordinary response;
                // there is no target for Fetch's redirect algorithm to follow.
                response = candidate;
                break;
            };

            if request.redirect_mode == crate::RedirectMode::Manual {
                candidate.redirect_count = request.redirect_count + 1;
                self.finalize_manual_redirect_response(&request, &mut candidate)?;
                self.process_response_cookies(&request, &url, &mut candidate)?;
                log::debug!(
                    "manual redirect {status} returned as opaque from {}",
                    crate::log_url::loggable(url.as_str())
                );
                candidate.body = Body::null();
                return Ok(candidate.expose());
            }

            if request.redirect_mode == crate::RedirectMode::Error {
                log::warn!(
                    "redirect {status} from {} refused by redirect mode error",
                    crate::log_url::loggable(url.as_str())
                );
                return Err(RequestError::RedirectFailure);
            }
            let location = location.to_str().map_err(|_| {
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
                .validate_redirect_location(&next_url)
                .inspect_err(|error| {
                    log::warn!(
                        "redirect target blocked: {} -> {} ({})",
                        crate::log_url::loggable(url.as_str()),
                        crate::log_url::loggable(next_url.as_str()),
                        crate::log_url::loggable_error(error)
                    );
                })?;

            self.finalize_response(&request, &mut candidate)
                .inspect_err(|_| {
                    self.clear_preflight_cache_after_failure(&request, &url);
                })?;
            self.process_response_cookies(&request, &url, &mut candidate)?;
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
            log::debug!(
                "following redirect {status}: {} -> {} (hop {})",
                crate::log_url::loggable(url.as_str()),
                crate::log_url::loggable(next_url.as_str()),
                request.redirect_count + 1
            );
            request.apply_redirect_policy(&next_url, &candidate.headers);
            request.follow_url(next_url.clone());
            self.inner
                .policy
                .validate_redirect_target(&request, &next_url)
                .inspect_err(|error| {
                    log::warn!(
                        "redirect hop policy blocked: {} ({})",
                        crate::log_url::loggable(next_url.as_str()),
                        crate::log_url::loggable_error(error)
                    );
                })?;
            url = next_url;
        }

        self.finalize_response(&request, &mut response)
            .inspect_err(|_| {
                self.clear_preflight_cache_after_failure(&request, &url);
            })?;
        self.process_response_cookies(&request, &url, &mut response)?;

        if request.is_cacheable_method() && request.cache_mode != CacheMode::NoStore {
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

    /// Repeats browser-owned request authorization for a followed hop.
    fn authorize_redirect_hop(&self, request: &Request) -> Result<(), RequestError> {
        if let Some(services) = &self.inner.services {
            services.check_request(request)?;
        }
        Ok(())
    }

    /// Builds all non-credential request headers once for a validated hop.
    fn transport_headers(
        &self,
        request: &Request,
        url: &reqwest::Url,
    ) -> Result<(crate::Headers, crate::Headers), RequestError> {
        let mut headers = request.fetch_headers(url)?;
        let generated = if let Some(services) = &self.inner.services {
            let generated = services.generated_request_headers(request, url)?;
            headers.merge_generated_request_headers(&generated)?;
            generated
        } else {
            crate::Headers::generated_request_headers()
        };
        Ok((headers, generated))
    }

    /// Copies controller redirect state into a transport or cached response.
    fn set_response_redirect_state(request: &Request, response: &mut InternalResponse) {
        response.url_list = request.url_list.iter().map(ToString::to_string).collect();
        response.redirect_count = request.redirect_count;
        response.origin = response
            .url_list
            .last()
            .and_then(|url| reqwest::Url::parse(url).ok())
            .and_then(|url| crate::Origin::from_url(&url).ok());
        response.request_origin = request.effective_origin().cloned();
    }

    /// Gives approved HTTP response cookies to browser-owned storage policy.
    fn process_response_cookies(
        &self,
        request: &Request,
        url: &reqwest::Url,
        response: &mut InternalResponse,
    ) -> Result<(), RequestError> {
        if response.cookie_headers_processed || !request.credentials_allowed(url) {
            return Ok(());
        }
        if let Some(services) = &self.inner.services {
            let values: Vec<http::HeaderValue> = response
                .headers
                .get_all(http::header::SET_COOKIE)
                .iter()
                .cloned()
                .collect();
            services.store_set_cookie(request, &values)?;
            response.cookie_headers_processed = true;
        }
        Ok(())
    }
}
