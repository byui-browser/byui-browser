//! Contract for browser-owned `blob:` resources.

use url::Url;

use crate::api::{providers::ProviderFuture, request::Request};

/// Resolves browser-owned `blob:` URLs into provider-neutral resource data.
pub trait BlobProvider: Send + Sync {
    /// Resolves `url` for `request`, or returns a typed missing/revoked error.
    fn resolve<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a>;
}
