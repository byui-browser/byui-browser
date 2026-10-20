//! Browser adapter from security-owned file access to `net::FileProvider`.

use std::sync::Arc;

use net::{
    FileProvider, HeaderName, HeaderValue, Origin, ProviderErrorKind, ProviderFuture, Request,
    RequestError, SchemeBody, SchemeResource, Url,
};
use security::file_access::{
    FileAccessError, FileAccessPermission, FileAccessPolicy, FileOrigin, FileRequestContext,
    FileRequestKind, FileSandboxId, SharedFileSystem,
};

/// Explicit browser-owned grant used when composing a local-file capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAccessGrant {
    permission: FileAccessPermission,
    sandbox: FileSandboxId,
}

impl FileAccessGrant {
    /// Creates the permission and sandbox state for a local-file capability.
    pub fn new(permission: FileAccessPermission, sandbox: FileSandboxId) -> Self {
        Self {
            permission,
            sandbox,
        }
    }
}

/// Adapts security policy and filesystem access to the network provider API.
#[derive(Clone)]
pub struct BrowserFileProvider {
    policy: Arc<FileAccessPolicy>,
    filesystem: SharedFileSystem,
    grant: FileAccessGrant,
}

impl BrowserFileProvider {
    /// Creates a provider using the supplied security policy and filesystem.
    pub fn new(
        policy: Arc<FileAccessPolicy>,
        filesystem: SharedFileSystem,
        grant: FileAccessGrant,
    ) -> Self {
        Self {
            policy,
            filesystem,
            grant,
        }
    }
}

impl FileProvider for BrowserFileProvider {
    fn open<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let result = (|| {
            self.policy
                .authorize_request(&request_context(request, &self.grant))
                .map_err(provider_error)?;
            let path = url
                .to_file_path()
                .map_err(|_| provider_error(FileAccessError::InvalidPath))?;
            let resource = self
                .policy
                .open_file(path, self.filesystem.as_ref())
                .and_then(|opened| opened.read_all())
                .map_err(provider_error)?;
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

/// Translates networking request state into the security crate's file-access context.
fn request_context(request: &Request, grant: &FileAccessGrant) -> FileRequestContext {
    let initiating_origin = match request.environment().origin.as_ref() {
        None => FileOrigin::Absent,
        Some(origin) if origin.is_opaque() => FileOrigin::Opaque,
        Some(origin) => FileOrigin::Tuple(origin.as_str()),
    };
    let kind = if request.mode() == net::RequestMode::Navigate {
        FileRequestKind::Navigation
    } else {
        FileRequestKind::Subresource
    };
    FileRequestContext {
        initiating_origin,
        requested_origin: FileOrigin::LocalFile,
        kind,
        permission: grant.permission,
        sandbox: grant.sandbox.clone(),
    }
}

fn provider_error(error: FileAccessError) -> RequestError {
    let kind = match error {
        FileAccessError::Missing => ProviderErrorKind::ResourceMissing,
        FileAccessError::Denied => ProviderErrorKind::AccessDenied,
        FileAccessError::InvalidPath => ProviderErrorKind::MalformedMetadata,
        FileAccessError::UnsupportedFile => ProviderErrorKind::Io,
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
    grant: FileAccessGrant,
) -> Arc<dyn FileProvider> {
    Arc::new(BrowserFileProvider::new(policy, filesystem, grant))
}
