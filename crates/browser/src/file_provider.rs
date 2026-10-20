//! Browser adapter from security-owned file access to `net::FileProvider`.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use net::{
    FileProvider, HeaderName, HeaderValue, Origin, ProviderErrorKind, ProviderFuture, Request,
    RequestError, SchemeBody, SchemeResource, Url,
};
use security::file_access::{
    FileAccessError, FileAccessPermission, FileAccessPolicy, FileOrigin, FileRequestContext,
    FileRequestKind, FileSandboxId, SharedFileSystem,
};
use tokio::sync::Semaphore;

const FILE_READ_CHUNK_SIZE: usize = 64 * 1024;
const MAX_CONCURRENT_FILE_READS: usize = 4;

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
    read_slots: Arc<Semaphore>,
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
            read_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_FILE_READS)),
        }
    }
}

impl FileProvider for BrowserFileProvider {
    fn open<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let policy = Arc::clone(&self.policy);
        let filesystem = Arc::clone(&self.filesystem);
        let grant = self.grant.clone();
        let read_slots = Arc::clone(&self.read_slots);
        let url = url.clone();
        let is_head = request.method() == net::Method::HEAD;
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = CancellationFlag(Arc::clone(&cancelled));

        Box::pin(async move {
            let _cancellation = cancellation;
            let permit = read_slots
                .acquire_owned()
                .await
                .map_err(|_| provider_io_error())?;
            let path = url
                .to_file_path()
                .map_err(|_| provider_error(FileAccessError::InvalidPath))?;
            let context = request_context(request, &grant);
            let task = tokio::task::spawn_blocking(move || {
                if cancelled.load(Ordering::Acquire) {
                    return Err(RequestError::Aborted);
                }
                policy.authorize_request(&context).map_err(provider_error)?;
                let mut opened = policy
                    .open_file(path, filesystem.as_ref())
                    .map_err(provider_error)?;
                let length = opened.length();
                let media_type = if is_head {
                    opened.media_type().map(str::to_owned)
                } else {
                    let mut bytes = Vec::new();
                    let mut buffer = [0; FILE_READ_CHUNK_SIZE];
                    loop {
                        if cancelled.load(Ordering::Acquire) {
                            return Err(RequestError::Aborted);
                        }
                        match opened.read_chunk(&mut buffer).map_err(provider_error)? {
                            0 => break,
                            count => bytes.extend_from_slice(&buffer[..count]),
                        }
                    }
                    return Ok((length, opened.media_type().map(str::to_owned), bytes));
                };
                Ok((length, media_type, Vec::new()))
            });
            let (length, media_type, bytes) = task.await.map_err(|_| provider_io_error())??;
            drop(permit);

            let length =
                HeaderValue::from_str(&length.to_string()).map_err(|_| RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::MalformedMetadata,
                })?;
            let mut headers = vec![(HeaderName::from_static("content-length"), length)];
            let media_type = media_type.and_then(|value| HeaderValue::from_str(&value).ok());
            if let Some(media_type) = media_type.clone() {
                headers.push((HeaderName::from_static("content-type"), media_type));
            }
            Ok(SchemeResource {
                url,
                status: 200,
                status_text: Some("OK".into()),
                headers,
                media_type,
                origin: Origin::opaque(),
                body: if is_head {
                    SchemeBody::Null
                } else {
                    SchemeBody::Bytes(bytes)
                },
            })
        })
    }
}

struct CancellationFlag(Arc<AtomicBool>);

impl Drop for CancellationFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn provider_io_error() -> RequestError {
    RequestError::Provider {
        provider: "file",
        kind: ProviderErrorKind::Io,
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
