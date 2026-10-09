//! Request and response policy boundary.

use crate::{
    error::RequestError,
    request::{CacheMode, CredentialsMode, RedirectMode, Request, RequestPriority},
    response::InternalResponse,
};

/// Validates URL and browser request/response policy at the controller boundary.
///
/// Transport performs HTTP I/O only; this type is where browser-facing policy
/// checks can evolve without coupling them to reqwest.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestPolicy;

impl RequestPolicy {
    /// Rejects malformed URLs and schemes unsupported by the HTTP transport.
    ///
    /// Validation happens before cookies are attached or a scheduler permit is
    /// acquired, so invalid requests fail without side effects or network work.
    pub(crate) fn validate_request(
        &self,
        request: &Request,
        max_keepalive_body_size: u64,
    ) -> Result<reqwest::Url, RequestError> {
        request.validate_body_state(max_keepalive_body_size)?;
        if request.keepalive {
            return Err(RequestError::UnsupportedFeature("keepalive lifetime"));
        }
        if request.context.mode == crate::RequestMode::Navigate {
            return Err(RequestError::UnsupportedFeature("navigation fetch"));
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
        if request.redirect_mode != RedirectMode::Follow {
            return Err(RequestError::UnsupportedFeature("redirect mode"));
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
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RequestError::UnsupportedScheme(url.scheme().to_owned()));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(RequestError::InvalidUrl(request.url.clone()));
        }
        if request.context.mode == crate::RequestMode::Cors
            && request
                .context
                .environment
                .origin
                .as_ref()
                .is_some_and(|origin| {
                    crate::Origin::from_url(&url)
                        .is_ok_and(|target| !origin.is_same_origin(&target))
                })
            && request.requires_cors_preflight()
        {
            return Err(RequestError::UnsupportedFeature("CORS preflight"));
        }
        Ok(url)
    }

    /// Validates response policy after transport headers have arrived.
    ///
    /// The initial implementation accepts all responses; the method remains a
    /// separate hook for future status, redirect, or response-security rules.
    pub(crate) fn validate_response(
        &self,
        _request: &Request,
        _response: &InternalResponse,
    ) -> Result<(), RequestError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https_urls() {
        let policy = RequestPolicy;
        assert!(
            policy
                .validate_request(&Request::get("http://example.test"), 65_536)
                .is_ok()
        );
        assert!(
            policy
                .validate_request(&Request::get("https://example.test"), 65_536)
                .is_ok()
        );
    }

    #[test]
    fn rejects_invalid_urls_before_transport() {
        let error = RequestPolicy
            .validate_request(&Request::get("not a URL"), 65_536)
            .unwrap_err();

        assert!(matches!(error, RequestError::InvalidUrl(url) if url == "not a URL"));
    }

    #[test]
    fn rejects_relative_urls() {
        let error = RequestPolicy
            .validate_request(&Request::get("/relative/path"), 65_536)
            .unwrap_err();

        assert!(matches!(error, RequestError::InvalidUrl(url) if url == "/relative/path"));
    }

    #[test]
    fn rejects_urls_with_invalid_ports() {
        for value in [
            "http://example.test:not-a-port/",
            "http://example.test:65536/",
        ] {
            let error = RequestPolicy
                .validate_request(&Request::get(value), 65_536)
                .unwrap_err();

            assert!(matches!(error, RequestError::InvalidUrl(url) if url == value));
        }
    }

    #[test]
    fn accepts_uppercase_http_urls_and_returns_normalized_url() {
        let url = RequestPolicy
            .validate_request(&Request::get("HTTP://EXAMPLE.TEST/Path"), 65_536)
            .unwrap();

        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("example.test"));
        assert_eq!(url.path(), "/Path");
    }

    #[test]
    fn rejects_non_http_schemes() {
        let error = RequestPolicy
            .validate_request(&Request::get("ftp://example.test/file"), 65_536)
            .unwrap_err();

        assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
    }

    #[test]
    fn unsupported_integrity_is_rejected_before_transport() {
        let mut request = Request::get("https://example.test/data");
        request.set_integrity(Some("sha256-example".into()));
        let error = RequestPolicy
            .validate_request(&request, 65_536)
            .unwrap_err();
        assert!(matches!(
            error,
            RequestError::UnsupportedFeature("subresource integrity")
        ));
    }

    #[test]
    fn preflight_required_cross_origin_request_is_rejected_before_transport() {
        let mut request = Request::new("https://remote.test/data", "PUT").unwrap();
        request.context.environment.origin =
            Some(crate::Origin::parse("https://client.test").unwrap());
        let error = RequestPolicy
            .validate_request(&request, 65_536)
            .unwrap_err();
        assert!(matches!(
            error,
            RequestError::UnsupportedFeature("CORS preflight")
        ));
    }

    #[test]
    fn keepalive_request_is_rejected_until_lifetime_is_supported() {
        let mut request = Request::get("https://example.test/");
        request.set_keepalive(true);
        let error = RequestPolicy
            .validate_request(&request, 65_536)
            .unwrap_err();
        assert!(matches!(
            error,
            RequestError::UnsupportedFeature("keepalive lifetime")
        ));
    }
}
