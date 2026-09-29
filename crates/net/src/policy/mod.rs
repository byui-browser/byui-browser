//! Request and response policy boundary.

use crate::{error::RequestError, request::Request, response::Response};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestPolicy;

impl RequestPolicy {
    pub(crate) fn validate_request(&self, request: &Request) -> Result<(), RequestError> {
        let url = reqwest::Url::parse(&request.url)
            .map_err(|_| RequestError::InvalidUrl(request.url.clone()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RequestError::UnsupportedScheme(url.scheme().to_owned()));
        }
        Ok(())
    }

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
                .validate_request(&Request::get("http://example.test"))
                .is_ok()
        );
        assert!(
            policy
                .validate_request(&Request::get("https://example.test"))
                .is_ok()
        );
    }

    #[test]
    fn rejects_invalid_urls_before_transport() {
        let error = RequestPolicy
            .validate_request(&Request::get("not a URL"))
            .unwrap_err();

        assert!(matches!(error, RequestError::InvalidUrl(url) if url == "not a URL"));
    }

    #[test]
    fn rejects_non_http_schemes() {
        let error = RequestPolicy
            .validate_request(&Request::get("ftp://example.test/file"))
            .unwrap_err();

        assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
    }
}
