use reqwest::{StatusCode, header::HeaderMap};

use super::ResponseType;
use crate::api::{
    body::Body,
    request::{CredentialsMode, FetchContext, Origin, RequestMode},
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
    pub(crate) from_cache: bool,
    pub(crate) cookie_headers_processed: bool,
}

impl InternalResponse {
    /// Builds internal HTTP response state from a transport or cache result.
    pub(crate) fn from_http_parts(
        status: StatusCode,
        headers: HeaderMap,
        url: String,
        context: &FetchContext,
        body: Body,
        from_cache: bool,
        cookie_headers_processed: bool,
    ) -> Self {
        let origin = reqwest::Url::parse(&url)
            .ok()
            .and_then(|url| Origin::from_url(&url).ok());
        Self {
            status,
            status_text: status.canonical_reason().unwrap_or_default().to_owned(),
            headers,
            url_list: vec![url],
            redirect_count: 0,
            origin,
            request_origin: context.environment.origin.clone(),
            request_mode: context.mode,
            credentials_mode: context.credentials,
            response_type: ResponseType::Basic,
            body,
            from_cache,
            cookie_headers_processed,
        }
    }
}
