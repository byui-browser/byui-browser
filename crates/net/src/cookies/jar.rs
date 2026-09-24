//! Cookie-store boundary. Storage and matching rules belong to security/storage.

use reqwest::header::HeaderMap;

use crate::{error::RequestError, request::Request};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CookieStore;

impl CookieStore {
    pub(crate) fn attach(&self, _request: &mut Request) -> Result<(), RequestError> {
        Ok(())
    }

    pub(crate) fn process_response(
        &self,
        _request: &Request,
        _headers: &HeaderMap,
    ) -> Result<(), RequestError> {
        Ok(())
    }
}
