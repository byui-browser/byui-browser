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
        let (mut body, body_is_null) = resource.body.into_body();
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
}
