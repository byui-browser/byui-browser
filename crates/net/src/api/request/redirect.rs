use http::header::HeaderName;
use url::Url;

use crate::api::error::RequestError;

use super::{ReferrerPolicy, Request, ServiceWorkersMode};

impl Request {
    /// Records the cache partition selected during request preparation.
    pub(crate) fn set_cache_partition(&mut self, partition: String) {
        self.execution.cache_partition = Some(partition);
    }

    /// Records the validated URL as the current and initial request URL.
    pub(crate) fn prepare_url(&mut self, url: Url) {
        if self.execution.url_list.is_empty() {
            self.execution.url_list.push(url.clone());
        }
        if self.execution.request_origin.is_none() {
            self.execution.request_origin = self.context.environment.origin.clone();
        }
        self.execution.current_url = Some(url);
    }

    pub(crate) fn follow_url(&mut self, url: Url) {
        log::trace!(
            "request {} now targets {}",
            self.id,
            crate::log_url::loggable(url.as_str())
        );
        self.execution.url_list.push(url.clone());
        self.execution.current_url = Some(url);
        self.execution.redirect_count += 1;
    }

    pub(crate) fn rewrite_for_redirect(&mut self) -> Result<(), RequestError> {
        self.set_method("GET")?;
        self.body = None;
        for name in [
            "content-encoding",
            "content-language",
            "content-location",
            "content-type",
            "content-length",
        ] {
            self.headers
                .remove_internal_header(HeaderName::from_static(name));
        }
        Ok(())
    }

    /// Applies the complete state transition for one followed network redirect.
    ///
    /// The redirect response must be finalized before this is called. The
    /// caller validates the resulting target after the transition and before
    /// starting the next HTTP hop.
    pub(crate) fn transition_to_redirect(
        &mut self,
        next_url: Url,
        response_headers: &http::HeaderMap,
        rewrite_to_get: bool,
    ) -> Result<(), RequestError> {
        if rewrite_to_get {
            self.rewrite_for_redirect()?;
        }
        self.apply_redirect_policy(&next_url, response_headers);
        self.follow_url(next_url);
        Ok(())
    }

    pub(crate) fn strip_sensitive_redirect_headers(&mut self) {
        log::trace!("stripping sensitive headers for cross-origin redirect");
        for name in [
            HeaderName::from_static("authorization"),
            HeaderName::from_static("proxy-authorization"),
        ] {
            self.headers.remove_internal_header(name);
        }
    }

    /// Applies origin and referrer-policy transitions before following a redirect.
    pub(crate) fn apply_redirect_policy(
        &mut self,
        next_url: &Url,
        response_headers: &http::HeaderMap,
    ) {
        if let Some(policy) = redirect_referrer_policy(response_headers) {
            self.referrer_policy = policy;
        }
        let crosses_origin = self.execution.current_url.as_ref().is_some_and(|current| {
            current.scheme() != next_url.scheme() || current.origin() != next_url.origin()
        });
        if crosses_origin {
            self.strip_sensitive_redirect_headers();
        }
        self.service_workers = ServiceWorkersMode::None;
    }
}

/// Selects the last recognized policy token from `Referrer-Policy` metadata.
fn redirect_referrer_policy(headers: &http::HeaderMap) -> Option<ReferrerPolicy> {
    headers
        .get_all(http::header::REFERRER_POLICY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|token| match token.trim().to_ascii_lowercase().as_str() {
            "no-referrer" => Some(ReferrerPolicy::NoReferrer),
            "no-referrer-when-downgrade" => Some(ReferrerPolicy::NoReferrerWhenDowngrade),
            "origin" => Some(ReferrerPolicy::Origin),
            "origin-when-cross-origin" => Some(ReferrerPolicy::OriginWhenCrossOrigin),
            "same-origin" => Some(ReferrerPolicy::SameOrigin),
            "strict-origin" => Some(ReferrerPolicy::StrictOrigin),
            "strict-origin-when-cross-origin" => Some(ReferrerPolicy::StrictOriginWhenCrossOrigin),
            "unsafe-url" => Some(ReferrerPolicy::UnsafeUrl),
            _ => None,
        })
        .next_back()
}
