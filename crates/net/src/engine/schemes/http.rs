//! HTTP(S)-specific Fetch execution paths.

use crate::{
    api::{
        body::Body,
        diagnostics::NetworkPhase,
        error::RequestError,
        request::{CacheMode, PreparedRequest, Request, ValidatedHop},
        response::{InternalResponse, Response, has_null_body},
    },
    cache::StoredResponse,
    engine::{
        RequestController, cache_capture::with_cache_capture,
        process_response::ResponseFinalization,
    },
};

impl RequestController {
    /// Executes HTTP(S) exchanges and controller-owned redirects.
    pub(crate) async fn fetch_http(
        &self,
        prepared: PreparedRequest,
    ) -> Result<Response, RequestError> {
        let (mut request, mut url) = prepared.into_parts();
        let mut response;
        loop {
            let transport_headers = self.validate_http_hop(&mut request, &url).await?;

            let cacheable = request.is_cacheable_method();
            let cache_lookup = match request.cache_mode() {
                CacheMode::Default | CacheMode::OnlyIfCached => Some(false),
                CacheMode::ForceCache => Some(true),
                CacheMode::NoStore | CacheMode::Reload | CacheMode::NoCache => None,
            };
            if cacheable {
                self.observe(&request, NetworkPhase::CacheLookup, &url, None, false);
                if let Some(allow_stale) = cache_lookup
                    && let Some(cached) =
                        self.inner
                            .cache
                            .get_with_staleness(&request, &url, allow_stale)
                {
                    self.observe(
                        &request,
                        NetworkPhase::CacheHit,
                        &url,
                        Some(cached.status.as_u16()),
                        true,
                    );
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
                        Body::once_with_signal(cached.body, request.signal().clone())
                    };
                    body.set_response_metadata(&cached.headers, None);
                    let mut internal = InternalResponse::from_http_parts(
                        cached.status,
                        cached.headers,
                        cached.url,
                        request.context(),
                        body,
                        true,
                        cached.cookie_headers_processed,
                    );
                    internal.apply_request_state(&request);
                    self.finalize_response(&request, &mut internal, ResponseFinalization::Standard)
                        .inspect_err(|_| {
                            self.clear_preflight_cache_after_failure(&request, &url);
                        })?;
                    self.observe(
                        &request,
                        NetworkPhase::ResponseFinalized,
                        &url,
                        Some(internal.status.as_u16()),
                        true,
                    );
                    return Ok(internal.expose());
                }
                if cache_lookup.is_some() {
                    self.observe(&request, NetworkPhase::CacheMiss, &url, None, false);
                    log::debug!(
                        "cache miss: {} {}",
                        request.method(),
                        crate::log_url::loggable(url.as_str())
                    );
                }
                if request.cache_mode() == CacheMode::OnlyIfCached {
                    log::debug!(
                        "only-if-cached request has no cache entry: {}",
                        crate::log_url::loggable(url.as_str())
                    );
                    return Err(RequestError::NetworkError);
                }
            }

            let had_nonreplayable_body = request
                .body()
                .as_ref()
                .is_some_and(|body| !body.is_replayable());
            let validated = self.finish_http_hop(&mut request, &url, transport_headers)?;
            let mut candidate = self
                .inner
                .scheduler
                .submit(validated, request.transport_priority())
                .await
                .inspect_err(|_| {
                    self.clear_preflight_cache_after_failure(&request, &url);
                })?;
            candidate.apply_request_state(&request);
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

            if request.redirect_mode() == crate::RedirectMode::Manual {
                candidate.redirect_count = request.redirect_count() + 1;
                self.finalize_http_response(
                    &request,
                    &url,
                    &mut candidate,
                    ResponseFinalization::ManualRedirect,
                )?;
                log::debug!(
                    "manual redirect {status} returned as opaque from {}",
                    crate::log_url::loggable(url.as_str())
                );
                candidate.body = Body::null();
                return Ok(candidate.expose());
            }

            if request.redirect_mode() == crate::RedirectMode::Error {
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

            self.finalize_http_response(
                &request,
                &url,
                &mut candidate,
                ResponseFinalization::Standard,
            )
            .inspect_err(|_| {
                self.clear_preflight_cache_after_failure(&request, &url);
            })?;
            if request.redirect_count() >= 20 {
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
            log::debug!(
                "following redirect {status}: {} -> {} (hop {})",
                crate::log_url::loggable(url.as_str()),
                crate::log_url::loggable(next_url.as_str()),
                request.redirect_count() + 1
            );
            self.observe(&request, NetworkPhase::Redirect, &url, Some(status), false);
            request.transition_to_redirect(next_url.clone(), &candidate.headers, rewrite_to_get)?;
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

        self.finalize_http_response(
            &request,
            &url,
            &mut response,
            ResponseFinalization::Standard,
        )
        .inspect_err(|_| {
            self.clear_preflight_cache_after_failure(&request, &url);
        })?;
        self.observe(
            &request,
            NetworkPhase::ResponseFinalized,
            &url,
            Some(response.status.as_u16()),
            response.from_cache,
        );
        if request.is_cacheable_method() && request.cache_mode() != CacheMode::NoStore {
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

    /// Validates the request state shared by every executable HTTP hop.
    ///
    /// Initial browser authorization has already run during preparation. For
    /// followed redirects this method performs the per-hop authorization
    /// before deriving any target-dependent headers or starting preflight.
    /// Cookie selection intentionally remains after cache lookup so cache hits
    /// do not consult browser cookie storage.
    async fn validate_http_hop(
        &self,
        request: &mut Request,
        url: &reqwest::Url,
    ) -> Result<crate::Headers, RequestError> {
        if request.signal().is_aborted() {
            log::debug!(
                "request aborted before transport: {}",
                crate::log_url::loggable(url.as_str())
            );
            return Err(RequestError::Aborted);
        }
        if request.redirect_count() > 0
            && let Some(policy) = &self.inner.request_policy
        {
            policy.check_request(request)?;
        }
        request.reduce_referrer_for_target(url);
        let (headers, generated_headers) = self.transport_headers(request, url)?;
        self.ensure_cors_preflight(request, url, &generated_headers)
            .await?;
        Ok(headers)
    }

    /// Completes a cache-miss hop with credentials and transport ownership.
    fn finish_http_hop(
        &self,
        request: &mut Request,
        url: &reqwest::Url,
        mut transport_headers: crate::Headers,
    ) -> Result<ValidatedHop, RequestError> {
        self.select_cookies(request, url, &mut transport_headers)?;
        Ok(ValidatedHop::new(
            request.for_transport()?,
            transport_headers,
            url.clone(),
        ))
    }

    /// Adds the browser-selected cookie to a cache-miss transport attempt.
    fn select_cookies(
        &self,
        request: &Request,
        url: &reqwest::Url,
        transport_headers: &mut crate::Headers,
    ) -> Result<(), RequestError> {
        if request.credentials_allowed(url)
            && let Some(cookies) = &self.inner.cookies
            && let Some(cookie) = cookies.cookie_header(request)?
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
        Ok(())
    }

    /// Builds all non-credential request headers once for a validated hop.
    fn transport_headers(
        &self,
        request: &Request,
        url: &reqwest::Url,
    ) -> Result<(crate::Headers, crate::Headers), RequestError> {
        let mut headers = request.fetch_headers(url)?;
        let generated = if let Some(policy) = &self.inner.request_policy {
            let generated = policy.generated_request_headers(request, url)?;
            headers.merge_generated_request_headers(&generated)?;
            generated
        } else {
            crate::Headers::generated_request_headers()
        };
        Ok((headers, generated))
    }
}
