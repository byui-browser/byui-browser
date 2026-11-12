//! Response tainting and the supported simple-CORS validation boundary.

use crate::api::{
    error::RequestError,
    request::{CredentialsMode, Origin, RedirectTaint, Request, RequestMode},
    response::{InternalResponse, ResponseType},
};

/// Cross-origin validation boundary used by the request controller.
///
/// Cross-origin responses are rejected or filtered before reaching Rust callers.
/// Preflight and ordinary CORS responses share the same origin and credential
/// validation rules before any response is exposed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CorsChecker;

impl CorsChecker {
    /// Chooses the exposed response class, failing closed when cross-origin access is denied.
    pub(crate) fn response_type(
        &self,
        request: &Request,
        response: &InternalResponse,
    ) -> Result<ResponseType, RequestError> {
        let final_url = response.url_list.last().ok_or(RequestError::NetworkError)?;
        let origin = request.effective_origin();
        let same_origin = request.redirect_taint() == RedirectTaint::SameOrigin
            && origin.is_some_and(|origin| {
                response
                    .origin
                    .as_ref()
                    .is_some_and(|target| origin.is_same_origin(target))
                    || reqwest::Url::parse(final_url)
                        .ok()
                        .and_then(|url| Origin::from_url(&url).ok())
                        .is_some_and(|target| origin.is_same_origin(&target))
            });
        if same_origin || (origin.is_none() && request.context.mode != RequestMode::NoCors) {
            return Ok(ResponseType::Basic);
        }
        match request.context.mode {
            RequestMode::SameOrigin => {
                log::warn!(
                    "same-origin request blocked from reaching {}",
                    crate::log_url::loggable(final_url.as_str())
                );
                Err(RequestError::SameOriginViolation)
            }
            RequestMode::NoCors => {
                log::trace!(
                    "cross-origin no-cors response is opaque: {}",
                    crate::log_url::loggable(final_url.as_str())
                );
                Ok(ResponseType::Opaque)
            }
            RequestMode::Navigate => Ok(ResponseType::Basic),
            RequestMode::Cors => {
                if self.validate_response_headers(request, &response.headers) {
                    Ok(ResponseType::Cors)
                } else {
                    log::warn!(
                        "cors check denied response from {}",
                        crate::log_url::loggable(final_url.as_str())
                    );
                    Err(RequestError::CorsDenied)
                }
            }
        }
    }

    /// Validates the allow-origin and credential fields shared by CORS checks.
    pub(crate) fn validate_response_headers(
        &self,
        request: &Request,
        headers: &http::HeaderMap,
    ) -> bool {
        let allowed_origin = single_header_value(headers, "access-control-allow-origin");
        let serialized = request.serialized_origin().unwrap_or_default();
        let allowed = allowed_origin == Some(serialized.as_str())
            || (allowed_origin == Some("*")
                && request.context.credentials != CredentialsMode::Include);
        let credentials_ok = request.context.credentials != CredentialsMode::Include
            || single_header_value(headers, "access-control-allow-credentials") == Some("true");
        log::trace!(
            "cors response headers: allow-origin={allowed_origin:?} \
             allowed={allowed} credentials_ok={credentials_ok}"
        );
        allowed && credentials_ok
    }
}

/// Returns one textual field value, rejecting duplicated or non-text values.
fn single_header_value<'a>(headers: &'a http::HeaderMap, name: &'static str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::body::Body;

    /// Builds a response with the supplied headers for CORS tests.
    fn response(headers: reqwest::header::HeaderMap) -> InternalResponse {
        InternalResponse {
            status: reqwest::StatusCode::OK,
            status_text: "OK".into(),
            headers,
            url_list: vec!["https://remote.test/data".into()],
            redirect_count: 0,
            origin: Some(Origin::parse("https://remote.test").unwrap()),
            request_origin: Some(Origin::parse("https://client.test").unwrap()),
            request_mode: RequestMode::Cors,
            credentials_mode: CredentialsMode::SameOrigin,
            response_type: ResponseType::Basic,
            body: Body::once(Vec::new()),
            from_cache: false,
            cookie_headers_processed: false,
        }
    }

    /// Builds a request with an explicit CORS mode for policy tests.
    fn request(mode: RequestMode) -> Request {
        let mut request = Request::get("https://remote.test/data");
        request.context.environment.origin = Some(Origin::parse("https://client.test").unwrap());
        request.set_mode(mode).unwrap();
        request
    }

    #[test]
    /// Verifies same-origin, CORS, and no-CORS cross-origin outcomes.
    fn cross_origin_modes_fail_or_filter_before_exposure() {
        let checker = CorsChecker;
        let response = response(reqwest::header::HeaderMap::new());
        assert!(matches!(
            checker.response_type(&request(RequestMode::SameOrigin), &response),
            Err(RequestError::SameOriginViolation)
        ));
        assert!(matches!(
            checker.response_type(&request(RequestMode::Cors), &response),
            Err(RequestError::CorsDenied)
        ));
        assert_eq!(
            checker
                .response_type(&request(RequestMode::NoCors), &response)
                .unwrap(),
            ResponseType::Opaque
        );
    }

    #[test]
    /// Verifies simple CORS requires a matching allow-origin value.
    fn simple_cors_requires_matching_allow_origin() {
        let checker = CorsChecker;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "access-control-allow-origin",
            "https://client.test".parse().unwrap(),
        );
        assert_eq!(
            checker
                .response_type(&request(RequestMode::Cors), &response(headers))
                .unwrap(),
            ResponseType::Cors
        );
    }

    #[test]
    /// Verifies duplicated singleton CORS fields fail closed.
    fn simple_cors_rejects_duplicate_allow_origin_fields() {
        let checker = CorsChecker;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            "access-control-allow-origin",
            "https://client.test".parse().unwrap(),
        );
        headers.append("access-control-allow-origin", "*".parse().unwrap());
        assert!(matches!(
            checker.response_type(&request(RequestMode::Cors), &response(headers)),
            Err(RequestError::CorsDenied)
        ));
    }

    #[test]
    /// Verifies a redirect-tainted return to the request origin remains CORS filtered.
    fn tainted_same_origin_return_still_requires_cors_approval() {
        let checker = CorsChecker;
        let mut request = request(RequestMode::Cors);
        request.request_origin = Some(Origin::parse("https://client.test").unwrap());
        request.url_list = [
            "https://client.test/start",
            "https://remote.test/middle",
            "https://client.test/final",
        ]
        .into_iter()
        .map(|url| reqwest::Url::parse(url).unwrap())
        .collect();

        let mut final_response = response(reqwest::header::HeaderMap::new());
        final_response.url_list = vec!["https://client.test/final".into()];
        final_response.origin = Some(Origin::parse("https://client.test").unwrap());
        assert!(matches!(
            checker.response_type(&request, &final_response),
            Err(RequestError::CorsDenied)
        ));

        final_response
            .headers
            .insert("access-control-allow-origin", "null".parse().unwrap());
        assert_eq!(
            checker.response_type(&request, &final_response).unwrap(),
            ResponseType::Cors
        );
    }

    #[test]
    /// Verifies no-CORS responses remain opaque without a client origin.
    fn no_cors_without_client_origin_is_still_opaque() {
        let mut request = Request::get("https://remote.test/data");
        request.set_mode(RequestMode::NoCors).unwrap();
        assert_eq!(
            CorsChecker
                .response_type(&request, &response(reqwest::header::HeaderMap::new()))
                .unwrap(),
            ResponseType::Opaque
        );
    }
}
