use std::{
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
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
        let mut remaining = self.remaining.load(Ordering::Acquire);
        loop {
            let Some(next) = remaining.checked_sub(bytes) else {
                return false;
            };
            match self.remaining.compare_exchange_weak(
                remaining,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => remaining = current,
            }
        }
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
    generation: u64,
}

/// A path-free local-file permission lifecycle event.
///
/// These events intentionally contain no paths, URLs, file contents, or OS
/// diagnostic data, so embedders may retain them in ordinary audit logs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileAccessAuditEvent {
    /// A replacement grant superseded the prior grant.
    Replaced,
    /// Browser code explicitly revoked the session.
    Revoked,
    /// The configured grant lifetime elapsed.
    Expired,
    /// Browser shutdown explicitly ended the session.
    Shutdown,
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
    audit: Arc<Mutex<Vec<FileAccessAuditEvent>>>,
    lifetime: Option<Duration>,
}

impl FileAccessGrantSource {
    /// Creates a source with `grant` as its initial authorization state.
    ///
    /// When `lifetime` is present, each initial or replacement grant expires
    /// after that duration through the regular revocation path.
    pub(crate) fn new(
        grant: FileAccessGrant,
        navigation_authorizations: Arc<FileNavigationAuthorizations>,
        lifetime: Option<Duration>,
    ) -> Self {
        let source = Self {
            state: Arc::new(RwLock::new(FileAccessGrantState {
                grant: Some(grant),
                cancellation: FileAccessCancellation::new(),
                generation: 0,
            })),
            navigation_authorizations,
            audit: Arc::new(Mutex::new(Vec::new())),
            lifetime,
        };
        source.schedule_expiry(0);
        source
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
            current.generation = current.generation.wrapping_add(1);
            self.navigation_authorizations.clear();
            self.record(FileAccessAuditEvent::Replaced);
            self.schedule_expiry(current.generation);
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
            self.record(FileAccessAuditEvent::Revoked);
        }
    }

    /// Ends the local-file session during browser process shutdown.
    ///
    /// Shutdown follows the same cancellation and authorization-clearing path
    /// as revocation, and records a distinct path-free audit event.
    pub fn shutdown(&self) {
        if let Ok(mut current) = self.state.write() {
            revoke_current(&mut current);
            self.navigation_authorizations.clear();
            self.record(FileAccessAuditEvent::Shutdown);
        }
    }

    /// Returns lifecycle events recorded by this source in occurrence order.
    pub fn audit_events(&self) -> Vec<FileAccessAuditEvent> {
        self.audit
            .lock()
            .map_or_else(|_| Vec::new(), |events| events.clone())
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

    fn record(&self, event: FileAccessAuditEvent) {
        if let Ok(mut events) = self.audit.lock() {
            events.push(event);
        }
    }

    fn schedule_expiry(&self, generation: u64) {
        let Some(lifetime) = self.lifetime else {
            return;
        };
        let state = Arc::downgrade(&self.state);
        let navigation_authorizations = Arc::downgrade(&self.navigation_authorizations);
        let audit = Arc::downgrade(&self.audit);
        thread::spawn(move || {
            thread::sleep(lifetime);
            expire_generation(state, navigation_authorizations, audit, generation);
        });
    }
}

fn revoke_current(current: &mut FileAccessGrantState) {
    if let Some(grant) = current.grant.as_ref() {
        grant.revoke();
    }
    current.cancellation.cancel();
    current.grant = None;
}

fn expire_generation(
    state: Weak<RwLock<FileAccessGrantState>>,
    navigation_authorizations: Weak<FileNavigationAuthorizations>,
    audit: Weak<Mutex<Vec<FileAccessAuditEvent>>>,
    generation: u64,
) {
    let Some(state) = state.upgrade() else { return };
    let expired = state.write().is_ok_and(|mut current| {
        if current.generation != generation || current.grant.is_none() {
            return false;
        }
        revoke_current(&mut current);
        true
    });
    if !expired {
        return;
    }
    if let Some(navigation_authorizations) = navigation_authorizations.upgrade() {
        navigation_authorizations.clear();
    }
    if let Some(audit) = audit.upgrade() {
        if let Ok(mut events) = audit.lock() {
            events.push(FileAccessAuditEvent::Expired);
        }
    }
}
