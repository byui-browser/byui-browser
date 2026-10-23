use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use futures_util::Stream;
use net::RequestError;
use security::file_access::FileAccessLease;
use tokio::sync::mpsc;

use super::session::FileAccessCancellation;

/// A body stream that cancels its blocking reader when Fetch discards it.
pub(super) struct FileBodyStream {
    pub(super) receiver: mpsc::Receiver<Result<Vec<u8>, RequestError>>,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) session_cancellation: FileAccessCancellation,
    pub(super) lease: FileAccessLease,
    pub(super) reported_session_cancellation: bool,
}

impl Stream for FileBodyStream {
    type Item = Result<Vec<u8>, RequestError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.session_cancellation.is_cancelled() || !self.lease.is_current() {
            if self.reported_session_cancellation {
                return Poll::Ready(None);
            }
            self.reported_session_cancellation = true;
            return Poll::Ready(Some(Err(RequestError::Aborted)));
        }
        match self.receiver.poll_recv(context) {
            Poll::Ready(Some(_)) if !self.lease.is_current() => {
                if self.reported_session_cancellation {
                    Poll::Ready(None)
                } else {
                    self.reported_session_cancellation = true;
                    Poll::Ready(Some(Err(RequestError::Aborted)))
                }
            }
            result => result,
        }
    }
}

impl Drop for FileBodyStream {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Stops pending provider work if Fetch drops the resolution future.
pub(super) struct CancellationGuard {
    cancelled: Arc<AtomicBool>,
    armed: bool,
}

impl CancellationGuard {
    pub(super) fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            armed: true,
        }
    }

    pub(super) fn disarm(&mut self) {
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
