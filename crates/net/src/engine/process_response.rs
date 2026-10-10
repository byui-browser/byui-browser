//! Browser-service response processing.

use crate::{
    api::{
        error::RequestError,
        request::{HeaderList, Request},
        response::InternalResponse,
        services::ResponseInfo,
    },
    engine::RequestController,
};

impl RequestController {
    pub(crate) fn approve_response(
        &self,
        request: &Request,
        response: &InternalResponse,
    ) -> Result<(), RequestError> {
        if let Some(services) = &self.inner.services {
            let info = ResponseInfo {
                status: response.status.as_u16(),
                status_text: response.status_text.clone(),
                url: response.url_list.last().cloned().unwrap_or_default(),
                headers: HeaderList::internal_response(&response.headers),
                response_origin: response.origin.clone(),
                request_origin: response.request_origin.clone(),
                request_mode: response.request_mode,
                response_type: response.response_type,
                redirect_count: response.redirect_count,
                from_cache: response.from_cache,
            };
            services.check_response(request, &info)?;
            services.response_headers(request, &info);
        }
        Ok(())
    }
}
