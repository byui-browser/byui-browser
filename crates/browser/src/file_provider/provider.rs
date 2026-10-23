use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use net::{
    FileProvider, HeaderName, HeaderValue, Origin, ProviderErrorKind, ProviderFuture, Request,
    RequestError, SchemeBody, SchemeResource, Url,
};
use security::file_access::{FileAccessCapability, FileAccessError, FileRequestKind};
use tokio::sync::mpsc;

use super::{
    admission::{FileReadAdmission, MAX_CONCURRENT_FILE_READS, MAX_PENDING_FILE_READS},
    authorization::FileNavigationAuthorizations,
    policy::{
        allows_content, is_native_executable_prefix, provider_error, provider_io_error,
        provider_resource_exhausted_error, request_context,
    },
    reader::{FILE_READ_CHANNEL_CAPACITY, FileReadTask, read_file_stream},
    session::{FileAccessGrantSource, FileSessionByteBudget},
    stream::{CancellationGuard, FileBodyStream},
};

/// Adapts security policy and filesystem access to the network provider API.
#[derive(Clone)]
pub(crate) struct BrowserFileProvider {
    pub(super) capability: Arc<FileAccessCapability>,
    pub(super) grants: FileAccessGrantSource,
    pub(super) navigation_authorizations: Arc<FileNavigationAuthorizations>,
    pub(super) read_admission: FileReadAdmission,
    pub(super) limits: crate::FileAccessLimits,
    pub(super) content_policy: crate::FileAccessContentPolicy,
    pub(super) session_budget: FileSessionByteBudget,
    #[cfg(test)]
    /// Deterministically mutates the already-open handle's file before this
    /// provider publishes response metadata.
    pub(super) after_open_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl BrowserFileProvider {
    /// Creates a provider using the supplied browser-owned capability.
    pub(crate) fn new(
        capability: Arc<FileAccessCapability>,
        grants: FileAccessGrantSource,
        navigation_authorizations: Arc<FileNavigationAuthorizations>,
        limits: crate::FileAccessLimits,
        content_policy: crate::FileAccessContentPolicy,
    ) -> Self {
        Self {
            capability,
            grants,
            navigation_authorizations,
            read_admission: FileReadAdmission::new(
                MAX_CONCURRENT_FILE_READS,
                MAX_PENDING_FILE_READS,
            ),
            session_budget: FileSessionByteBudget::new(limits.max_session_bytes),
            limits,
            content_policy,
            #[cfg(test)]
            after_open_hook: None,
        }
    }
}

impl FileProvider for BrowserFileProvider {
    fn open<'a>(&'a self, request: &'a Request, url: &'a Url) -> ProviderFuture<'a> {
        let capability = Arc::clone(&self.capability);
        let grants = self.grants.clone();
        let navigation_authorizations = Arc::clone(&self.navigation_authorizations);
        let read_admission = self.read_admission.clone();
        let limits = self.limits;
        let content_policy = self.content_policy;
        let session_budget = self.session_budget.clone();
        let url = url.clone();
        let is_head = request.method() == net::Method::HEAD;
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut cancellation = CancellationGuard::new(Arc::clone(&cancelled));
        let signal = request.signal().clone();
        let preparation_cancelled = Arc::clone(&cancelled);
        #[cfg(test)]
        let after_open_hook = self.after_open_hook.clone();

        Box::pin(async move {
            let (grant, lease, session_cancellation) = grants
                .snapshot()
                .ok_or_else(|| provider_error(FileAccessError::Denied))?;
            if session_cancellation.is_cancelled() || !lease.is_current() {
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
            let deadline = Instant::now() + limits.max_read_duration;
            let setup_cancellation = session_cancellation.clone();
            let setup_lease = lease.clone();
            let task = tokio::task::spawn_blocking(move || {
                if preparation_cancelled.load(Ordering::Acquire)
                    || setup_cancellation.is_cancelled()
                    || !setup_lease.is_current()
                {
                    return Err(RequestError::Aborted);
                }
                let opened = capability
                    .open_authorized_file(&context, &grant, &setup_lease, path)
                    .map_err(|error| {
                        if setup_lease.is_current() {
                            provider_error(error)
                        } else {
                            RequestError::Aborted
                        }
                    })?;
                let length = opened.length();
                let media_type = opened.media_type().map(str::to_owned);
                Ok((opened, length, media_type))
            });
            let (mut opened, length, media_type) =
                task.await.map_err(|_| provider_io_error())??;

            #[cfg(test)]
            if let Some(hook) = after_open_hook {
                hook();
            }
            if signal.is_aborted()
                || session_cancellation.is_cancelled()
                || !lease.is_current()
                || Instant::now() >= deadline
            {
                return Err(if Instant::now() >= deadline {
                    provider_resource_exhausted_error()
                } else {
                    RequestError::Aborted
                });
            }
            if length > limits.max_file_size {
                return Err(provider_resource_exhausted_error());
            }
            if !allows_content(content_policy, media_type.as_deref()) {
                return Err(provider_error(FileAccessError::UnsupportedFile));
            }
            let mut prefix = [0; 4];
            let prefix_length = opened.read_prefix(&mut prefix).map_err(provider_error)?;
            if is_native_executable_prefix(&prefix[..prefix_length]) {
                return Err(provider_error(FileAccessError::UnsupportedFile));
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
                headers.push((
                    HeaderName::from_static("x-content-type-options"),
                    HeaderValue::from_static("nosniff"),
                ));
            }
            // This is the linearization point for metadata publication. A
            // revocation observed first aborts setup; a later revocation is
            // enforced by the body lease checks below.
            if !lease.is_current() {
                return Err(RequestError::Aborted);
            }
            Ok(SchemeResource {
                url,
                status: 200,
                status_text: Some("OK".into()),
                headers,
                media_type,
                // Local-file documents deliberately receive an opaque origin.
                // The security policy denies their subresources until a
                // file-document container identity is implemented.
                origin: Origin::opaque(),
                body: if is_head {
                    drop(opened);
                    drop(permit);
                    SchemeBody::Null
                } else {
                    let (sender, receiver) = mpsc::channel(FILE_READ_CHANNEL_CAPACITY);
                    let worker_cancelled = Arc::clone(&cancelled);
                    let worker_session_cancellation = session_cancellation.clone();
                    let worker_lease = lease.clone();
                    tokio::task::spawn_blocking(move || {
                        read_file_stream(FileReadTask {
                            opened,
                            sender,
                            _permit: permit,
                            cancelled: worker_cancelled,
                            session_cancellation: worker_session_cancellation,
                            lease: worker_lease,
                            signal,
                            expected_length,
                            deadline,
                            session_budget,
                        });
                    });
                    cancellation.disarm();
                    SchemeBody::Stream(Box::pin(FileBodyStream {
                        receiver,
                        cancelled,
                        session_cancellation,
                        lease,
                        reported_session_cancellation: false,
                        deadline,
                        reported_timeout: false,
                    }))
                },
            })
        })
    }
}

/// Creates a provider from one browser-owned file-access capability.
pub(crate) fn provider(
    capability: Arc<FileAccessCapability>,
    grants: FileAccessGrantSource,
    navigation_authorizations: Arc<FileNavigationAuthorizations>,
    limits: crate::FileAccessLimits,
    content_policy: crate::FileAccessContentPolicy,
) -> Arc<dyn FileProvider> {
    Arc::new(BrowserFileProvider::new(
        capability,
        grants,
        navigation_authorizations,
        limits,
        content_policy,
    ))
}
