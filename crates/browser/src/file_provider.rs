//! Browser adapter from security-owned file access to `net::FileProvider`.

use std::sync::Arc;

use net::{
    FileProvider, HeaderName, HeaderValue, Origin, ProviderErrorKind, ProviderFuture, Request,
    RequestError, SchemeBody, SchemeResource, Url,
};
use security::file_access::{FileAccessError, FileAccessPolicy, SharedFileSystem};

/// Adapts security policy and filesystem access to the network provider API.
#[derive(Clone)]
pub struct BrowserFileProvider {
    policy: Arc<FileAccessPolicy>,
    filesystem: SharedFileSystem,
}

impl BrowserFileProvider {
    /// Creates a provider using the supplied security policy and filesystem.
    pub fn new(policy: Arc<FileAccessPolicy>, filesystem: SharedFileSystem) -> Self {
        Self { policy, filesystem }
    }
}

impl FileProvider for BrowserFileProvider {
    fn open<'a>(&'a self, _request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let result = (|| {
            let path = url
                .to_file_path()
                .map_err(|_| provider_error(FileAccessError::InvalidPath))?;
            let authorized = self.policy.authorize_path(path).map_err(provider_error)?;
            let resource = self.filesystem.read(&authorized).map_err(provider_error)?;
            let length =
                HeaderValue::from_str(&resource.bytes.len().to_string()).map_err(|_| {
                    RequestError::Provider {
                        provider: "file",
                        kind: ProviderErrorKind::MalformedMetadata,
                    }
                })?;
            let mut headers = vec![(HeaderName::from_static("content-length"), length)];
            let media_type = resource
                .media_type
                .and_then(|value| HeaderValue::from_str(&value).ok());
            if let Some(media_type) = media_type.clone() {
                headers.push((HeaderName::from_static("content-type"), media_type));
            }
            Ok(SchemeResource {
                url: url.clone(),
                status: 200,
                status_text: Some("OK".into()),
                headers,
                media_type,
                origin: Origin::opaque(),
                body: SchemeBody::Bytes(resource.bytes),
            })
        })();

        Box::pin(async move { result })
    }
}

fn provider_error(error: FileAccessError) -> RequestError {
    let kind = match error {
        FileAccessError::Missing => ProviderErrorKind::ResourceMissing,
        FileAccessError::Denied => ProviderErrorKind::AccessDenied,
        FileAccessError::InvalidPath => ProviderErrorKind::MalformedMetadata,
        FileAccessError::Io => ProviderErrorKind::Io,
    };
    RequestError::Provider {
        provider: "file",
        kind,
    }
}

/// Creates a provider from a policy and filesystem backend.
pub fn provider(
    policy: Arc<FileAccessPolicy>,
    filesystem: SharedFileSystem,
) -> Arc<dyn FileProvider> {
    Arc::new(BrowserFileProvider::new(policy, filesystem))
}
