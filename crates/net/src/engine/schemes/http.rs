//! HTTP(S)-specific Fetch execution paths.

use crate::{
    api::{
        body::Body,
        diagnostics::NetworkPhase,
        error::RequestError,
        request::{
            CacheMode, PreparedRequest, Request, RequestMode, ValidatedHop, urls_are_same_origin,
        },
        response::{InternalResponse, Response, has_null_body},
    },
    cache::{CacheLookup, CacheValidator, RevalidationLease, StoredResponse},
    engine::{
        RequestController, cache_capture::with_cache_capture,
        process_response::ResponseFinalization,
    },
};

/// Result of looking up an HTTP request in the cache.
enum CacheDecision {
    /// A finalized terminal response can return without a transport attempt.
    Hit(Response),
    /// A stored redirect must continue through redirect processing.
    Redirect(StoredResponse),
    /// A transport attempt is required, optionally with a cached validator and
    /// a stale representation that may mask a network or server failure.
    Miss {
        revalidation: Option<(StoredResponse, CacheValidator)>,
        stale_if_error: Option<StoredResponse>,
    },
}

/// Result of processing one HTTP transport response.
enum HopResult {
    /// The response completes the fetch.
    Complete(InternalResponse),
    /// Redirect processing prepared another request URL.
    Follow(reqwest::Url),
    /// A manual redirect can be exposed immediately.
    Expose(Response),
}

impl RequestController {
    /// Executes HTTP(S) exchanges and controller-owned redirects.
    pub(crate) async fn fetch_http(
        &self,
        prepared: PreparedRequest,
    ) -> Result<Response, RequestError> {
        let (mut request, mut url) = prepared.into_parts();
        if request.cache_mode() == CacheMode::OnlyIfCached
            && request.mode() != RequestMode::SameOrigin
        {
            log::debug!(
                "only-if-cached request requires same-origin mode: {}",
                crate::log_url::loggable(url.as_str())
            );
            return Err(RequestError::NetworkError);
        }
        let (response, request_headers) = loop {
            let mut transport_headers = self.validate_http_hop(&mut request, &url).await?;
            self.select_cookies(&request, &url, &mut transport_headers)?;
            let (revalidation, stale_if_error) =
                match self.lookup_http_cache(&request, &url, &transport_headers)? {
                    CacheDecision::Hit(response) => return Ok(response),
                    CacheDecision::Redirect(cached) => {
                        let response = self.response_from_cached_hit(&request, &url, cached);
                        match self.process_http_result(
                            &mut request,
                            &url,
                            response,
                            &transport_headers,
                            false,
                        )? {
                            HopResult::Complete(response) => {
                                return self
                                    .finalize_cached_http_response(&request, &url, response);
                            }
                            HopResult::Follow(next_url) => {
                                url = next_url;
                                continue;
                            }
                            HopResult::Expose(response) => return Ok(response),
                        }
                    }
                    CacheDecision::Miss {
                        revalidation,
                        stale_if_error,
                    } => (revalidation, stale_if_error),
                };

            let had_nonreplayable_body = request
                .body()
                .as_ref()
                .is_some_and(|body| !body.is_replayable());
            let (candidate, request_headers) = match self
                .submit_http_hop(&mut request, &url, &mut transport_headers, revalidation)
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    if let Some(cached) = stale_if_error {
                        log::debug!(
                            "serving stale response after transport failure for {}",
                            crate::log_url::loggable(url.as_str())
                        );
                        return self.serve_cached_response(&request, &url, cached);
                    }
                    return Err(error);
                }
            };
            if candidate.status.is_server_error()
                && let Some(cached) = stale_if_error
            {
                log::debug!(
                    "serving stale response after {} for {}",
                    candidate.status,
                    crate::log_url::loggable(url.as_str())
                );
                return self.serve_cached_response(&request, &url, cached);
            }
            match self.process_http_result(
                &mut request,
                &url,
                candidate,
                &request_headers,
                had_nonreplayable_body,
            )? {
                HopResult::Complete(response) => break (response, request_headers),
                HopResult::Follow(next_url) => {
                    url = next_url;
                    continue;
                }
                HopResult::Expose(response) => return Ok(response),
            }
        };

        self.finalize_and_cache_http_response(&request, &request_headers, url, response)
    }

    /// Looks up the current hop and selects an optional validator for transport.
    fn lookup_http_cache(
        &self,
        request: &Request,
        url: &reqwest::Url,
        request_headers: &crate::Headers,
    ) -> Result<CacheDecision, RequestError> {
        if !request.is_cacheable_method() {
            return Ok(CacheDecision::Miss {
                revalidation: None,
                stale_if_error: None,
            });
        }
        self.observe(request, NetworkPhase::CacheLookup, url, None, false);
        let cache_lookup = matches!(
            request.cache_mode(),
            CacheMode::Default
                | CacheMode::OnlyIfCached
                | CacheMode::ForceCache
                | CacheMode::NoCache
        );
        let mut revalidation = None;
        let mut stale_if_error = None;
        let cached = match request.cache_mode() {
            CacheMode::NoCache => {
                revalidation = self.inner.cache.lookup_for_revalidation_with_headers(
                    request,
                    url,
                    request_headers,
                );
                None
            }
            CacheMode::ForceCache | CacheMode::OnlyIfCached => self
                .inner
                .cache
                .lookup_matching_with_headers(request, url, request_headers),
            _ => match cache_lookup
                .then(|| {
                    self.inner
                        .cache
                        .lookup_with_headers(request, url, request_headers)
                })
                .flatten()
            {
                Some(CacheLookup::Fresh(response)) => Some(response),
                Some(CacheLookup::Stale {
                    response,
                    validator,
                    stale_while_revalidate,
                    stale_if_error: may_serve_stale_if_error,
                }) => {
                    if stale_while_revalidate
                        && let Ok(mut background_request) = request.try_clone()
                    {
                        if let Some(lease) = self.inner.cache.begin_background_revalidation(
                            request,
                            url,
                            request_headers,
                        ) {
                            background_request.set_cache_mode(CacheMode::NoCache);
                            self.spawn_background_revalidation(background_request, lease);
                        }
                        // A refresh already in progress is sufficient: every
                        // caller inside the stale-while-revalidate window
                        // receives the stale representation immediately.
                        Some(response)
                    } else {
                        if may_serve_stale_if_error {
                            stale_if_error = Some(response.clone());
                        }
                        revalidation = validator.map(|validator| (response, validator));
                        None
                    }
                }
                None => None,
            },
        };
        if let Some(cached) = cached {
            if is_redirect_status(cached.status) {
                return Ok(CacheDecision::Redirect(cached));
            }
            return self
                .serve_cached_response(request, url, cached)
                .map(CacheDecision::Hit);
        }
        if cache_lookup {
            self.observe(request, NetworkPhase::CacheMiss, url, None, false);
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
        Ok(CacheDecision::Miss {
            revalidation,
            stale_if_error,
        })
    }

    /// Refreshes one stale representation without delaying its caller.
    fn spawn_background_revalidation(&self, request: Request, lease: RevalidationLease) {
        let controller = self.clone();
        tokio::spawn(async move {
            // The lease remains alive for the full fetch, including body
            // capture setup, and releases the duplicate-refresh marker when
            // this task returns.
            let _lease = lease;
            if let Err(error) = controller.fetch(request).await {
                log::debug!("background cache revalidation failed: {error}");
            }
        });
    }

    /// Records a cache hit and reconstructs its internal response state.
    fn response_from_cached_hit(
        &self,
        request: &Request,
        url: &reqwest::Url,
        cached: StoredResponse,
    ) -> InternalResponse {
        self.observe(
            request,
            NetworkPhase::CacheHit,
            url,
            Some(cached.status.as_u16()),
            true,
        );
        log::debug!(
            "cache hit: {} {} -> {} ({} bytes)",
            request.method(),
            crate::log_url::loggable(url.as_str()),
            cached.status,
            cached.body.len()
        );
        self.response_from_stored(request, cached)
    }

    /// Finalizes and exposes a terminal cached representation without selecting cookies.
    fn serve_cached_response(
        &self,
        request: &Request,
        url: &reqwest::Url,
        cached: StoredResponse,
    ) -> Result<Response, RequestError> {
        let response = self.response_from_cached_hit(request, url, cached);
        self.finalize_cached_http_response(request, url, response)
    }

    /// Applies terminal-response policy and exposes a cached representation.
    fn finalize_cached_http_response(
        &self,
        request: &Request,
        url: &reqwest::Url,
        mut response: InternalResponse,
    ) -> Result<Response, RequestError> {
        self.finalize_response(request, &mut response, ResponseFinalization::Standard)
            .inspect_err(|_| self.clear_preflight_cache_after_failure(request, url))?;
        self.observe(
            request,
            NetworkPhase::ResponseFinalized,
            url,
            Some(response.status.as_u16()),
            true,
        );
        Ok(response.expose())
    }

    /// Creates an internal response from a stored HTTP representation.
    fn response_from_stored(&self, request: &Request, cached: StoredResponse) -> InternalResponse {
        let mut body = if has_null_body(request.method(), cached.status) {
            Body::null()
        } else {
            Body::once_bytes_with_signal(cached.body, request.signal().clone())
        };
        body.set_response_metadata(&cached.headers, None);
        let mut response = InternalResponse::from_http_parts(
            cached.status,
            cached.headers,
            cached.url,
            request.context(),
            body,
            true,
            cached.cookie_headers_processed,
        );
        response.apply_request_state(request);
        response
    }

    /// Submits one cache-miss hop and merges a successful conditional response.
    async fn submit_http_hop(
        &self,
        request: &mut Request,
        url: &reqwest::Url,
        transport_headers: &mut crate::Headers,
        revalidation: Option<(StoredResponse, CacheValidator)>,
    ) -> Result<(InternalResponse, crate::Headers), RequestError> {
        if let Some((_, validator)) = &revalidation {
            validator.apply(transport_headers)?;
        }
        let validated = self.finish_http_hop(request, url, transport_headers.clone())?;
        let mut response = self
            .inner
            .scheduler
            .submit(validated, request.transport_priority())
            .await
            .inspect_err(|_| self.clear_preflight_cache_after_failure(request, url))?;
        response.apply_request_state(request);
        log::debug!(
            "transport response: {} {} -> {}",
            request.method(),
            crate::log_url::loggable(url.as_str()),
            response.status
        );
        if !request.method().is_safe() {
            self.invalidate_after_unsafe_response(request, url, &response);
        }
        if response.status == http::StatusCode::NOT_MODIFIED
            && let Some((validated_response, _)) = &revalidation
            && let Some(cached) = self.inner.cache.merge_not_modified_with_headers(
                request,
                url,
                transport_headers,
                validated_response,
                &response.headers,
            )
        {
            let validation_cookie_headers: Vec<http::HeaderValue> = response
                .headers
                .get_all(http::header::SET_COOKIE)
                .iter()
                .cloned()
                .collect();
            let mut merged = self.response_from_stored(request, cached);
            merged.validation_cookie_headers = Some(validation_cookie_headers.into_boxed_slice());
            return Ok((merged, transport_headers.clone()));
        }
        if response.status == http::StatusCode::NOT_MODIFIED
            && revalidation.is_some()
            && let Some(cached) =
                self.inner
                    .cache
                    .lookup_matching_with_headers(request, url, transport_headers)
        {
            // A concurrent revalidation replaced the variant after this
            // request selected its validator. The newer cached
            // representation, rather than this obsolete `304`, is the only
            // safe response to expose.
            let validation_cookie_headers: Vec<http::HeaderValue> = response
                .headers
                .get_all(http::header::SET_COOKIE)
                .iter()
                .cloned()
                .collect();
            let mut cached = self.response_from_stored(request, cached);
            cached.validation_cookie_headers = Some(validation_cookie_headers.into_boxed_slice());
            return Ok((cached, transport_headers.clone()));
        }
        Ok((response, transport_headers.clone()))
    }

    /// Processes a response as terminal, manual redirect, or followed redirect.
    fn process_http_result(
        &self,
        request: &mut Request,
        url: &reqwest::Url,
        mut response: InternalResponse,
        request_headers: &crate::Headers,
        had_nonreplayable_body: bool,
    ) -> Result<HopResult, RequestError> {
        let status = response.status.as_u16();
        if !is_redirect_status(response.status) {
            return Ok(HopResult::Complete(response));
        }
        let Some(location) = response.headers.get(http::header::LOCATION) else {
            return Ok(HopResult::Complete(response));
        };
        if request.redirect_mode() == crate::RedirectMode::Manual {
            response.redirect_count = request.redirect_count() + 1;
            self.finalize_http_response(
                request,
                url,
                &mut response,
                ResponseFinalization::ManualRedirect,
            )?;
            log::debug!(
                "manual redirect {status} returned as opaque from {}",
                crate::log_url::loggable(url.as_str())
            );
            response.body = Body::null();
            return Ok(HopResult::Expose(response.expose()));
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
        self.finalize_http_response(request, url, &mut response, ResponseFinalization::Standard)
            .inspect_err(|_| self.clear_preflight_cache_after_failure(request, url))?;
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
        self.cache_redirect_response(request, url, request_headers, &response);
        log::debug!(
            "following redirect {status}: {} -> {} (hop {})",
            crate::log_url::loggable(url.as_str()),
            crate::log_url::loggable(next_url.as_str()),
            request.redirect_count() + 1
        );
        self.observe(request, NetworkPhase::Redirect, url, Some(status), false);
        request.transition_to_redirect(next_url.clone(), &response.headers, rewrite_to_get)?;
        self.inner
            .policy
            .validate_redirect_target(request, &next_url)
            .inspect_err(|error| {
                log::warn!(
                    "redirect hop policy blocked: {} ({})",
                    crate::log_url::loggable(next_url.as_str()),
                    crate::log_url::loggable_error(error)
                );
            })?;
        Ok(HopResult::Follow(next_url))
    }

    /// Stores an eligible followed redirect after response policy has approved it.
    ///
    /// Redirect bodies are never exposed while following, so retaining an
    /// empty body preserves the reusable redirect metadata without buffering
    /// discarded transport bytes.
    fn cache_redirect_response(
        &self,
        request: &Request,
        url: &reqwest::Url,
        request_headers: &crate::Headers,
        response: &InternalResponse,
    ) {
        if response.from_cache
            || !request.is_cacheable_method()
            || request.cache_mode() == CacheMode::NoStore
        {
            return;
        }
        self.inner.cache.insert_with_headers(
            request,
            url,
            request_headers,
            StoredResponse {
                status: response.status,
                headers: response.headers.clone(),
                url: response.url_list.last().cloned().unwrap_or_default(),
                body: bytes::Bytes::new(),
                cookie_headers_processed: response.cookie_headers_processed,
            },
        );
    }

    /// Finalizes a terminal response and arranges cache storage when allowed.
    fn finalize_and_cache_http_response(
        &self,
        request: &Request,
        request_headers: &crate::Headers,
        url: reqwest::Url,
        mut response: InternalResponse,
    ) -> Result<Response, RequestError> {
        self.finalize_http_response(request, &url, &mut response, ResponseFinalization::Standard)
            .inspect_err(|_| self.clear_preflight_cache_after_failure(request, &url))?;
        self.observe(
            request,
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
                self.inner.cache.insert_with_headers(
                    request,
                    &url,
                    request_headers,
                    StoredResponse {
                        status: response.status,
                        headers: response.headers.clone(),
                        url: response.url_list.last().cloned().unwrap_or_default(),
                        body: bytes::Bytes::new(),
                        cookie_headers_processed: response.cookie_headers_processed,
                    },
                );
            } else {
                response = with_cache_capture(
                    response,
                    self.inner.cache.clone(),
                    request.for_cache(),
                    request_headers.clone(),
                    url,
                    self.inner.max_cache_body_size,
                )?;
            }
        }
        Ok(response.expose())
    }

    /// Validates the request state shared by every executable HTTP hop.
    ///
    /// Initial browser authorization has already run during preparation. For
    /// followed redirects this method performs the per-hop authorization
    /// before deriving any target-dependent headers or starting preflight.
    /// Cookie selection follows this step so cache matching can use the
    /// complete outgoing header set.
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

    /// Transfers a cache-miss hop's already selected transport inputs.
    fn finish_http_hop(
        &self,
        request: &mut Request,
        url: &reqwest::Url,
        transport_headers: crate::Headers,
    ) -> Result<ValidatedHop, RequestError> {
        Ok(ValidatedHop::new(
            request.for_transport()?,
            transport_headers,
            url.clone(),
        ))
    }

    /// Invalidates cache entries affected by one completed unsafe HTTP request.
    ///
    /// The request URL is always invalidated. A successful response can also
    /// identify a same-origin target through `Location`; Fetch requires that
    /// target to be invalidated without permitting one origin to evict another
    /// origin's cached data.
    fn invalidate_after_unsafe_response(
        &self,
        request: &Request,
        url: &reqwest::Url,
        response: &InternalResponse,
    ) {
        self.inner.cache.invalidate(request, url);
        if !(200..400).contains(&response.status.as_u16()) {
            return;
        }
        let Some(location) = response.headers.get(http::header::LOCATION) else {
            return;
        };
        let Ok(location) = location.to_str() else {
            return;
        };
        let Ok(target) = url.join(location) else {
            return;
        };
        if urls_are_same_origin(url, &target) {
            self.inner.cache.invalidate(request, &target);
        }
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
        match request.cache_mode() {
            CacheMode::NoCache if !headers.has(http::header::CACHE_CONTROL.as_str()) => headers
                .append_internal_header(
                    http::header::CACHE_CONTROL,
                    http::HeaderValue::from_static("max-age=0"),
                )?,
            CacheMode::NoStore | CacheMode::Reload => {
                if !headers.has(http::header::PRAGMA.as_str()) {
                    headers.append_internal_header(
                        http::header::PRAGMA,
                        http::HeaderValue::from_static("no-cache"),
                    )?;
                }
                if !headers.has(http::header::CACHE_CONTROL.as_str()) {
                    headers.append_internal_header(
                        http::header::CACHE_CONTROL,
                        http::HeaderValue::from_static("no-cache"),
                    )?;
                }
            }
            _ => {}
        }
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

/// Reports whether a response status triggers Fetch redirect processing.
fn is_redirect_status(status: http::StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}
