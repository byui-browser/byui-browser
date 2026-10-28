//! Streaming implementation for Fetch bodies.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::Stream;

use crate::api::error::RequestError;

use super::{Body, BodySource};

impl Stream for Body {
    type Item = Result<Bytes, RequestError>;

    /// Polls the body, updating lock, used, completion, and failure state.
    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut state = self
            .state
            .lock()
            .expect("body state lock should not be poisoned");
        if matches!(state.source, BodySource::Null) {
            state.used = true;
            state.completed = true;
            return Poll::Ready(None);
        }

        let replayable = match &state.source {
            BodySource::Replayable(bytes) => Some(bytes.clone()),
            _ => None,
        };
        if let Some(bytes) = replayable {
            if state.used {
                state.completed = true;
                return Poll::Ready(None);
            }
            state.used = true;
            state.completed = true;
            state.permit = None;
            return Poll::Ready(Some(Ok(bytes)));
        }

        let one_shot = match &mut state.source {
            BodySource::OneShotBytes(bytes) => Some(bytes.take()),
            _ => None,
        };
        if let Some(bytes) = one_shot {
            state.completed = true;
            return match bytes {
                Some(bytes) => {
                    state.used = true;
                    state.permit = None;
                    Poll::Ready(Some(Ok(bytes)))
                }
                None => Poll::Ready(None),
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
                state.used = true;
                state.completed = true;
                state.permit = None;
                Poll::Ready(None)
            }
            Some(Poll::Pending) => Poll::Pending,
            Some(Poll::Ready(Some(Ok(chunk)))) => {
                state.used = true;
                Poll::Ready(Some(Ok(chunk)))
            }
            Some(Poll::Ready(Some(Err(error)))) => {
                if let BodySource::Stream(stream) = &mut state.source {
                    *stream = None;
                }
                state.used = true;
                state.failed = !matches!(error, RequestError::Aborted);
                state.aborted = matches!(error, RequestError::Aborted);
                state.completed = true;
                state.permit = None;
                Poll::Ready(Some(Err(error)))
            }
        }
    }
}
