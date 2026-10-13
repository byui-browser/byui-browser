use reqwest::{StatusCode, header::HeaderMap};

use super::{Response, ResponseType};
use crate::api::{
    HeaderGuard, Headers,
    body::Body,
    providers::SchemeResource,
    request::{CredentialsMode, Origin, Request, RequestMode},
};

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
    ) -> Result<Self, crate::RequestError> {
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
