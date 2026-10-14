//! Browser-owned resource results used by scheme-specific Fetch providers.

use std::{future::Future, pin::Pin, sync::Arc};

use crate::api::error::RequestError;

mod blob;
mod body;
mod file;
mod page;

pub use blob::BlobProvider;
pub use body::{ProviderBodyStream, SchemeBody, SchemeResource};
pub use file::FileProvider;
pub use page::PageProvider;

/// A boxed asynchronous provider operation.
pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SchemeResource, RequestError>> + Send + 'a>>;

/// Browser-owned providers installed beside [`FetchServices`](crate::FetchServices).
///
/// Providers are optional capabilities. They are held by shared, thread-safe
/// references and are selected strictly by URL scheme; `net` never performs a
/// global registry lookup or opens files itself.
#[derive(Clone, Default)]
pub struct SchemeProviders {
    blob: Option<Arc<dyn BlobProvider>>,
    file: Option<Arc<dyn FileProvider>>,
    page: Option<Arc<dyn PageProvider>>,
}

impl SchemeProviders {
    /// Creates an empty provider bundle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs the provider for `blob:` URLs.
    pub fn with_blob(mut self, provider: Arc<dyn BlobProvider>) -> Self {
        self.blob = Some(provider);
        self
    }

    /// Installs the provider for `file:` URLs.
    pub fn with_file(mut self, provider: Arc<dyn FileProvider>) -> Self {
        self.file = Some(provider);
        self
    }

    /// Installs the provider for `byui:` URLs.
    pub fn with_page(mut self, provider: Arc<dyn PageProvider>) -> Self {
        self.page = Some(provider);
        self
    }

    /// Returns the installed `blob:` provider, if any.
    pub(crate) fn blob(&self) -> Option<&Arc<dyn BlobProvider>> {
        self.blob.as_ref()
    }
    /// Returns the installed `file:` provider, if any.
    pub(crate) fn file(&self) -> Option<&Arc<dyn FileProvider>> {
        self.file.as_ref()
    }
    /// Returns the installed `byui:` page provider, if any.
    pub(crate) fn page(&self) -> Option<&Arc<dyn PageProvider>> {
        self.page.as_ref()
    }
}
