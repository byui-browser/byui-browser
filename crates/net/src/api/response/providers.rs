use reqwest::{StatusCode, header::HeaderMap};

use super::{InternalResponse, ResponseType, has_null_body};
use crate::api::{media_type::is_valid_media_type, providers::SchemeResource, request::Request};
use crate::policy::FetchScheme;

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
            log::warn!(
                "scheme provider returned invalid status {}",
                resource.status
            );
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
        if !response_forbids_body && let Some(length) = resource.body.known_length() {
            headers
                .entry(http::header::CONTENT_LENGTH)
                .or_insert_with(|| {
                    http::HeaderValue::from_str(&length.to_string())
                        .expect("a byte length is a valid header value")
                });
        }
        let force_null = has_null_body(request.method(), status) || response_forbids_body;
        let mut body = resource
            .body
            .into_body(request.signal().clone(), force_null);
        body.set_response_metadata(&headers, None);
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
            request_origin: request.effective_origin().cloned(),
            request_mode: request.context.mode,
            credentials_mode: request.context.credentials,
            response_type: ResponseType::Basic,
            body,
            from_cache: false,
            cookie_headers_processed: false,
        })
    }
}

/// Validates provider metadata before it enters the policy and exposure pipeline.
fn validate_scheme_resource(
    resource: &SchemeResource,
    expected_scheme: FetchScheme,
) -> Result<(), crate::RequestError> {
    let expected = expected_scheme.name();
    if resource.url.scheme() != expected {
        log::warn!(
            "{expected} provider returned a url with scheme {:?}, expected {expected}",
            resource.url.scheme()
        );
        return Err(provider_error(
            expected,
            crate::ProviderErrorKind::MalformedMetadata,
        ));
    }

    if resource
        .status_text
        .as_deref()
        .is_some_and(|text| text.bytes().any(|byte| byte < 0x20 || byte == 0x7f))
    {
        log::warn!("{expected} provider returned a status text containing control characters");
        return Err(provider_error(
            expected,
            crate::ProviderErrorKind::MalformedMetadata,
        ));
    }

    let header_media_type = resource
        .headers
        .iter()
        .find(|(name, _)| name == http::header::CONTENT_TYPE)
        .map(|(_, value)| value);
    if let Some(media_type) = &resource.media_type
        && (!is_valid_media_type(media_type)
            || header_media_type.is_some_and(|header| header != media_type))
    {
        log::warn!(
            "{expected} provider returned a media type that is invalid or differs from its content-type header"
        );
        return Err(provider_error(
            expected,
            crate::ProviderErrorKind::MalformedMetadata,
        ));
    }
    if header_media_type.is_some_and(|value| !is_valid_media_type(value)) {
        log::warn!("{expected} provider returned an invalid content-type header");
        return Err(provider_error(
            expected,
            crate::ProviderErrorKind::MalformedMetadata,
        ));
    }

    if !resource.origin.is_opaque()
        && crate::api::request::Origin::parse(&resource.origin.as_str()).is_err()
    {
        log::warn!("{expected} provider returned an unparseable origin");
        return Err(provider_error(
            expected,
            crate::ProviderErrorKind::InvalidOrigin,
        ));
    }
    Ok(())
}

/// Constructs a stable provider validation error.
fn provider_error(provider: &'static str, kind: crate::ProviderErrorKind) -> crate::RequestError {
    crate::RequestError::Provider { provider, kind }
}
