use reqwest::{StatusCode, header::HeaderMap};

use super::{ResponseBody, ResponseType, StreamingResponse};
use crate::api::{
    HeaderGuard, Headers,
    request::{Origin, RequestMode},
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
    pub(crate) response_type: ResponseType,
    pub(crate) body: ResponseBody,
    pub(crate) body_is_null: bool,
    pub(crate) from_cache: bool,
    pub(crate) cookie_headers_processed: bool,
}

impl InternalResponse {
    pub(crate) fn expose(self) -> StreamingResponse {
        let hidden = matches!(
            self.response_type,
            ResponseType::Opaque | ResponseType::OpaqueRedirect | ResponseType::Error
        );
        let headers = if hidden {
            Headers::with_guard(HeaderGuard::Immutable)
        } else {
            Headers::exposed_response(&self.headers, self.response_type == ResponseType::Cors)
        };
        StreamingResponse {
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
                ResponseBody::empty()
            } else {
                self.body
            },
            from_cache: !hidden && self.from_cache,
        }
    }
}
