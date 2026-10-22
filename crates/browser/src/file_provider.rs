//! Browser adapter from security-owned file access to `net::FileProvider`.

use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};
use std::{
    collections::HashSet,
    fmt,
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
const MAX_PENDING_FILE_NAVIGATIONS: usize = 64;

/// Failure while preparing a browser-authorized local-file navigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileNavigationError {
    /// The browser has reached its bounded number of prepared navigations.
    PendingLimitReached {
        /// Maximum number of navigation authorizations that may await use.
        limit: usize,
    },
    /// Browser-owned navigation state could not be read safely.
    Unavailable,
}

impl fmt::Display for FileNavigationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PendingLimitReached { limit } => {
                write!(
                    formatter,
                    "too many pending file navigations (limit: {limit})"
                )
            }
            Self::Unavailable => {
                formatter.write_str("file navigation authorization is unavailable")
            }
        }
    }
}

impl std::error::Error for FileNavigationError {}

/// Browser-owned, single-use authorizations for local-file navigations.
#[derive(Debug)]
pub(crate) struct FileNavigationAuthorizations {
    pending: std::sync::Mutex<HashSet<u64>>,
}

impl FileNavigationAuthorizations {
    pub(crate) fn new() -> Self {
        Self {
            pending: std::sync::Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn authorize(&self, request_id: u64) -> Result<(), FileNavigationError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| FileNavigationError::Unavailable)?;
        if pending.len() == MAX_PENDING_FILE_NAVIGATIONS {
            return Err(FileNavigationError::PendingLimitReached {
                limit: MAX_PENDING_FILE_NAVIGATIONS,
            });
        }
        pending.insert(request_id);
        Ok(())
    }

    pub(crate) fn consume(&self, request_id: u64) -> bool {
        self.pending
            .lock()
            .is_ok_and(|mut pending| pending.remove(&request_id))
    }

    fn clear(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }
}

/// Mutable browser-owned source for local-file authorization state.
///
/// Providers snapshot this source immediately before request authorization.
/// Revocation therefore denies subsequent opens, while an already opened file
/// stream retains its existing handle until it completes, fails, or is dropped.
#[derive(Clone, Debug)]
pub struct FileAccessGrantSource {
    grant: Arc<RwLock<Option<FileAccessGrant>>>,
    navigation_authorizations: Arc<FileNavigationAuthorizations>,
}

impl FileAccessGrantSource {
    /// Creates a shared source with `grant` as its initial authorization state.
    pub(crate) fn new(
        grant: FileAccessGrant,
        navigation_authorizations: Arc<FileNavigationAuthorizations>,
    ) -> Self {
        Self {
            grant: Arc::new(RwLock::new(Some(grant))),
            navigation_authorizations,
        }
    }

    /// Replaces the authorization state used by future file opens.
    ///
    /// A poisoned source fails closed: this operation leaves its previous state
    /// unchanged and future snapshots are denied.
    pub fn replace(&self, grant: FileAccessGrant) {
        if let Ok(mut current) = self.grant.write() {
            *current = Some(grant);
            self.navigation_authorizations.clear();
        }
    }

    /// Revokes local-file access for future opens by removing its policy-issued grant.
    pub fn revoke(&self) {
        if let Ok(mut grant) = self.grant.write() {
            *grant = None;
            self.navigation_authorizations.clear();
        }
    }

    fn snapshot(&self) -> Option<FileAccessGrant> {
        self.grant.read().map(|grant| grant.clone()).unwrap_or(None)
    }
}

/// Adapts security policy and filesystem access to the network provider API.
#[derive(Clone)]
pub(crate) struct BrowserFileProvider {
    capability: Arc<FileAccessCapability>,
    grants: FileAccessGrantSource,
    navigation_authorizations: Arc<FileNavigationAuthorizations>,
    read_slots: Arc<Semaphore>,
}

impl BrowserFileProvider {
    /// Creates a provider using the supplied browser-owned capability.
    pub(crate) fn new(
        capability: Arc<FileAccessCapability>,
        grants: FileAccessGrantSource,
        navigation_authorizations: Arc<FileNavigationAuthorizations>,
    ) -> Self {
        Self {
            capability,
            grants,
            navigation_authorizations,
            read_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_FILE_READS)),
        }
    }
}

impl FileProvider for BrowserFileProvider {
    fn open<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let capability = Arc::clone(&self.capability);
        let grants = self.grants.clone();
        let navigation_authorizations = Arc::clone(&self.navigation_authorizations);
        let read_slots = Arc::clone(&self.read_slots);
        let url = url.clone();
        let is_head = request.method() == net::Method::HEAD;
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut cancellation = CancellationGuard::new(Arc::clone(&cancelled));
        let signal = request.signal().clone();
        let preparation_cancelled = Arc::clone(&cancelled);

        Box::pin(async move {
            let grant = grants
                .snapshot()
                .ok_or_else(|| provider_error(FileAccessError::Denied))?;
            let context = request_context(request);
            if context.kind != FileRequestKind::Navigation
                || !navigation_authorizations.consume(request.id())
            {
                return Err(provider_error(FileAccessError::Denied));
            }
            capability
                .authorize_request(&context, &grant)
                .map_err(provider_error)?;
            let path = url
                .to_file_path()
                .map_err(|_| provider_error(FileAccessError::InvalidPath))?;
            let permit = read_slots
                .acquire_owned()
                .await
                .map_err(|_| provider_io_error())?;
            let task = tokio::task::spawn_blocking(move || {
                if preparation_cancelled.load(Ordering::Acquire) {
                    return Err(RequestError::Aborted);
                }
                let opened = capability
                    .open_authorized_file(&context, &grant, path)
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
pub(crate) fn provider(
    capability: Arc<FileAccessCapability>,
    grants: FileAccessGrantSource,
    navigation_authorizations: Arc<FileNavigationAuthorizations>,
) -> Arc<dyn FileProvider> {
    Arc::new(BrowserFileProvider::new(
        capability,
        grants,
        navigation_authorizations,
    ))
}
