//! CORS validation boundary. The initial implementation preserves responses;
//! the policy can be made strict without changing the controller API.

use crate::{error::RequestError, request::Request, response::Response};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CorsChecker;

impl CorsChecker {
    pub(crate) fn validate(
        &self,
        _request: &Request,
        _response: &Response,
    ) -> Result<(), RequestError> {
        Ok(())
    }
}
