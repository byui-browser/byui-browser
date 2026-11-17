use crate::api::{error::RequestError, request::Request};

use super::{FetchScheme, RequestPolicy};

impl RequestPolicy {
    pub(crate) fn validate_redirect_target(
        &self,
        request: &Request,
        url: &reqwest::Url,
    ) -> Result<(), RequestError> {
        self.validate_redirect_location(url)?;
        self.validate_target(request, url)
            .map_err(|error| match error {
                RequestError::UnsupportedFeature(_) | RequestError::SameOriginViolation => error,
                _ => RequestError::RedirectFailure,
            })
    }

    /// Rejects redirect locations that cannot remain in the HTTP fetch path.
    pub(crate) fn validate_redirect_location(
        &self,
        url: &reqwest::Url,
    ) -> Result<(), RequestError> {
        let scheme = FetchScheme::from_url(url).map_err(|_| RequestError::RedirectFailure)?;
        if !scheme.is_http() {
            log::warn!("redirect target uses non-HTTP scheme: {}", scheme.name());
            return Err(RequestError::RedirectFailure);
        }
        if !url.username().is_empty() || url.password().is_some() {
            log::warn!(
                "redirect target contains embedded credentials: {}",
                url.origin().ascii_serialization()
            );
            return Err(RequestError::RedirectFailure);
        }
        Ok(())
    }
}
