//! Shared request and response body state.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use http::header::HeaderValue;

use crate::api::error::RequestError;

mod blob;
mod construct;
mod consume;
mod form_data;
mod metadata;
mod stream;
mod transport;
#[cfg(test)]
mod tests;

pub use blob::Blob;
pub use form_data::{FormData, FormDataEntry};

/// A sendable stream used by a live Fetch body.
pub(crate) type BoxedBodyStream = std::pin::Pin<
    Box<dyn futures_util::Stream<Item = Result<Bytes, RequestError>> + Send>,
>;

/// Storage strategy that determines whether a body can be replayed or consumed once.
enum BodySource {
    Null,
    Replayable(Bytes),
    OneShotBytes(Option<Bytes>),
    Stream(Option<BoxedBodyStream>),
}

/// Mutable state shared by body clones and updated while the stream is polled.
struct BodyState {
    source: BodySource,
    content_type: Option<HeaderValue>,
    length: Option<u64>,
    locked: bool,
    used: bool,
    completed: bool,
    failed: bool,
    aborted: bool,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

/// A Fetch body shared by request construction and response delivery.
///
/// A body is either a null body, replayable bytes, a one-shot byte source, or
/// a one-shot stream. Consumption through [`Stream`] or an `into_*` helper
/// marks the body used. Cloning replayable bytes creates an independent body;
/// cloning a one-shot source shares its consumption state.
pub struct Body {
    state: Arc<Mutex<BodyState>>,
}

impl Clone for Body {
    /// Clones replayable data independently and shares one-shot consumption state.
    fn clone(&self) -> Self {
        let state = self
            .state
            .lock()
            .expect("body state lock should not be poisoned");
        match &state.source {
            BodySource::Null => Self::null(),
            BodySource::Replayable(bytes) => {
                Self::from_bytes_with_content_type(bytes.to_vec(), state.content_type.clone())
            }
            BodySource::OneShotBytes(_) | BodySource::Stream(_) => Self {
                state: Arc::clone(&self.state),
            },
        }
    }
}
impl fmt::Debug for Body {
    /// Formats body metadata without consuming or exposing its payload.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self
            .state
            .lock()
            .expect("body state lock should not be poisoned");
        formatter
            .debug_struct("Body")
            .field("is_null", &matches!(state.source, BodySource::Null))
            .field(
                "replayable",
                &matches!(state.source, BodySource::Replayable(_)),
            )
            .field("content_type", &state.content_type)
            .field("length", &state.length)
            .field("locked", &state.locked)
            .field("used", &state.used)
            .field("completed", &state.completed)
            .field("failed", &state.failed)
            .field("aborted", &state.aborted)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Body {
    /// Compares replayable payloads by value and one-shot bodies by identity.
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.state, &other.state) {
            return true;
        }
        let left_state = self
            .state
            .lock()
            .expect("body state lock should not be poisoned");
        let right_state = other
            .state
            .lock()
            .expect("body state lock should not be poisoned");
        match (&left_state.source, &right_state.source) {
            (BodySource::Null, BodySource::Null) => true,
            (BodySource::Replayable(left), BodySource::Replayable(right)) => {
                left == right && left_state.content_type == right_state.content_type
            }
            (BodySource::OneShotBytes(_), BodySource::OneShotBytes(_))
            | (BodySource::Stream(_), BodySource::Stream(_)) => {
                Arc::ptr_eq(&self.state, &other.state)
            }
            _ => false,
        }
    }
}
impl Eq for Body {}

impl Body {
    /// Initializes shared body state with optional media type and byte length.
    fn new(source: BodySource, content_type: Option<HeaderValue>, length: Option<u64>) -> Self {
        Self {
            state: Arc::new(Mutex::new(BodyState {
                source,
                content_type,
                length,
                locked: false,
                used: false,
                completed: false,
                failed: false,
                aborted: false,
                permit: None,
            })),
        }
    }
}
