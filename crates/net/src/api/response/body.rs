use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};

use crate::api::{cancellation::AbortSignal, error::RequestError};

pub(crate) type BoxedResponseStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, RequestError>> + Send>>;

/// A stream of response body chunks.
///
/// Each item represents one transport-provided chunk. The stream applies
/// backpressure to the underlying HTTP response and may yield a transport
/// error after response headers have already been delivered.
pub struct ResponseBody {
    inner: BoxedResponseStream,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl std::fmt::Debug for ResponseBody {
    /// Formats the stream without attempting to consume it.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResponseBody")
            .finish_non_exhaustive()
    }
}

impl Stream for ResponseBody {
    type Item = Result<Bytes, RequestError>;

    /// Polls the next response body chunk.
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(context)
    }
}

impl ResponseBody {
    /// Wraps an asynchronous byte stream as a response body.
    ///
    /// The stream is boxed so transports can provide different concrete stream
    /// types while exposing one response API. It must be `Send` because the
    /// networking work may be polled on a different executor thread.
    pub(crate) fn from_stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        Self {
            inner: Box::pin(stream),
            permit: None,
        }
    }

    /// Creates a one-item stream for a body that is already buffered.
    pub(crate) fn once(body: Vec<u8>) -> Self {
        Self::from_stream_with_signal(
            futures_util::stream::once(async move { Ok(Bytes::from(body)) }),
            AbortSignal::new(),
        )
    }

    /// Creates a null body with no stream chunks.
    pub(crate) fn empty() -> Self {
        Self::from_stream(futures_util::stream::empty())
    }

    /// Creates a buffered response stream that observes a request signal.
    pub(crate) fn once_with_signal(body: Vec<u8>, signal: AbortSignal) -> Self {
        Self::from_stream_with_signal(
            futures_util::stream::once(async move { Ok(Bytes::from(body)) }),
            signal,
        )
    }

    /// Wraps a byte stream so aborting its signal terminates it with an error.
    pub(crate) fn from_stream_with_signal<S>(stream: S, signal: AbortSignal) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        let stream = futures_util::stream::unfold(
            (Some(Box::pin(stream)), signal),
            |(stream, signal)| async move {
                let mut stream = stream?;
                if signal.is_aborted() {
                    return Some((Err(RequestError::Aborted), (None, signal)));
                }

                tokio::select! {
                    _ = signal.cancelled() => Some((Err(RequestError::Aborted), (None, signal))),
                    item = stream.next() => item.map(|item| (item, (Some(stream), signal))),
                }
            },
        );
        Self::from_stream(stream)
    }

    /// Separates the body stream from the permit that keeps a request admitted.
    ///
    /// This is used when the controller wraps the stream to capture completed
    /// responses for caching. The permit must be reattached to the replacement
    /// body so concurrency remains bounded until consumption finishes.
    pub(crate) fn into_parts(
        self,
    ) -> (
        BoxedResponseStream,
        Option<tokio::sync::OwnedSemaphorePermit>,
    ) {
        (self.inner, self.permit)
    }

    /// Holds a scheduler permit until this body is dropped.
    ///
    /// A streaming request is considered in flight while its body can still
    /// produce bytes, so releasing the permit at header receipt would allow
    /// more active network work than the configured limit permits.
    pub(crate) fn attach_permit(&mut self, permit: tokio::sync::OwnedSemaphorePermit) {
        self.permit = Some(permit);
    }
}
