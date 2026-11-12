//! Response approval helpers.

use crate::api::{error::RequestError, request::Request, response::InternalResponse};

/// Validates response metadata that is shared by all dispatch schemes.
pub(crate) fn validate_response(
    _request: &Request,
    _response: &InternalResponse,
) -> Result<(), RequestError> {
    Ok(())
}
