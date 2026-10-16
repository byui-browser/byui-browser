//! Contract for browser-owned `byui:` pages.

use url::Url;

use crate::api::{providers::ProviderFuture, request::Request};

/// Resolves registered browser-local `byui:` pages.
pub trait PageProvider: Send + Sync {
    /// Looks up `url` for `request`, or returns a typed missing-page error.
    ///
    /// The operation is cancelled by dropping its future when the request's
    /// signal is aborted. Any returned stream follows the same request signal.
    fn resolve<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a>;
}
