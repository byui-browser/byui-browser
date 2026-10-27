use std::{collections::HashSet, fmt, sync::Mutex};

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
    pending: Mutex<HashSet<u64>>,
}

impl FileNavigationAuthorizations {
    pub(crate) fn new() -> Self {
        Self {
            pending: Mutex::new(HashSet::new()),
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

    pub(super) fn clear(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }
}
