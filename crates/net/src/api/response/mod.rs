//! Response types returned by the networking client.

mod types;
mod view;

pub use types::{Response, ResponseType};
pub(crate) use view::InternalResponse;

/// Returns whether Fetch requires a response body to be null for this exchange.
pub(crate) fn has_null_body(method: &http::Method, status: reqwest::StatusCode) -> bool {
    method == http::Method::HEAD || matches!(status.as_u16(), 101 | 204 | 205 | 304)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures_util::{StreamExt, stream};
    use reqwest::{StatusCode, header::HeaderMap};

    use super::*;
    use crate::{AbortController, Body, RequestError, RequestMode};

    #[test]
    fn null_body_rule_covers_head_and_bodyless_statuses() {
        for status in [101, 204, 205, 304] {
            assert!(has_null_body(
                &http::Method::GET,
                StatusCode::from_u16(status).unwrap()
            ));
        }
        assert!(has_null_body(&http::Method::HEAD, StatusCode::OK));
        assert!(!has_null_body(&http::Method::GET, StatusCode::OK));
    }

    #[test]
    /// Verifies provider failures expose stable, non-diagnostic categories.
    fn provider_error_categories_are_stable_and_non_diagnostic() {
        let error = RequestError::Provider {
            provider: "file",
            kind: crate::ProviderErrorKind::AccessDenied,
        };

        assert_eq!(
            error.to_string(),
            "Fetch provider file failed: AccessDenied"
        );
        assert!(matches!(
            error,
            RequestError::Provider {
                provider: "file",
                kind: crate::ProviderErrorKind::AccessDenied
            }
        ));
    }

    #[test]
    /// Verifies an aborted response stream ends with the typed abort error.
    fn a_pending_response_stream_ends_with_aborted_error() {
        let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
        let controller = AbortController::new();
        let mut body = Body::from_stream_with_signal(
            stream::pending::<Result<Bytes, RequestError>>(),
            controller.signal(),
        );
        controller.abort();

        let item = runtime
            .block_on(body.next())
            .expect("abort should produce an item");

        assert!(matches!(item, Err(RequestError::Aborted)));
    }

    #[test]
    /// Verifies basic response exposure removes cookie headers.
    fn basic_view_hides_cookie_headers_before_exposure() {
        let mut headers = HeaderMap::new();
        headers.insert("set-cookie", "session=secret".parse().unwrap());
        headers.insert("x-visible", "yes".parse().unwrap());
        let internal = InternalResponse {
            status: StatusCode::OK,
            status_text: "OK".into(),
            headers,
            url_list: vec!["https://example.test/".into()],
            redirect_count: 0,
            origin: None,
            request_origin: None,
            request_mode: RequestMode::Cors,
            credentials_mode: crate::CredentialsMode::SameOrigin,
            response_type: ResponseType::Basic,
            body: Body::once(b"body".to_vec()),
            from_cache: false,
            cookie_headers_processed: true,
        };
        assert!(internal.headers.contains_key("set-cookie"));
        let exposed = internal.expose();
        assert_eq!(exposed.headers.get("x-visible").unwrap(), "yes");
        assert!(!exposed.headers.has("set-cookie"));
        assert_eq!(exposed.headers.guard(), crate::HeaderGuard::Immutable);
    }

    #[test]
    /// Verifies filtered and null views do not reveal transport bytes.
    fn opaque_and_null_views_cannot_yield_transport_bytes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for (response_type, body_is_null) in [
            (ResponseType::Opaque, false),
            (ResponseType::OpaqueRedirect, false),
            (ResponseType::Error, false),
            (ResponseType::Basic, true),
        ] {
            let internal = InternalResponse {
                status: StatusCode::NO_CONTENT,
                status_text: "No Content".into(),
                headers: HeaderMap::new(),
                url_list: vec!["https://example.test/".into()],
                redirect_count: 0,
                origin: None,
                request_origin: None,
                request_mode: RequestMode::Cors,
                credentials_mode: crate::CredentialsMode::SameOrigin,
                response_type,
                body: if body_is_null {
                    Body::null()
                } else {
                    Body::once(b"forbidden".to_vec())
                },
                from_cache: response_type == ResponseType::Opaque,
                cookie_headers_processed: false,
            };
            let mut exposed = internal.expose();
            if matches!(
                response_type,
                ResponseType::Opaque | ResponseType::OpaqueRedirect | ResponseType::Error
            ) {
                assert_eq!(exposed.status, 0);
                assert!(exposed.url.is_empty());
                assert!(exposed.headers.is_empty());
                assert!(!exposed.from_cache);
            }
            assert!(
                runtime
                    .block_on(async { exposed.body.next().await })
                    .is_none()
            );
        }
    }

    #[test]
    /// Verifies response cloning copies metadata and tees live body bytes.
    fn response_clone_tees_an_unused_live_body() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut response = Response {
            response_type: ResponseType::Basic,
            status: 200,
            status_text: "OK".into(),
            headers: crate::Headers::with_guard(crate::HeaderGuard::Immutable),
            url: "https://example.test/".into(),
            redirected: false,
            body: Body::from_stream(stream::iter([
                Ok(Bytes::from_static(b"first")),
                Ok(Bytes::from_static(b"second")),
            ])),
            from_cache: false,
        };
        let clone = response.try_clone().unwrap();
        let (left, right) =
            runtime.block_on(async { tokio::join!(response.body.bytes(), clone.body.bytes()) });

        assert_eq!(left.unwrap(), b"firstsecond");
        assert_eq!(right.unwrap(), b"firstsecond");
    }

    #[test]
    /// Verifies network errors and response-derived helpers expose no body data.
    fn network_error_and_response_helpers_follow_filtered_semantics() {
        let error = Response::network_error();
        assert_eq!(error.response_type, ResponseType::Error);
        assert_eq!(error.status, 0);
        assert!(!error.ok());
        assert!(!error.has_body());
        assert!(error.headers.is_empty());

        let response = Response {
            response_type: ResponseType::Basic,
            status: 204,
            status_text: "No Content".into(),
            headers: crate::Headers::with_guard(crate::HeaderGuard::Immutable),
            url: "https://example.test/".into(),
            redirected: false,
            body: Body::null(),
            from_cache: false,
        };
        assert!(response.ok());
        assert!(!response.has_body());
    }
}
