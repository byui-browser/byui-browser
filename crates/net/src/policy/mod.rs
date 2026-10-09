//! Request and response policy boundary.

use crate::{error::RequestError, request::Request, response::Response};

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
        Ok(url)
    }

    /// Validates response policy after transport headers have arrived.
    ///
    /// The initial implementation accepts all responses; the method remains a
    /// separate hook for future status, redirect, or response-security rules.
    pub(crate) fn validate_response(
        &self,
        _request: &Request,
        _response: &Response,
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
}
