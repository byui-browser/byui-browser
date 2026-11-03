use reqwest::{StatusCode, header::HeaderMap};

use super::{Response, ResponseType};
use crate::api::{
    HeaderGuard, Headers,
    body::Body,
    providers::SchemeResource,
    request::{CredentialsMode, Origin, Request, RequestMode},
};
use crate::policy::FetchScheme;

/// Complete response state retained inside the Fetch engine.
///
/// Policy and cookie processing use this state before a filtered public view
/// is built. The body owns its transport stream and scheduler permit.
pub(crate) struct InternalResponse {
    pub(crate) status: StatusCode,
    pub(crate) status_text: String,
    pub(crate) headers: HeaderMap,
    pub(crate) url_list: Vec<String>,
    pub(crate) redirect_count: usize,
    pub(crate) origin: Option<Origin>,
    pub(crate) request_origin: Option<Origin>,
    pub(crate) request_mode: RequestMode,
    pub(crate) credentials_mode: CredentialsMode,
    pub(crate) response_type: ResponseType,
    pub(crate) body: Body,
    pub(crate) body_is_null: bool,
    pub(crate) from_cache: bool,
    pub(crate) cookie_headers_processed: bool,
}

impl InternalResponse {
    /// Converts provider-owned resource metadata into the internal response
    /// representation used by the Fetch policy and exposure pipeline.
    pub(crate) fn from_scheme_resource(
        resource: SchemeResource,
        request: &Request,
        expected_scheme: FetchScheme,
    ) -> Result<Self, crate::RequestError> {
        validate_scheme_resource(&resource, expected_scheme)?;
        let status = StatusCode::from_u16(resource.status).map_err(|error| {
            crate::RequestError::InvalidResponse(format!(
                "scheme provider returned invalid status {}: {error}",
                resource.status
            ))
        })?;
        let mut headers = HeaderMap::new();
        for (name, value) in resource.headers {
            headers.append(name, value);
        }
        let content_type = resource
            .media_type
            .or_else(|| headers.get(http::header::CONTENT_TYPE).cloned());
        if let Some(content_type) = &content_type {
            headers
                .entry(http::header::CONTENT_TYPE)
                .or_insert_with(|| content_type.clone());
        }
        let response_forbids_body = matches!(status.as_u16(), 101 | 204 | 205 | 304);
        if !response_forbids_body {
            if let Some(length) = resource.body.known_length() {
                headers
                    .entry(http::header::CONTENT_LENGTH)
                    .or_insert_with(|| {
                        http::HeaderValue::from_str(&length.to_string())
                            .expect("a byte length is a valid header value")
                    });
            }
        }
        let force_null = request.method() == http::Method::HEAD || response_forbids_body;
        let (mut body, body_is_null) = resource
            .body
            .into_body(request.signal().clone(), force_null);
        body.set_content_type(content_type);
        let status_text = resource
            .status_text
            .unwrap_or_else(|| status.canonical_reason().unwrap_or_default().to_owned());

        Ok(Self {
            status,
            status_text,
            headers,
            url_list: vec![resource.url.to_string()],
            redirect_count: 0,
            origin: Some(resource.origin),
            request_origin: request.context.environment.origin.clone(),
            request_mode: request.context.mode,
            credentials_mode: request.context.credentials,
            response_type: ResponseType::Basic,
            body,
            body_is_null,
            from_cache: false,
            cookie_headers_processed: false,
        })
    }

    /// Builds the public filtered response according to its response type.
    pub(crate) fn expose(self) -> Response {
        let hidden = matches!(
            self.response_type,
            ResponseType::Opaque | ResponseType::OpaqueRedirect | ResponseType::Error
        );
        let headers = if hidden {
            Headers::with_guard(HeaderGuard::Immutable)
        } else {
            Headers::exposed_response(
                &self.headers,
                self.response_type == ResponseType::Cors,
                self.credentials_mode,
            )
        };
        Response {
            response_type: self.response_type,
            status: if hidden { 0 } else { self.status.as_u16() },
            status_text: if hidden {
                String::new()
            } else {
                self.status_text
            },
            headers,
            url: if hidden {
                String::new()
            } else {
                self.url_list.last().cloned().unwrap_or_default()
            },
            redirected: !hidden && self.redirect_count > 0,
            body: if hidden || self.body_is_null {
                Body::null()
            } else {
                self.body
            },
            from_cache: !hidden && self.from_cache,
        }
    }
}

/// Validates provider metadata before it enters the policy and exposure pipeline.
fn validate_scheme_resource(
    resource: &SchemeResource,
    expected_scheme: FetchScheme,
) -> Result<(), crate::RequestError> {
    let expected = match expected_scheme {
        FetchScheme::Http => "http",
        FetchScheme::Https => "https",
        FetchScheme::Data => "data",
        FetchScheme::Blob => "blob",
        FetchScheme::File => "file",
        FetchScheme::Byui => "byui",
        FetchScheme::About => "about",
    };
    if resource.url.scheme() != expected {
        return Err(crate::RequestError::Provider {
            provider: expected,
            kind: crate::ProviderErrorKind::MalformedMetadata,
        });
    }

    if resource
        .status_text
        .as_deref()
        .is_some_and(|text| text.bytes().any(|byte| byte < 0x20 || byte == 0x7f))
    {
        return Err(crate::RequestError::Provider {
            provider: expected,
            kind: crate::ProviderErrorKind::MalformedMetadata,
        });
    }

    let header_media_type = resource
        .headers
        .iter()
        .find(|(name, _)| name == http::header::CONTENT_TYPE)
        .map(|(_, value)| value);
    if let Some(media_type) = &resource.media_type {
        if !valid_media_type(media_type)
            || header_media_type.is_some_and(|header| header != media_type)
        {
            return Err(crate::RequestError::Provider {
                provider: expected,
                kind: crate::ProviderErrorKind::MalformedMetadata,
            });
        }
    }
    if header_media_type.is_some_and(|value| !valid_media_type(value)) {
        return Err(crate::RequestError::Provider {
            provider: expected,
            kind: crate::ProviderErrorKind::MalformedMetadata,
        });
    }

    if !resource.origin.is_opaque()
        && crate::api::request::Origin::parse(&resource.origin.as_str()).is_err()
    {
        return Err(crate::RequestError::Provider {
            provider: expected,
            kind: crate::ProviderErrorKind::InvalidOrigin,
        });
    }
    Ok(())
}

/// Checks the syntax used for provider and response `Content-Type` metadata.
fn valid_media_type(value: &http::HeaderValue) -> bool {
    let Ok(value) = value.to_str() else {
        return false;
    };
    let mut parts = value.split(';');
    let Some(essence) = parts.next() else {
        return false;
    };
    let Some((kind, subtype)) = essence.trim().split_once('/') else {
        return false;
    };
    if kind.is_empty()
        || subtype.is_empty()
        || kind.bytes().any(|byte| byte.is_ascii_whitespace())
        || subtype.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return false;
    }
    parts.all(|parameter| {
        parameter
            .split_once('=')
            .is_some_and(|(name, value)| !name.trim().is_empty() && !value.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::providers::{SchemeBody, SchemeResource};

    /// Builds minimal provider metadata for response-view tests.
    fn resource(url: &str) -> SchemeResource {
        SchemeResource {
            url: reqwest::Url::parse(url).unwrap(),
            status: 200,
            status_text: Some("OK".into()),
            headers: Vec::new(),
            media_type: None,
            origin: Origin::opaque(),
            body: SchemeBody::Bytes(Vec::new()),
        }
    }

    #[test]
    /// Verifies a provider cannot return metadata for another scheme.
    fn scheme_metadata_must_match_the_selected_dispatcher() {
        let request = Request::get("data:,ok");
        let result = InternalResponse::from_scheme_resource(
            resource("blob:https://example.test/id"),
            &request,
            FetchScheme::Data,
        );

        assert!(matches!(
            result,
            Err(crate::RequestError::Provider {
                provider: "data",
                kind: crate::ProviderErrorKind::MalformedMetadata
            })
        ));
    }

    #[test]
    /// Verifies conflicting media-type metadata is rejected.
    fn conflicting_media_type_metadata_fails_closed() {
        let mut resource = resource("data:,ok");
        resource.media_type = Some("text/plain".parse().unwrap());
        resource.headers.push((
            http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        ));
        let result = InternalResponse::from_scheme_resource(
            resource,
            &Request::get("data:,ok"),
            FetchScheme::Data,
        );

        assert!(matches!(
            result,
            Err(crate::RequestError::Provider {
                provider: "data",
                kind: crate::ProviderErrorKind::MalformedMetadata
            })
        ));
    }

    #[test]
    fn scheme_resources_accept_each_supported_scheme() {
        let request = Request::get("https://client.test/");
        for (url, scheme) in [
            ("http://example.test/resource", FetchScheme::Http),
            ("https://example.test/resource", FetchScheme::Https),
            ("data:,resource", FetchScheme::Data),
            ("blob:https://example.test/id", FetchScheme::Blob),
            ("file:///tmp/resource", FetchScheme::File),
            ("byui://welcome/page", FetchScheme::Byui),
            ("about:blank", FetchScheme::About),
        ] {
            assert!(
                InternalResponse::from_scheme_resource(resource(url), &request, scheme).is_ok(),
                "{url} should match {scheme:?}"
            );
        }
    }

    #[test]
    fn scheme_resource_rejects_invalid_status_text_media_type_and_status() {
        let request = Request::get("data:,ok");

        let mut invalid_status_text = resource("data:,ok");
        invalid_status_text.status_text = Some("not\nok".into());
        assert!(matches!(
            InternalResponse::from_scheme_resource(
                invalid_status_text,
                &request,
                FetchScheme::Data
            ),
            Err(crate::RequestError::Provider {
                provider: "data",
                kind: crate::ProviderErrorKind::MalformedMetadata
            })
        ));

        let mut invalid_media_type = resource("data:,ok");
        invalid_media_type.headers.push((
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("not-a-media-type"),
        ));
        assert!(matches!(
            InternalResponse::from_scheme_resource(invalid_media_type, &request, FetchScheme::Data),
            Err(crate::RequestError::Provider {
                provider: "data",
                kind: crate::ProviderErrorKind::MalformedMetadata
            })
        ));

        let mut invalid_status = resource("data:,ok");
        invalid_status.status = 99;
        assert!(matches!(
            InternalResponse::from_scheme_resource(invalid_status, &request, FetchScheme::Data),
            Err(crate::RequestError::InvalidResponse(_))
        ));
    }

    #[test]
    fn scheme_resource_preserves_bytes_streams_and_explicit_null_bodies() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let request = Request::get("data:,ok");

        let mut bytes = resource("data:,ok");
        bytes.body = SchemeBody::Bytes(b"bytes".to_vec());
        let bytes = InternalResponse::from_scheme_resource(bytes, &request, FetchScheme::Data)
            .unwrap()
            .expose();
        assert_eq!(runtime.block_on(bytes.body.bytes()).unwrap(), b"bytes");

        let mut stream = resource("data:,ok");
        stream.body = SchemeBody::Stream(Box::pin(futures_util::stream::iter(vec![
            Ok(b"stream-".to_vec()),
            Ok(b"body".to_vec()),
        ])));
        let stream = InternalResponse::from_scheme_resource(stream, &request, FetchScheme::Data)
            .unwrap()
            .expose();
        assert_eq!(
            runtime.block_on(stream.body.bytes()).unwrap(),
            b"stream-body"
        );

        let mut null = resource("data:,ok");
        null.body = SchemeBody::Null;
        let null = InternalResponse::from_scheme_resource(null, &request, FetchScheme::Data)
            .unwrap()
            .expose();
        assert!(null.body.is_null());
        assert!(runtime.block_on(null.body.bytes()).unwrap().is_empty());
    }

    #[test]
    fn scheme_resources_synthesize_lengths_for_known_representations() {
        let request = Request::get("data:,ok");

        let mut bytes = resource("data:,ok");
        bytes.body = SchemeBody::Bytes(b"bytes".to_vec());
        let bytes = InternalResponse::from_scheme_resource(bytes, &request, FetchScheme::Data)
            .unwrap()
            .expose();
        assert_eq!(bytes.headers.get("content-length").as_deref(), Some("5"));

        let mut metadata_only = resource("data:,ok");
        metadata_only.body = SchemeBody::NullWithLength(7);
        let metadata_only =
            InternalResponse::from_scheme_resource(metadata_only, &request, FetchScheme::Data)
                .unwrap()
                .expose();
        assert_eq!(
            metadata_only.headers.get("content-length").as_deref(),
            Some("7")
        );
        assert!(metadata_only.body.is_null());

        let mut stream = resource("data:,ok");
        stream.body = SchemeBody::Stream(Box::pin(futures_util::stream::empty()));
        let stream = InternalResponse::from_scheme_resource(stream, &request, FetchScheme::Data)
            .unwrap()
            .expose();
        assert!(!stream.headers.has("content-length"));
    }

    #[test]
    fn scheme_resource_length_preserves_explicit_metadata_and_bodyless_status_rules() {
        let request = Request::get("data:,ok");

        let mut explicit = resource("data:,ok");
        explicit.body = SchemeBody::Bytes(b"bytes".to_vec());
        explicit.headers.push((
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_static("9"),
        ));
        let explicit =
            InternalResponse::from_scheme_resource(explicit, &request, FetchScheme::Data)
                .unwrap()
                .expose();
        assert_eq!(explicit.headers.get("content-length").as_deref(), Some("9"));

        for status in [204, 205, 304] {
            let mut bodyless = resource("data:,ok");
            bodyless.status = status;
            bodyless.body = SchemeBody::Bytes(b"bytes".to_vec());
            let bodyless =
                InternalResponse::from_scheme_resource(bodyless, &request, FetchScheme::Data)
                    .unwrap()
                    .expose();
            assert!(!bodyless.headers.has("content-length"));
            assert!(bodyless.body.is_null());
        }
    }
}
