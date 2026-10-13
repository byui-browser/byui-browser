//! Response approval helpers.

use crate::api::{error::RequestError, request::Request, response::InternalResponse};

pub(crate) fn validate_response(
    _request: &Request,
    _response: &InternalResponse,
) -> Result<(), RequestError> {
    Ok(())
}
