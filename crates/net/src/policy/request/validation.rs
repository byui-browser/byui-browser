use crate::api::{
    error::RequestError,
    request::{CacheMode, CredentialsMode, Request, RequestPriority},
};

use super::FetchScheme;

/// Validates URL and browser request/response policy at the controller boundary.
///
/// Transport performs HTTP I/O only; this type is where browser-facing policy
/// checks can evolve without coupling them to reqwest.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestPolicy;

/// Parsed URL and the dispatcher selection derived from it.
#[derive(Clone, Debug)]
pub(crate) struct ValidatedUrl {
    /// Normalized URL used by the selected fetch algorithm.
    pub(crate) url: reqwest::Url,
    /// Scheme-specific dispatcher selected during validation.
    pub(crate) scheme: FetchScheme,
}

impl RequestPolicy {
    /// Parses a URL and selects the scheme-specific Fetch dispatcher.
    ///
    /// Validation happens before cookies are attached or a scheduler permit is
    /// acquired, so invalid requests fail without side effects or network work.
    pub(crate) fn validate_request(
        &self,
        request: &Request,
        max_keepalive_body_size: u64,
    ) -> Result<ValidatedUrl, RequestError> {
        request.validate_body_state(max_keepalive_body_size)?;
        if request.keepalive {
            return Err(RequestError::UnsupportedFeature("keepalive lifetime"));
        }
        if request.context.mode == crate::RequestMode::SameOrigin
            && request.context.environment.origin.is_none()
        {
            return Err(RequestError::UnsupportedFeature("client origin"));
        }
        if request.context.credentials == CredentialsMode::Include
            && request.context.environment.origin.is_none()
        {
            return Err(RequestError::UnsupportedFeature("client origin"));
        }
        if request.integrity.is_some() {
            return Err(RequestError::UnsupportedFeature("subresource integrity"));
        }
        if request.cache_mode == CacheMode::NoCache {
            return Err(RequestError::UnsupportedFeature("HTTP cache revalidation"));
        }
        if request.priority != RequestPriority::Auto {
            return Err(RequestError::UnsupportedFeature("priority scheduling"));
        }
        let url = match reqwest::Url::parse(&request.url) {
            Ok(url) => url,
            Err(_) => match &request.context.environment.base_url {
                Some(base_url) => base_url
                    .join(&request.url)
                    .map_err(|_| RequestError::InvalidUrl(request.url.clone()))?,
                None => return Err(RequestError::InvalidUrl(request.url.clone())),
            },
        };
        self.validate_target(request, &url)?;
        let scheme = FetchScheme::from_url(&url)?;
        if request.context.mode == crate::RequestMode::Navigate && scheme != FetchScheme::File {
            return Err(RequestError::UnsupportedFeature("navigation fetch"));
        }
        log::trace!(
            "request validated: {} as {scheme:?}",
            crate::log_url::loggable(url.as_str())
        );
        Ok(ValidatedUrl { url, scheme })
    }
    pub(super) fn validate_target(
        &self,
        request: &Request,
        url: &reqwest::Url,
    ) -> Result<(), RequestError> {
        if !url.username().is_empty() || url.password().is_some() {
            log::warn!(
                "url with embedded credentials rejected: {}",
                url.origin().ascii_serialization()
            );
            return Err(RequestError::InvalidUrl(url.to_string()));
        }
        if request.context.mode == crate::RequestMode::SameOrigin
            && request.effective_origin().is_some_and(|origin| {
                crate::Origin::from_url(url).is_ok_and(|target| !origin.is_same_origin(&target))
            })
            && request.redirect_count > 0
        {
            log::warn!(
                "same-origin redirect target blocked before transport: {}",
                crate::log_url::loggable(url.as_str())
            );
            return Err(RequestError::SameOriginViolation);
        }
        Ok(())
    }
}
