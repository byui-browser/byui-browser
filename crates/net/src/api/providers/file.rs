//! Contract for browser-authorized `file:` resources.

use url::Url;

use crate::api::{providers::ProviderFuture, request::Request};

/// Authorizes and opens browser-owned `file:` URLs.
pub trait FileProvider: Send + Sync {
    /// Opens `url` for `request` after applying filesystem policy.
    ///
    /// The operation is cancelled by dropping its future when the request's
    /// signal is aborted. Any returned stream follows the same request signal.
    fn open<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a>;
}
