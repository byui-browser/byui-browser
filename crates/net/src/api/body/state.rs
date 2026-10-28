//! Shared body storage and lifecycle state.

use std::{
    fmt,
    pin::Pin,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures_util::Stream;
use http::header::HeaderValue;

use crate::api::error::RequestError;

/// A sendable stream used by a live Fetch body.
pub(crate) type BoxedBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, RequestError>> + Send>>;

/// Storage strategy that determines whether a body can be replayed or consumed once.
pub(crate) enum BodySource {
    Null,
    Replayable(Bytes),
    OneShotBytes(Option<Bytes>),
    Stream(Option<BoxedBodyStream>),
}

/// Authoritative lifecycle state for one body view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BodyPhase {
    Unused,
    Locked,
    Streaming,
    Transferred,
    Complete,
    Failed,
    Aborted,
}

/// Observable lifecycle state of a Fetch body view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyStatus {
    /// No consumer has claimed or polled the body.
    Unused,
    /// A consumption helper has exclusively claimed the body.
    Locked,
    /// The body is being read through its stream interface.
    Streaming,
    /// The source was handed to transport or an internal wrapper.
    Transferred,
    /// The body reached a clean end-of-stream.
    Complete,
    /// The body stream ended with a non-abort error.
    Failed,
    /// The body stream ended because its request was aborted.
    Aborted,
}

impl From<BodyPhase> for BodyStatus {
    fn from(phase: BodyPhase) -> Self {
        match phase {
            BodyPhase::Unused => Self::Unused,
            BodyPhase::Locked => Self::Locked,
            BodyPhase::Streaming => Self::Streaming,
            BodyPhase::Transferred => Self::Transferred,
            BodyPhase::Complete => Self::Complete,
            BodyPhase::Failed => Self::Failed,
            BodyPhase::Aborted => Self::Aborted,
        }
    }
}

/// Mutable state shared by body clones and updated while the stream is polled.
pub(crate) struct BodyState {
    pub(crate) source: BodySource,
    pub(crate) content_type: Option<HeaderValue>,
    pub(crate) length: Option<u64>,
    pub(crate) phase: BodyPhase,
    pub(crate) permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

/// A Fetch body shared by request construction and response delivery.
///
/// A body is either a null body, replayable bytes, a one-shot byte source, or
/// a one-shot stream. Consumption through [`Stream`] or an `into_*` helper
/// marks the body used. Cloning replayable bytes creates an independent body;
/// cloning a one-shot source shares its consumption state.
pub struct Body {
    pub(crate) state: Arc<Mutex<BodyState>>,
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
            .field("phase", &state.phase)
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
    /// Creates an independent clone when the source is safely duplicable.
    pub fn try_clone(&self) -> Result<Self, RequestError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RequestError::BodyNotCloneable)?;
        if state.phase != BodyPhase::Unused {
            return Err(RequestError::BodyNotCloneable);
        }
        match &state.source {
            BodySource::Null => Ok(Self::null()),
            BodySource::Replayable(bytes) => Ok(Self::new(
                BodySource::Replayable(bytes.clone()),
                state.content_type.clone(),
                state.length,
            )),
            BodySource::OneShotBytes(Some(bytes)) => Ok(Self::new(
                BodySource::OneShotBytes(Some(bytes.clone())),
                state.content_type.clone(),
                state.length,
            )),
            BodySource::OneShotBytes(None) | BodySource::Stream(_) => {
                Err(RequestError::BodyNotCloneable)
            }
        }
    }
}
