//! Request and response policy boundary.

use crate::api::{
    error::RequestError,
    request::{CacheMode, CredentialsMode, RedirectMode, Request, RequestPriority},
};

/// Validates URL and browser request/response policy at the controller boundary.
///
/// Transport performs HTTP I/O only; this type is where browser-facing policy
/// checks can evolve without coupling them to reqwest.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestPolicy;

/// URL scheme selected by the Fetch controller after URL parsing.
///
/// Non-HTTP variants are classified here so the controller can select a
/// scheme-specific implementation without coupling URL validation to HTTP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FetchScheme {
    /// HTTP network fetch.
    Http,
    /// HTTPS network fetch.
    Https,
    /// Inline data URL fetch.
    Data,
    /// Browser-owned object URL fetch.
    Blob,
    /// Provider-authorized local file fetch.
    File,
    /// Browser-owned local page fetch.
    Byui,
    /// Supported browser-defined `about:` fetch.
    About,
}

impl FetchScheme {
    /// Classifies a parsed URL or returns the typed unsupported-scheme error.
    pub(crate) fn from_url(url: &reqwest::Url) -> Result<Self, RequestError> {
        match url.scheme() {
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            "data" => Ok(Self::Data),
            "blob" => Ok(Self::Blob),
            "file" => Ok(Self::File),
            "byui" => Ok(Self::Byui),
            "about" => Ok(Self::About),
            scheme => Err(RequestError::UnsupportedScheme(scheme.to_owned())),
        }
    }

    /// Returns whether this scheme uses the existing HTTP network path.
    pub(crate) fn is_http(self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }
}

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
        if !url.username().is_empty() || url.password().is_some() {
            return Err(RequestError::InvalidUrl(request.url.clone()));
        }
        let scheme = FetchScheme::from_url(&url)?;
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
        Ok(ValidatedUrl { url, scheme })
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
        let validated = RequestPolicy
            .validate_request(&Request::get("HTTP://EXAMPLE.TEST/Path"), 65_536)
            .unwrap();

        assert_eq!(validated.url.scheme(), "http");
        assert_eq!(validated.url.host_str(), Some("example.test"));
        assert_eq!(validated.url.path(), "/Path");
        assert_eq!(validated.scheme, FetchScheme::Http);
    }

    #[test]
    fn rejects_non_http_schemes() {
        let error = RequestPolicy
            .validate_request(&Request::get("ftp://example.test/file"), 65_536)
            .unwrap_err();

        assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
    }

    #[test]
    fn classifies_supported_non_http_schemes_for_dispatch() {
        for (url, expected) in [
            ("data:text/plain,hello", FetchScheme::Data),
            ("blob:https://example.test/id", FetchScheme::Blob),
            ("file:///tmp/page.html", FetchScheme::File),
            ("byui://welcome", FetchScheme::Byui),
            ("about:blank", FetchScheme::About),
        ] {
            let validated = RequestPolicy
                .validate_request(&Request::get(url), 65_536)
                .unwrap();
            assert_eq!(validated.scheme, expected);
        }
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
