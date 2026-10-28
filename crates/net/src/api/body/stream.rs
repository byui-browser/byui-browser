//! Streaming, teeing, and backpressure behavior for Fetch bodies.

use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use bytes::Bytes;
use futures_util::Stream;

use crate::api::error::RequestError;

use super::{Body, BodyPhase, BodySource, BoxedBodyStream};

/// Shared bounded fan-out state for two response body branches.
struct TeeState {
    source: Option<BoxedBodyStream>,
    buffers: [VecDeque<Result<Bytes, RequestError>>; 2],
    active: [bool; 2],
    wakers: [Option<Waker>; 2],
    done: bool,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

/// One independently consumable branch of a bounded body tee.
struct TeeBranch {
    shared: Arc<Mutex<TeeState>>,
    index: usize,
}

impl Stream for TeeBranch {
    type Item = Result<Bytes, RequestError>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let index = self.index;
        let other = 1 - index;
        let mut state = self
            .shared
            .lock()
            .expect("body tee lock should not be poisoned");

        if let Some(item) = state.buffers[index].pop_front() {
            if let Some(waker) = state.wakers[other].take() {
                waker.wake();
            }
            return Poll::Ready(Some(item));
        }
        if !state.active[index] || state.done {
            return Poll::Ready(None);
        }

        state.wakers[index] = Some(context.waker().clone());
        // One queued chunk per branch keeps memory bounded and applies
        // backpressure when the other active branch is slower.
        if state.active[other] && !state.buffers[other].is_empty() {
            return Poll::Pending;
        }

        let poll = match state.source.as_mut() {
            Some(source) => source.as_mut().poll_next(context),
            None => Poll::Ready(None),
        };
        match poll {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                state.done = true;
                state.source = None;
                state.permit = None;
                if let Some(waker) = state.wakers[other].take() {
                    waker.wake();
                }
                Poll::Ready(None)
            }
            Poll::Ready(Some(item)) => {
                if state.active[other] {
                    state.buffers[other].push_back(item.clone());
                    if let Some(waker) = state.wakers[other].take() {
                        waker.wake();
                    }
                }
                if item.is_err() {
                    state.done = true;
                    state.source = None;
                    state.permit = None;
                }
                Poll::Ready(Some(item))
            }
        }
    }
}

impl Drop for TeeBranch {
    fn drop(&mut self) {
        let other = 1 - self.index;
        let mut state = self
            .shared
            .lock()
            .expect("body tee lock should not be poisoned");
        state.active[self.index] = false;
        state.buffers[self.index].clear();
        if let Some(waker) = state.wakers[other].take() {
            waker.wake();
        }
        if !state.active[other] {
            state.source = None;
            state.permit = None;
            state.done = true;
        }
    }
}

impl Body {
    /// Tees an unused body, retaining one branch and returning the other.
    pub fn tee(&mut self) -> Result<Self, RequestError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RequestError::BodyNotCloneable)?;
        if state.phase != BodyPhase::Unused {
            return Err(RequestError::BodyNotCloneable);
        }
        let content_type = state.content_type.clone();
        let length = state.length;
        let branches = match &mut state.source {
            BodySource::Null => return Ok(Self::null()),
            BodySource::Replayable(bytes) => {
                return Ok(Self::new(
                    BodySource::Replayable(bytes.clone()),
                    content_type,
                    length,
                ));
            }
            BodySource::OneShotBytes(bytes) => {
                let bytes = bytes.as_ref().ok_or(RequestError::BodyNotCloneable)?;
                return Ok(Self::new(
                    BodySource::OneShotBytes(Some(bytes.clone())),
                    content_type,
                    length,
                ));
            }
            BodySource::Stream(stream) => {
                let stream = stream.take().ok_or(RequestError::BodyNotCloneable)?;
                let permit = state.permit.take();
                state.phase = BodyPhase::Transferred;
                let shared = Arc::new(Mutex::new(TeeState {
                    source: Some(stream),
                    buffers: [VecDeque::new(), VecDeque::new()],
                    active: [true, true],
                    wakers: [None, None],
                    done: false,
                    permit,
                }));
                let branch = |index| {
                    Self::from_body_stream(
                        TeeBranch {
                            shared: Arc::clone(&shared),
                            index,
                        },
                        content_type.clone(),
                        length,
                    )
                };
                (branch(0), branch(1))
            }
        };
        drop(state);
        let (original, cloned) = branches;
        *self = original;
        Ok(cloned)
    }
}

impl Stream for Body {
    type Item = Result<Bytes, RequestError>;

    /// Polls the body, updating lock, used, completion, and failure state.
    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut state = self
            .state
            .lock()
            .expect("body state lock should not be poisoned");
        if matches!(
            state.phase,
            BodyPhase::Transferred | BodyPhase::Complete | BodyPhase::Failed | BodyPhase::Aborted
        ) {
            return Poll::Ready(None);
        }
        if state.phase == BodyPhase::Unused {
            state.phase = BodyPhase::Streaming;
        }
        if matches!(state.source, BodySource::Null) {
            state.phase = BodyPhase::Complete;
            state.permit = None;
            return Poll::Ready(None);
        }

        let replayable = match &state.source {
            BodySource::Replayable(bytes) => Some(bytes.clone()),
            _ => None,
        };
        if let Some(bytes) = replayable {
            state.phase = BodyPhase::Complete;
            state.permit = None;
            return Poll::Ready(Some(Ok(bytes)));
        }

        let one_shot = match &mut state.source {
            BodySource::OneShotBytes(bytes) => Some(bytes.take()),
            _ => None,
        };
        if let Some(bytes) = one_shot {
            return match bytes {
                Some(bytes) => {
                    state.phase = BodyPhase::Complete;
                    state.permit = None;
                    Poll::Ready(Some(Ok(bytes)))
                }
                None => {
                    state.phase = BodyPhase::Complete;
                    state.permit = None;
                    Poll::Ready(None)
                }
            };
        }

        let item = match &mut state.source {
            BodySource::Stream(stream) => stream
                .as_mut()
                .map(|stream| stream.as_mut().poll_next(context)),
            _ => None,
        };
        match item {
            None | Some(Poll::Ready(None)) => {
                if let BodySource::Stream(stream) = &mut state.source {
                    *stream = None;
                }
                state.phase = BodyPhase::Complete;
                state.permit = None;
                Poll::Ready(None)
            }
            Some(Poll::Pending) => Poll::Pending,
            Some(Poll::Ready(Some(Ok(chunk)))) => Poll::Ready(Some(Ok(chunk))),
            Some(Poll::Ready(Some(Err(error)))) => {
                if let BodySource::Stream(stream) = &mut state.source {
                    *stream = None;
                }
                state.phase = if matches!(error, RequestError::Aborted) {
                    BodyPhase::Aborted
                } else {
                    BodyPhase::Failed
                };
                state.permit = None;
                Poll::Ready(Some(Err(error)))
            }
        }
    }
}
