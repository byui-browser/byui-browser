use std::sync::Arc;

use net::{AbortSignal, RequestError};
use tokio::sync::Semaphore;

use super::{
    policy::{provider_io_error, provider_resource_exhausted_error},
    session::FileAccessCancellation,
};

pub(super) const MAX_CONCURRENT_FILE_READS: usize = 4;
pub(super) const MAX_PENDING_FILE_READS: usize = 8;

/// Bounded admission for provider setup and live file readers.
///
/// At most four readers may hold an opened file and at most eight additional
/// authorized requests may wait for a reader. A waiting request retains no
/// file handle or blocking task.
#[derive(Clone, Debug)]
pub(super) struct FileReadAdmission {
    pub(super) active: Arc<Semaphore>,
    pub(super) pending: Arc<Semaphore>,
}

impl FileReadAdmission {
    pub(super) fn new(max_active: usize, max_pending: usize) -> Self {
        Self {
            active: Arc::new(Semaphore::new(max_active)),
            pending: Arc::new(Semaphore::new(max_pending)),
        }
    }

    pub(super) async fn acquire(
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
