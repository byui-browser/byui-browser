//! Browser adapter from security-owned file access to `net::FileProvider`.

use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};
use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures_util::Stream;
use net::{
    AbortSignal, FileProvider, HeaderName, HeaderValue, Origin, ProviderErrorKind, ProviderFuture,
    Request, RequestError, SchemeBody, SchemeResource, Url,
};
use security::file_access::{
    FileAccessCapability, FileAccessError, FileOrigin, FileRequestContext, FileRequestKind,
};
use tokio::sync::{Semaphore, mpsc};

pub use security::file_access::FileAccessGrant;

const FILE_READ_CHUNK_SIZE: usize = 64 * 1024;
const MAX_CONCURRENT_FILE_READS: usize = 4;
const FILE_READ_CHANNEL_CAPACITY: usize = 2;
const BACKPRESSURE_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Mutable browser-owned source for local-file authorization state.
///
/// Providers snapshot this source immediately before request authorization.
/// Revocation therefore denies subsequent opens, while an already opened file
/// stream retains its existing handle until it completes, fails, or is dropped.
#[derive(Clone, Debug)]
pub struct FileAccessGrantSource {
    grant: Arc<RwLock<Option<FileAccessGrant>>>,
}

impl FileAccessGrantSource {
    /// Creates a shared source with `grant` as its initial authorization state.
    pub(crate) fn new(grant: FileAccessGrant) -> Self {
        Self {
            grant: Arc::new(RwLock::new(Some(grant))),
        }
    }

    /// Replaces the authorization state used by future file opens.
    ///
    /// A poisoned source fails closed: this operation leaves its previous state
    /// unchanged and future snapshots are denied.
    pub fn replace(&self, grant: FileAccessGrant) {
        if let Ok(mut current) = self.grant.write() {
            *current = Some(grant);
        }
    }

    /// Revokes local-file access for future opens by removing its policy-issued grant.
    pub fn revoke(&self) {
        if let Ok(mut grant) = self.grant.write() {
            *grant = None;
        }
    }

    fn snapshot(&self) -> Option<FileAccessGrant> {
        self.grant.read().map(|grant| grant.clone()).unwrap_or(None)
    }
}

/// Adapts security policy and filesystem access to the network provider API.
#[derive(Clone)]
pub struct BrowserFileProvider {
    capability: Arc<FileAccessCapability>,
    grants: FileAccessGrantSource,
    read_slots: Arc<Semaphore>,
}

impl BrowserFileProvider {
    /// Creates a provider using the supplied browser-owned capability.
    pub fn new(capability: Arc<FileAccessCapability>, grants: FileAccessGrantSource) -> Self {
        Self {
            capability,
            grants,
            read_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_FILE_READS)),
        }
    }
}

impl FileProvider for BrowserFileProvider {
    fn open<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let capability = Arc::clone(&self.capability);
        let grants = self.grants.clone();
        let read_slots = Arc::clone(&self.read_slots);
        let url = url.clone();
        let is_head = request.method() == net::Method::HEAD;
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut cancellation = CancellationGuard::new(Arc::clone(&cancelled));
        let signal = request.signal().clone();
        let preparation_cancelled = Arc::clone(&cancelled);

        Box::pin(async move {
            let permit = read_slots
                .acquire_owned()
                .await
                .map_err(|_| provider_io_error())?;
            let path = url
                .to_file_path()
                .map_err(|_| provider_error(FileAccessError::InvalidPath))?;
            let grant = grants.snapshot();
            let context = request_context(request);
            let task = tokio::task::spawn_blocking(move || {
                if preparation_cancelled.load(Ordering::Acquire) {
                    return Err(RequestError::Aborted);
                }
                let opened = capability
                    .open_authorized_file(
                        &context,
                        grant
                            .as_ref()
                            .ok_or_else(|| provider_error(FileAccessError::Denied))?,
                        path,
                    )
                    .map_err(provider_error)?;
                let length = opened.length();
                let media_type = opened.media_type().map(str::to_owned);
                Ok((opened, length, media_type))
            });
            let (opened, length, media_type) = task.await.map_err(|_| provider_io_error())??;

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
                    drop(opened);
                    drop(permit);
                    SchemeBody::Null
                } else {
                    let (sender, receiver) = mpsc::channel(FILE_READ_CHANNEL_CAPACITY);
                    let worker_cancelled = Arc::clone(&cancelled);
                    tokio::task::spawn_blocking(move || {
                        read_file_stream(opened, sender, permit, worker_cancelled, signal);
                    });
                    cancellation.disarm();
                    SchemeBody::Stream(Box::pin(FileBodyStream {
                        receiver,
                        cancelled,
                    }))
                },
            })
        })
    }
}

/// A body stream that cancels its blocking reader when Fetch discards it.
struct FileBodyStream {
    receiver: mpsc::Receiver<Result<Vec<u8>, RequestError>>,
    cancelled: Arc<AtomicBool>,
}

impl Stream for FileBodyStream {
    type Item = Result<Vec<u8>, RequestError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(context)
    }
}

impl Drop for FileBodyStream {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Stops pending provider work if Fetch drops the resolution future.
struct CancellationGuard {
    cancelled: Arc<AtomicBool>,
    armed: bool,
}

impl CancellationGuard {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

/// Reads an opened file on Tokio's blocking pool and sends bounded body chunks.
fn read_file_stream(
    mut opened: security::file_access::OpenedFile,
    sender: mpsc::Sender<Result<Vec<u8>, RequestError>>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    cancelled: Arc<AtomicBool>,
    signal: AbortSignal,
) {
    let mut buffer = [0; FILE_READ_CHUNK_SIZE];
    loop {
        if cancelled.load(Ordering::Acquire) || signal.is_aborted() {
            return;
        }
        match opened.read_chunk(&mut buffer).map_err(provider_error) {
            Ok(0) => return,
            Ok(count) => {
                if !send_chunk(&sender, Ok(buffer[..count].to_vec()), &cancelled, &signal) {
                    return;
                }
            }
            Err(error) => {
                let _ = send_chunk(&sender, Err(error), &cancelled, &signal);
                return;
            }
        }
    }
}

/// Sends one chunk while honoring cancellation and bounded-channel backpressure.
fn send_chunk(
    sender: &mpsc::Sender<Result<Vec<u8>, RequestError>>,
    mut chunk: Result<Vec<u8>, RequestError>,
    cancelled: &AtomicBool,
    signal: &AbortSignal,
) -> bool {
    loop {
        if cancelled.load(Ordering::Acquire) || signal.is_aborted() {
            return false;
        }
        match sender.try_send(chunk) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(returned)) => {
                chunk = returned;
                std::thread::sleep(BACKPRESSURE_POLL_INTERVAL);
            }
        }
    }
}

fn provider_io_error() -> RequestError {
    RequestError::Provider {
        provider: "file",
        kind: ProviderErrorKind::Io,
    }
}

/// Translates networking request state into the security crate's file-access context.
fn request_context(request: &Request) -> FileRequestContext {
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

/// Creates a provider from one browser-owned file-access capability.
pub fn provider(
    capability: Arc<FileAccessCapability>,
    grants: FileAccessGrantSource,
) -> Arc<dyn FileProvider> {
    Arc::new(BrowserFileProvider::new(capability, grants))
}
