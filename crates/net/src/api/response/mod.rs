//! Response types returned by the networking client.

mod types;
mod view;

pub use types::{Response, ResponseType};
pub(crate) use view::InternalResponse;

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures_util::{StreamExt, stream};
    use reqwest::{StatusCode, header::HeaderMap};

    use super::*;
    use crate::{AbortController, Body, RequestError, RequestMode};

    #[test]
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
            body_is_null: false,
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
                body: Body::once(b"forbidden".to_vec()),
                body_is_null,
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
}
