use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};

/// Shared, monotonically consumed response-byte budget for one file session.
#[derive(Clone, Debug)]
pub(super) struct FileSessionByteBudget {
    remaining: Arc<std::sync::atomic::AtomicU64>,
}

impl FileSessionByteBudget {
    pub(super) fn new(limit: u64) -> Self {
        Self {
            remaining: Arc::new(std::sync::atomic::AtomicU64::new(limit)),
        }
    }

    pub(super) fn try_consume(&self, bytes: u64) -> bool {
        self.remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(bytes)
            })
            .is_ok()
    }
}

use security::file_access::{FileAccessGrant, FileAccessLease};
use tokio::sync::Notify;

use super::authorization::FileNavigationAuthorizations;

/// Cancellation state captured by one local-file setup operation or body stream.
#[derive(Clone, Debug)]
pub(super) struct FileAccessCancellation {
    state: Arc<FileAccessCancellationState>,
}

#[derive(Debug, Default)]
struct FileAccessCancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

impl FileAccessCancellation {
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new(FileAccessCancellationState::default()),
        }
    }

    pub(super) fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::AcqRel) {
            self.state.notify.notify_waiters();
        }
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    pub(super) async fn cancelled(&self) {
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
            if let Some(previous_grant) = current.grant.as_ref() {
                previous_grant.revoke();
            }
            current.cancellation.cancel();
            current.grant = Some(grant);
            current.cancellation = FileAccessCancellation::new();
            self.navigation_authorizations.clear();
        }
    }

    /// Revokes local-file access and cancels its pending setup and live streams.
    pub fn revoke(&self) {
        if let Ok(mut current) = self.state.write() {
            if let Some(grant) = current.grant.as_ref() {
                grant.revoke();
            }
            current.cancellation.cancel();
            current.grant = None;
            self.navigation_authorizations.clear();
        }
    }

    pub(super) fn snapshot(
        &self,
    ) -> Option<(FileAccessGrant, FileAccessLease, FileAccessCancellation)> {
        self.state.read().ok().and_then(|state| {
            state.grant.clone().and_then(|grant| {
                grant
                    .lease()
                    .map(|lease| (grant, lease, state.cancellation.clone()))
            })
        })
    }

    pub(crate) fn is_active(&self) -> bool {
        self.state.read().is_ok_and(|state| {
            state
                .grant
                .as_ref()
                .is_some_and(|grant| grant.lease().is_some())
                && !state.cancellation.is_cancelled()
        })
    }
}
