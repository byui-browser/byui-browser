//! CORS validation boundary. The initial implementation preserves responses;
//! the policy can be made strict without changing the controller API.

use crate::{error::RequestError, request::Request, response::Response};

/// Cross-origin validation boundary used by the request controller.
///
/// The current checker is permissive. Keeping it as a separate component lets
/// strict CORS behavior be added without moving policy decisions into the HTTP
/// transport.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CorsChecker;

impl CorsChecker {
    /// Validates a response against the request's cross-origin context.
    ///
    /// This initial implementation accepts every response and leaves the
    /// request and response available for the future validator.
    pub(crate) fn validate(
        &self,
        _request: &Request,
        _response: &Response,
    ) -> Result<(), RequestError> {
        Ok(())
    }
}
