//! Contract for browser-owned `blob:` resources.

use url::Url;

use crate::api::{providers::ProviderFuture, request::Request};

/// Resolves browser-owned `blob:` URLs into provider-neutral resource data.
pub trait BlobProvider: Send + Sync {
    /// Resolves `url` for `request`, or returns a typed missing/revoked error.
    ///
    /// The operation is cancelled by dropping its future when the request's
    /// signal is aborted. Any returned stream follows the same request signal.
    fn resolve<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a>;
}
