//! Cookie-store boundary. Storage and matching rules belong to security/storage.

use reqwest::header::HeaderMap;

use crate::{error::RequestError, request::Request};

/// Cookie-jar integration point for request and response processing.
///
/// The type is currently stateless and intentionally preserves the boundary
/// where persistent storage and domain/path matching will be implemented.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CookieStore;

impl CookieStore {
    /// Adds cookies applicable to a request before transport execution.
    ///
    /// This initial implementation leaves the request unchanged and succeeds;
    /// callers can already depend on the future cookie-processing boundary.
    pub(crate) fn attach(&self, _request: &mut Request) -> Result<(), RequestError> {
        Ok(())
    }

    /// Records cookies received from a response.
    ///
    /// Cookie persistence is not implemented yet, so response headers are
    /// currently ignored.
    pub(crate) fn process_response(
        &self,
        _request: &Request,
        _headers: &HeaderMap,
    ) -> Result<(), RequestError> {
        Ok(())
    }
}
