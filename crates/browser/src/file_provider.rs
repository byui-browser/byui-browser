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
use tokio::sync::Notify;
use tokio::sync::{Semaphore, mpsc};

#[cfg(test)]
use std::sync::atomic::AtomicU64;

pub use security::file_access::FileAccessGrant;

const FILE_READ_CHUNK_SIZE: usize = 64 * 1024;
const MAX_CONCURRENT_FILE_READS: usize = 4;
const MAX_PENDING_FILE_READS: usize = 8;
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
    /// The local-file session has been revoked or closed.
    SessionRevoked,
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
            Self::SessionRevoked => formatter.write_str("file access session has been revoked"),
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

/// Cancellation state captured by one local-file setup operation or body stream.
#[derive(Clone, Debug)]
struct FileAccessCancellation {
    state: Arc<FileAccessCancellationState>,
}

#[derive(Debug, Default)]
struct FileAccessCancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

impl FileAccessCancellation {
    fn new() -> Self {
        Self {
            state: Arc::new(FileAccessCancellationState::default()),
        }
    }

    fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::AcqRel) {
            self.state.notify.notify_waiters();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.state.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[derive(Debug)]
struct FileAccessGrantState {
    grant: Option<FileAccessGrant>,
    cancellation: FileAccessCancellation,
}

/// Mutable browser-owned source for one local-file session's authorization state.
///
/// Providers snapshot this source immediately before request authorization.
/// Replacing, revoking, or closing the session cancels every operation holding
/// the prior snapshot and denies future opens until a replacement grant exists.
#[derive(Clone, Debug)]
pub struct FileAccessGrantSource {
    state: Arc<RwLock<FileAccessGrantState>>,
    navigation_authorizations: Arc<FileNavigationAuthorizations>,
}

impl FileAccessGrantSource {
    /// Creates a shared source with `grant` as its initial authorization state.
    pub(crate) fn new(
        grant: FileAccessGrant,
        navigation_authorizations: Arc<FileNavigationAuthorizations>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(FileAccessGrantState {
                grant: Some(grant),
                cancellation: FileAccessCancellation::new(),
            })),
            navigation_authorizations,
        }
    }

    /// Replaces the authorization state used by future file opens.
    ///
    /// Replacing a grant cancels setup and streams admitted by the prior grant.
    /// A poisoned source fails closed: this operation leaves its previous state
    /// unchanged and future snapshots are denied.
    pub fn replace(&self, grant: FileAccessGrant) {
        if let Ok(mut current) = self.state.write() {
            current.cancellation.cancel();
            current.grant = Some(grant);
            current.cancellation = FileAccessCancellation::new();
            self.navigation_authorizations.clear();
        }
    }

    /// Revokes local-file access and cancels its pending setup and live streams.
    pub fn revoke(&self) {
        if let Ok(mut current) = self.state.write() {
            current.cancellation.cancel();
            current.grant = None;
            self.navigation_authorizations.clear();
        }
    }

    fn snapshot(&self) -> Option<(FileAccessGrant, FileAccessCancellation)> {
        self.state.read().ok().and_then(|state| {
            state
                .grant
                .clone()
                .map(|grant| (grant, state.cancellation.clone()))
        })
    }

    pub(crate) fn is_active(&self) -> bool {
        self.state
            .read()
            .is_ok_and(|state| state.grant.is_some() && !state.cancellation.is_cancelled())
    }
}

/// Bounded admission for provider setup and live file readers.
///
/// At most four readers may hold an opened file and at most eight additional
/// authorized requests may wait for a reader. A waiting request retains no
/// file handle or blocking task.
#[derive(Clone, Debug)]
struct FileReadAdmission {
    active: Arc<Semaphore>,
    pending: Arc<Semaphore>,
}

impl FileReadAdmission {
    fn new(max_active: usize, max_pending: usize) -> Self {
        Self {
            active: Arc::new(Semaphore::new(max_active)),
            pending: Arc::new(Semaphore::new(max_pending)),
        }
    }

    async fn acquire(
        &self,
        signal: &AbortSignal,
        session_cancellation: &FileAccessCancellation,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, RequestError> {
        if signal.is_aborted() || session_cancellation.is_cancelled() {
            return Err(RequestError::Aborted);
        }
        let pending = self
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| provider_resource_exhausted_error())?;
        let active = self.active.clone().acquire_owned();
        let result = tokio::select! {
            _ = signal.cancelled() => Err(RequestError::Aborted),
            _ = session_cancellation.cancelled() => Err(RequestError::Aborted),
            permit = active => permit.map_err(|_| provider_io_error()),
        };
        drop(pending);
        result
    }
}

/// Adapts security policy and filesystem access to the network provider API.
#[derive(Clone)]
pub(crate) struct BrowserFileProvider {
    capability: Arc<FileAccessCapability>,
    grants: FileAccessGrantSource,
    navigation_authorizations: Arc<FileNavigationAuthorizations>,
    read_admission: FileReadAdmission,
    max_file_size: u64,
    #[cfg(test)]
    post_setup_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl BrowserFileProvider {
    /// Creates a provider using the supplied browser-owned capability.
    pub(crate) fn new(
        capability: Arc<FileAccessCapability>,
        grants: FileAccessGrantSource,
        navigation_authorizations: Arc<FileNavigationAuthorizations>,
        max_file_size: u64,
    ) -> Self {
        Self {
            capability,
            grants,
            navigation_authorizations,
            read_admission: FileReadAdmission::new(
                MAX_CONCURRENT_FILE_READS,
                MAX_PENDING_FILE_READS,
            ),
            max_file_size,
            #[cfg(test)]
            post_setup_hook: None,
        }
    }
}

impl FileProvider for BrowserFileProvider {
    fn open<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let capability = Arc::clone(&self.capability);
        let grants = self.grants.clone();
        let navigation_authorizations = Arc::clone(&self.navigation_authorizations);
        let read_admission = self.read_admission.clone();
        let max_file_size = self.max_file_size;
        let url = url.clone();
        let is_head = request.method() == net::Method::HEAD;
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut cancellation = CancellationGuard::new(Arc::clone(&cancelled));
        let signal = request.signal().clone();
        let preparation_cancelled = Arc::clone(&cancelled);
        #[cfg(test)]
        let post_setup_hook = self.post_setup_hook.clone();

        Box::pin(async move {
            let (grant, session_cancellation) = grants
                .snapshot()
                .ok_or_else(|| provider_error(FileAccessError::Denied))?;
            if session_cancellation.is_cancelled() {
                return Err(RequestError::Aborted);
            }
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
            let permit = read_admission
                .acquire(&signal, &session_cancellation)
                .await?;
            let setup_cancellation = session_cancellation.clone();
            let task = tokio::task::spawn_blocking(move || {
                if preparation_cancelled.load(Ordering::Acquire)
                    || setup_cancellation.is_cancelled()
                {
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

            #[cfg(test)]
            if let Some(hook) = post_setup_hook {
                hook();
            }
            if signal.is_aborted() || session_cancellation.is_cancelled() {
                return Err(RequestError::Aborted);
            }
            if length > max_file_size {
                return Err(provider_resource_exhausted_error());
            }
            let expected_length = length;

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
                    let worker_session_cancellation = session_cancellation.clone();
                    tokio::task::spawn_blocking(move || {
                        read_file_stream(
                            opened,
                            sender,
                            permit,
                            worker_cancelled,
                            worker_session_cancellation,
                            signal,
                            expected_length,
                        );
                    });
                    cancellation.disarm();
                    SchemeBody::Stream(Box::pin(FileBodyStream {
                        receiver,
                        cancelled,
                        session_cancellation,
                        reported_session_cancellation: false,
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
    session_cancellation: FileAccessCancellation,
    reported_session_cancellation: bool,
}

impl Stream for FileBodyStream {
    type Item = Result<Vec<u8>, RequestError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.session_cancellation.is_cancelled() {
            if self.reported_session_cancellation {
                return Poll::Ready(None);
            }
            self.reported_session_cancellation = true;
            return Poll::Ready(Some(Err(RequestError::Aborted)));
        }
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
    session_cancellation: FileAccessCancellation,
    signal: AbortSignal,
    expected_length: u64,
) {
    let mut buffer = [0; FILE_READ_CHUNK_SIZE];
    let mut remaining = expected_length;
    loop {
        if cancelled.load(Ordering::Acquire)
            || session_cancellation.is_cancelled()
            || signal.is_aborted()
        {
            return;
        }
        if remaining == 0 {
            match opened.read_chunk(&mut buffer[..1]).map_err(provider_error) {
                Ok(0) => return,
                Ok(_) => {
                    let _ = send_chunk(
                        &sender,
                        Err(provider_io_error()),
                        &cancelled,
                        &session_cancellation,
                        &signal,
                    );
                    return;
                }
                Err(error) => {
                    let _ = send_chunk(
                        &sender,
                        Err(error),
                        &cancelled,
                        &session_cancellation,
                        &signal,
                    );
                    return;
                }
            }
        }
        let permitted = usize::try_from(remaining.min(FILE_READ_CHUNK_SIZE as u64))
            .expect("file read chunk size fits in usize");
        match opened
            .read_chunk(&mut buffer[..permitted])
            .map_err(provider_error)
        {
            Ok(0) => {
                let _ = send_chunk(
                    &sender,
                    Err(provider_io_error()),
                    &cancelled,
                    &session_cancellation,
                    &signal,
                );
                return;
            }
            Ok(count) => {
                remaining -= count as u64;
                if !send_chunk(
                    &sender,
                    Ok(buffer[..count].to_vec()),
                    &cancelled,
                    &session_cancellation,
                    &signal,
                ) {
                    return;
                }
            }
            Err(error) => {
                let _ = send_chunk(
                    &sender,
                    Err(error),
                    &cancelled,
                    &session_cancellation,
                    &signal,
                );
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
    session_cancellation: &FileAccessCancellation,
    signal: &AbortSignal,
) -> bool {
    loop {
        if cancelled.load(Ordering::Acquire)
            || session_cancellation.is_cancelled()
            || signal.is_aborted()
        {
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

fn provider_resource_exhausted_error() -> RequestError {
    RequestError::Provider {
        provider: "file",
        kind: ProviderErrorKind::ResourceExhausted,
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
        FileAccessError::UnsupportedFile => ProviderErrorKind::UnsupportedResource,
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
    max_file_size: u64,
) -> Arc<dyn FileProvider> {
    Arc::new(BrowserFileProvider::new(
        capability,
        grants,
        navigation_authorizations,
        max_file_size,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use cap_std::{ambient_authority, fs::Dir};
    use futures_util::StreamExt;
    use security::file_access::TrustedDirectorySelection;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn file_read_admission_rejects_excess_waiters_and_cancels_queued_work() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let admission = FileReadAdmission::new(0, 1);
            let abort = net::AbortController::new();
            let session_cancellation = FileAccessCancellation::new();
            let waiting_admission = admission.clone();
            let waiting_signal = abort.signal();
            let waiting_session_cancellation = session_cancellation.clone();
            let waiting = tokio::spawn(async move {
                waiting_admission
                    .acquire(&waiting_signal, &waiting_session_cancellation)
                    .await
            });

            while admission.pending.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
            assert!(matches!(
                admission
                    .acquire(&AbortSignal::new(), &session_cancellation)
                    .await,
                Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::ResourceExhausted,
                })
            ));

            abort.abort();
            assert!(matches!(waiting.await.unwrap(), Err(RequestError::Aborted)));

            let session_admission = FileReadAdmission::new(0, 1);
            let session_cancellation = FileAccessCancellation::new();
            let waiting_admission = session_admission.clone();
            let waiting_cancellation = session_cancellation.clone();
            let session_waiting = tokio::spawn(async move {
                waiting_admission
                    .acquire(&AbortSignal::new(), &waiting_cancellation)
                    .await
            });
            while session_admission.pending.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
            session_cancellation.cancel();
            assert!(matches!(
                session_waiting.await.unwrap(),
                Err(RequestError::Aborted)
            ));
        });
    }

    #[test]
    fn revocation_after_file_setup_rejects_get_and_head_and_releases_admission() {
        let directory = std::env::temp_dir().join(format!(
            "byui-browser-post-setup-abort-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let file = directory.join("page.html");
        fs::write(&file, b"prepared content").unwrap();
        let canonical_directory = fs::canonicalize(&directory).unwrap();
        let selection = TrustedDirectorySelection::from_open_directory(
            canonical_directory,
            Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
        )
        .unwrap();
        let capability =
            Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
        let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
        let grants = FileAccessGrantSource::new(
            capability.issue_grant(),
            Arc::clone(&navigation_authorizations),
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let url = Url::from_file_path(&file).unwrap();

        for method in [net::Method::GET, net::Method::HEAD] {
            let hook_grants = grants.clone();
            let mut provider = BrowserFileProvider::new(
                Arc::clone(&capability),
                grants.clone(),
                Arc::clone(&navigation_authorizations),
                u64::MAX,
            );
            provider.post_setup_hook = Some(Arc::new(move || hook_grants.revoke()));
            let mut request = Request::get(url.clone());
            request.set_method(method).unwrap();
            request.set_mode(net::RequestMode::Navigate).unwrap();
            navigation_authorizations.authorize(request.id()).unwrap();

            assert!(matches!(
                runtime.block_on(provider.open(&request, &url)),
                Err(RequestError::Aborted)
            ));
            assert_eq!(
                provider.read_admission.active.available_permits(),
                MAX_CONCURRENT_FILE_READS
            );
            assert_eq!(
                provider.read_admission.pending.available_permits(),
                MAX_PENDING_FILE_READS
            );

            grants.replace(capability.issue_grant());
            let probe = BrowserFileProvider {
                capability: Arc::clone(&provider.capability),
                grants: provider.grants.clone(),
                navigation_authorizations: Arc::clone(&provider.navigation_authorizations),
                read_admission: provider.read_admission.clone(),
                max_file_size: provider.max_file_size,
                post_setup_hook: None,
            };
            let mut probe_request = Request::get(url.clone());
            probe_request.set_method(net::Method::HEAD).unwrap();
            probe_request.set_mode(net::RequestMode::Navigate).unwrap();
            navigation_authorizations
                .authorize(probe_request.id())
                .unwrap();
            let response = runtime.block_on(probe.open(&probe_request, &url)).unwrap();
            assert!(matches!(response.body, SchemeBody::Null));
        }

        drop(runtime);
        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dropping_unread_or_partially_read_body_releases_reader_admission() {
        let directory = std::env::temp_dir().join(format!(
            "byui-browser-body-drop-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let file = directory.join("page.html");
        fs::write(&file, vec![b'x'; FILE_READ_CHUNK_SIZE * 4]).unwrap();
        let canonical_directory = fs::canonicalize(&directory).unwrap();
        let selection = TrustedDirectorySelection::from_open_directory(
            canonical_directory,
            Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
        )
        .unwrap();
        let capability =
            Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
        let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
        let grants = FileAccessGrantSource::new(
            capability.issue_grant(),
            Arc::clone(&navigation_authorizations),
        );
        let provider = BrowserFileProvider::new(
            Arc::clone(&capability),
            grants,
            Arc::clone(&navigation_authorizations),
            u64::MAX,
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let url = Url::from_file_path(&file).unwrap();

        for read_first_chunk in [false, true] {
            let mut request = Request::get(url.clone());
            request.set_mode(net::RequestMode::Navigate).unwrap();
            navigation_authorizations.authorize(request.id()).unwrap();
            let resource = runtime.block_on(provider.open(&request, &url)).unwrap();
            let SchemeBody::Stream(mut body) = resource.body else {
                panic!("GET requests should receive a live file body");
            };

            if read_first_chunk {
                let chunk = runtime.block_on(body.next()).unwrap().unwrap();
                assert!(!chunk.is_empty());
            }
            drop(body);

            runtime.block_on(wait_for_readers_to_finish(&provider.read_admission));
            assert_eq!(
                provider.read_admission.active.available_permits(),
                MAX_CONCURRENT_FILE_READS
            );
        }

        drop(runtime);
        drop(provider);
        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn file_body_fails_when_opened_length_does_not_match_live_bytes() {
        let directory = std::env::temp_dir().join(format!(
            "byui-browser-content-length-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let file = directory.join("page.html");
        let canonical_directory = fs::canonicalize(&directory).unwrap();
        let selection = TrustedDirectorySelection::from_open_directory(
            canonical_directory,
            Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
        )
        .unwrap();
        let capability =
            Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
        let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
        let grants = FileAccessGrantSource::new(
            capability.issue_grant(),
            Arc::clone(&navigation_authorizations),
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let url = Url::from_file_path(&file).unwrap();

        for actual in [b"a".as_slice(), b"abc".as_slice()] {
            fs::write(&file, b"ab").unwrap();
            let replacement = actual.to_vec();
            let replacement_path = file.clone();
            let mut provider = BrowserFileProvider::new(
                Arc::clone(&capability),
                grants.clone(),
                Arc::clone(&navigation_authorizations),
                u64::MAX,
            );
            provider.post_setup_hook = Some(Arc::new(move || {
                fs::write(&replacement_path, &replacement).unwrap();
            }));
            let mut request = Request::get(url.clone());
            request.set_mode(net::RequestMode::Navigate).unwrap();
            navigation_authorizations.authorize(request.id()).unwrap();
            let resource = runtime.block_on(provider.open(&request, &url)).unwrap();
            assert_eq!(
                resource
                    .headers
                    .iter()
                    .find(|(name, _)| name == "content-length")
                    .unwrap()
                    .1,
                HeaderValue::from_static("2")
            );
            let SchemeBody::Stream(mut body) = resource.body else {
                panic!("GET requests should receive a live file body");
            };

            let mut received = Vec::new();
            let error = loop {
                match runtime.block_on(body.next()).unwrap() {
                    Ok(chunk) => received.extend_from_slice(&chunk),
                    Err(error) => break error,
                }
            };
            assert!(matches!(
                error,
                RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::Io
                }
            ));
            assert_eq!(received, actual[..actual.len().min(2)]);
        }

        drop(runtime);
        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    async fn wait_for_readers_to_finish(admission: &FileReadAdmission) {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while admission.active.available_permits() != MAX_CONCURRENT_FILE_READS {
            assert!(
                std::time::Instant::now() < deadline,
                "dropped body should release its file-reader permit"
            );
            tokio::task::yield_now().await;
        }
    }
}
