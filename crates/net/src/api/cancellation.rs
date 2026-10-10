//! Cancellation primitives shared by Fetch requests and transport streams.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::Notify;

/// A clonable cancellation signal associated with one Fetch operation.
///
/// Clones observe the same cancellation state. Aborting a signal wakes tasks
/// waiting for scheduler admission and response-body bytes.
#[derive(Clone, Debug)]
pub struct AbortSignal {
    state: Arc<AbortState>,
}

#[derive(Debug, Default)]
struct AbortState {
    aborted: AtomicBool,
    notify: Notify,
}

impl Default for AbortSignal {
    /// Creates a signal that has not been aborted.
    fn default() -> Self {
        Self::new()
    }
}

impl AbortSignal {
    /// Creates a non-aborted signal.
    pub fn new() -> Self {
        Self {
            state: Arc::new(AbortState::default()),
        }
    }

    /// Returns whether this signal has been aborted.
    pub fn is_aborted(&self) -> bool {
        self.state.aborted.load(Ordering::Acquire)
    }

    /// Waits until this signal is aborted.
    pub(crate) async fn cancelled(&self) {
        if self.is_aborted() {
            return;
        }

        let notified = self.state.notify.notified();
        if self.is_aborted() {
            return;
        }
        notified.await;
    }

    /// Marks this signal as aborted and wakes all waiters.
    pub(crate) fn abort(&self) {
        if !self.state.aborted.swap(true, Ordering::AcqRel) {
            self.state.notify.notify_waiters();
        }
    }
}

/// The owner-side handle used to cancel a Fetch operation.
#[derive(Clone, Debug)]
pub struct AbortController {
    signal: AbortSignal,
}

impl Default for AbortController {
    /// Creates a controller with a fresh, non-aborted signal.
    fn default() -> Self {
        Self::new()
    }
}

impl AbortController {
    /// Creates a controller with a fresh signal.
    pub fn new() -> Self {
        Self {
            signal: AbortSignal::new(),
        }
    }

    /// Returns the signal that can be attached to a request.
    pub fn signal(&self) -> AbortSignal {
        self.signal.clone()
    }

    /// Aborts every request using this controller's signal.
    pub fn abort(&self) {
        self.signal.abort();
    }

    /// Returns whether this controller has already been aborted.
    pub fn is_aborted(&self) -> bool {
        self.signal.is_aborted()
    }
}
