//! Response tainting and the supported simple-CORS validation boundary.

use crate::{
    error::RequestError,
    request::{CredentialsMode, Origin, Request, RequestMode},
    response::{InternalResponse, ResponseType},
};

/// Cross-origin validation boundary used by the request controller.
///
/// Cross-origin responses are rejected or filtered before reaching Rust callers.
/// Preflight-required requests are rejected before transport by request policy.
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
        let url = reqwest::Url::parse(final_url).map_err(|_| RequestError::NetworkError)?;
        let origin = request.context.environment.origin.as_ref();
        let same_origin = origin.is_some_and(|origin| {
            Origin::from_url(&url).is_ok_and(|target| origin.is_same_origin(&target))
        });
        if same_origin || (origin.is_none() && request.context.mode != RequestMode::NoCors) {
            return Ok(ResponseType::Basic);
        }
        match request.context.mode {
            RequestMode::SameOrigin => Err(RequestError::SameOriginViolation),
            RequestMode::NoCors => Ok(ResponseType::Opaque),
            RequestMode::Navigate => Ok(ResponseType::Basic),
            RequestMode::Cors => {
                let allowed_origin = response
                    .headers
                    .get("access-control-allow-origin")
                    .and_then(|value| value.to_str().ok());
                let serialized = origin.map(Origin::as_str).unwrap_or_default();
                let allowed = allowed_origin == Some(serialized.as_str())
                    || (allowed_origin == Some("*")
                        && request.context.credentials != CredentialsMode::Include);
                let credentials_ok = request.context.credentials != CredentialsMode::Include
                    || response
                        .headers
                        .get("access-control-allow-credentials")
                        .is_some_and(|value| value.as_bytes() == b"true");
                if allowed && credentials_ok {
                    Ok(ResponseType::Cors)
                } else {
                    Err(RequestError::CorsDenied)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::ResponseBody;

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
            response_type: ResponseType::Basic,
            body: ResponseBody::once(Vec::new()),
            body_is_null: false,
            from_cache: false,
            cookie_headers_processed: false,
        }
    }

    fn request(mode: RequestMode) -> Request {
        let mut request = Request::get("https://remote.test/data");
        request.context.environment.origin = Some(Origin::parse("https://client.test").unwrap());
        request.set_mode(mode).unwrap();
        request
    }

    #[test]
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
