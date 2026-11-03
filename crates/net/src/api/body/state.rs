//! Per-view body ownership and lifecycle observation.

use std::{fmt, pin::Pin, sync::Mutex};

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

/// Lifecycle observation for one body view, independent of the tee source.
pub(crate) struct BodyLifecycle {
    pub(crate) phase: BodyPhase,
}

/// Owned source, metadata, and observation for one body view.
pub(crate) struct BodyState {
    pub(crate) source: BodySource,
    pub(crate) content_type: Option<HeaderValue>,
    pub(crate) length: Option<u64>,
    pub(crate) lifecycle: BodyLifecycle,
    pub(crate) permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

/// A Fetch body shared by request construction and response delivery.
///
/// A body is either a null body, replayable bytes, a one-shot byte source, or
/// a one-shot stream. Consumption through [`Stream`] or an `into_*` helper
/// marks the body used. Use [`Body::fork`] to create an independent body view
/// while it is unused; live streams are shared through a bounded tee.
pub struct Body {
    pub(crate) state: Mutex<BodyState>,
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
            .field("phase", &state.lifecycle.phase)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Body {
    /// Compares replayable payloads by value and one-shot bodies by identity.
    fn eq(&self, other: &Self) -> bool {
        if std::ptr::eq(self, other) {
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
            | (BodySource::Stream(_), BodySource::Stream(_)) => false,
            _ => false,
        }
    }
}

impl Eq for Body {}

impl Body {
    /// Duplicates an unused buffered source for a separate request send.
    pub(crate) fn try_duplicate_buffered(&self) -> Result<Self, RequestError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RequestError::BodyNotCloneable)?;
        if state.lifecycle.phase != BodyPhase::Unused {
            return Err(RequestError::BodyNotCloneable);
        }
        let source = match &state.source {
            BodySource::Null => BodySource::Null,
            BodySource::Replayable(bytes) => BodySource::Replayable(bytes.clone()),
            BodySource::OneShotBytes(Some(bytes)) => BodySource::OneShotBytes(Some(bytes.clone())),
            BodySource::OneShotBytes(None) | BodySource::Stream(_) => {
                return Err(RequestError::BodyNotCloneable);
            }
        };
        Ok(Self::new(source, state.content_type.clone(), state.length))
    }

    /// Duplicates an unused body into independently consumable views.
    ///
    /// Buffered sources copy their ownership; live streams share a bounded
    /// one-chunk tee. A used or transferred source cannot be forked.
    pub fn fork(&mut self) -> Result<Self, RequestError> {
        self.tee()
    }
}
