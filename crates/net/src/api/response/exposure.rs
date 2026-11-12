use super::{InternalResponse, Response, ResponseType};
use crate::api::{HeaderGuard, Headers, body::Body};

impl InternalResponse {
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
            body: if hidden || self.body.is_null() {
                Body::null()
            } else {
                self.body
            },
            from_cache: !hidden && self.from_cache,
        }
    }
}
