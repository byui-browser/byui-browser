//! Shared request and response body state.

use std::{
    collections::VecDeque,
    fmt, io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt};
use http::{
    HeaderMap,
    header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderValue},
};
use reqwest::Body as ReqwestBody;

use crate::api::{Headers, cancellation::AbortSignal, error::RequestError};

mod blob;
mod consume;
mod form_data;
mod metadata;
#[cfg(test)]
mod tests;

pub use blob::Blob;
pub use form_data::{FormData, FormDataEntry};

/// A sendable stream used by a live Fetch body.
pub(crate) type BoxedBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, RequestError>> + Send>>;

/// Storage strategy that determines whether a body can be replayed or consumed once.
enum BodySource {
    Null,
    Replayable(Bytes),
    OneShotBytes(Option<Bytes>),
    Stream(Option<BoxedBodyStream>),
}

/// Authoritative lifecycle state for one body view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BodyPhase {
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
struct BodyState {
    source: BodySource,
    content_type: Option<HeaderValue>,
    length: Option<u64>,
    phase: BodyPhase,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

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
    /// Creates replayable bytes with no inferred media type.
    pub fn from_bytes(body: impl Into<Vec<u8>>) -> Self {
        Self::from_bytes_with_content_type(body, None)
    }

    /// Creates replayable bytes from an ArrayBuffer-compatible input.
    pub fn from_array_buffer(body: impl Into<Vec<u8>>) -> Self {
        Self::from_bytes(body)
    }

    /// Copies a typed-array byte view into a replayable body.
    pub fn from_typed_array(body: &[u8]) -> Self {
        Self::from_bytes(body.to_vec())
    }

    /// Creates replayable UTF-8 text with Fetch's default text media type.
    pub fn from_text(body: impl Into<String>) -> Self {
        Self::from_bytes_with_content_type(
            body.into().into_bytes(),
            Some(HeaderValue::from_static("text/plain;charset=UTF-8")),
        )
    }

    /// Creates replayable bytes with a caller-selected media type.
    pub fn from_bytes_with_content_type(
        body: impl Into<Vec<u8>>,
        content_type: Option<HeaderValue>,
    ) -> Self {
        let bytes = Bytes::from(body.into());
        Self::new(
            BodySource::Replayable(bytes.clone()),
            content_type,
            Some(bytes.len() as u64),
        )
    }

    /// Creates URL-encoded form bytes with their standard media type.
    pub fn from_url_encoded(body: impl Into<Vec<u8>>) -> Self {
        Self::from_bytes_with_content_type(
            body,
            Some(HeaderValue::from_static(
                "application/x-www-form-urlencoded;charset=UTF-8",
            )),
        )
    }

    /// Creates a replayable body from blob-like bytes and media type.
    pub fn from_blob(blob: Blob) -> Self {
        let (bytes, media_type) = blob.into_parts();
        Self::from_bytes_with_content_type(bytes, media_type)
    }

    /// Creates a replayable URL-encoded or multipart body from form data.
    ///
    /// Text-only forms use `application/x-www-form-urlencoded`; a form with a
    /// file entry uses a deterministic multipart boundary and preserves the
    /// file media type where supplied.
    pub fn from_form_data(form: FormData) -> Self {
        let (bytes, content_type) = form_data::serialize(form);
        Self::from_bytes_with_content_type(bytes, Some(content_type))
    }

    /// Creates a one-shot byte body. Clones share its consumption state.
    pub fn one_shot_bytes(body: impl Into<Vec<u8>>) -> Self {
        let bytes = Bytes::from(body.into());
        Self::new(
            BodySource::OneShotBytes(Some(bytes.clone())),
            None,
            Some(bytes.len() as u64),
        )
    }

    /// Creates a one-shot request stream. Its length is unknown.
    pub fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Vec<u8>, io::Error>> + Send + 'static,
    {
        let stream = stream.map_ok(Bytes::from).map_err(|error| {
            RequestError::Transport(format!("request body stream failed: {error}"))
        });
        Self::from_body_stream(stream, None, None)
    }

    /// Creates an explicit null body that never yields bytes.
    pub(crate) fn null() -> Self {
        Self::new(BodySource::Null, None, Some(0))
    }

    /// Wraps a live response stream and observes request cancellation.
    pub(crate) fn from_stream_with_signal<S>(stream: S, signal: AbortSignal) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        let stream = futures_util::stream::unfold(
            (Some(Box::pin(stream) as BoxedBodyStream), signal),
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
        Self::from_body_stream(stream, None, None)
    }

    /// Wraps a live body stream without adding cancellation behavior.
    pub(crate) fn from_stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        Self::from_body_stream(stream, None, None)
    }

    /// Creates a single buffered response body that observes cancellation.
    pub(crate) fn once_with_signal(body: Vec<u8>, signal: AbortSignal) -> Self {
        let length = body.len() as u64;
        Self::from_stream_with_signal(
            futures_util::stream::once(async move { Ok(Bytes::from(body)) }),
            signal,
        )
        .with_length(Some(length))
    }

    #[cfg(test)]
    /// Creates a single buffered body for body-state tests.
    pub(crate) fn once(body: Vec<u8>) -> Self {
        let length = body.len() as u64;
        Self::from_stream(futures_util::stream::once(
            async move { Ok(Bytes::from(body)) },
        ))
        .with_length(Some(length))
    }

    /// Stores a sendable response or request stream as a one-shot body.
    fn from_body_stream<S>(
        stream: S,
        content_type: Option<HeaderValue>,
        length: Option<u64>,
    ) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        Self::new(
            BodySource::Stream(Some(Box::pin(stream))),
            content_type,
            length,
        )
    }

    /// Initializes shared body state with optional media type and byte length.
    fn new(source: BodySource, content_type: Option<HeaderValue>, length: Option<u64>) -> Self {
        Self {
            state: Arc::new(Mutex::new(BodyState {
                source,
                content_type,
                length,
                phase: BodyPhase::Unused,
                permit: None,
            })),
        }
    }

    /// Assigns a known or declared byte length and returns this body.
    fn with_length(mut self, length: Option<u64>) -> Self {
        self.set_length(length);
        self
    }

    /// Returns whether this is a null body.
    pub fn is_null(&self) -> bool {
        matches!(
            self.state
                .lock()
                .expect("body state lock should not be poisoned")
                .source,
            BodySource::Null
        )
    }

    /// Returns whether the source can be independently replayed.
    pub fn is_replayable(&self) -> bool {
        matches!(
            self.state
                .lock()
                .expect("body state lock should not be poisoned")
                .source,
            BodySource::Replayable(_)
        )
    }

    /// Returns whether this body is backed by a one-shot stream.
    pub(crate) fn is_streaming_source(&self) -> bool {
        matches!(
            self.state
                .lock()
                .expect("body state lock should not be poisoned")
                .source,
            BodySource::Stream(_)
        )
    }

    /// Returns the known length in bytes, if the source has one.
    pub fn length(&self) -> Option<u64> {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .length
    }

    /// Returns the inferred or assigned media type.
    pub fn content_type(&self) -> Option<HeaderValue> {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .content_type
            .clone()
    }

    /// Adds inferred body metadata to an internal transport header channel.
    pub(crate) fn apply_metadata(&self, headers: &mut Headers) -> Result<(), RequestError> {
        if let Some(content_type) = self.content_type()
            && !headers.entries.iter().any(|(name, _)| name == CONTENT_TYPE)
        {
            headers.insert_internal_header(CONTENT_TYPE, content_type)?;
        }
        if let Some(length) = self.length()
            && !headers
                .entries
                .iter()
                .any(|(name, _)| name == CONTENT_LENGTH)
        {
            let value = HeaderValue::from_str(&length.to_string())
                .map_err(|_| RequestError::InvalidHeaderValue("content-length".into()))?;
            headers.insert_internal_header(CONTENT_LENGTH, value)?;
        }
        Ok(())
    }

    /// Assigns response media type derived from transport or cache headers.
    pub(crate) fn set_content_type(&mut self, content_type: Option<HeaderValue>) {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .content_type = content_type;
    }

    /// Assigns body length metadata derived from response headers or providers.
    pub(crate) fn set_length(&mut self, length: Option<u64>) {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .length = length;
    }

    /// Applies response `Content-Type` and declared-length metadata.
    ///
    /// A valid `Content-Length` takes precedence over a source's inferred
    /// length; otherwise `fallback_length` and the source length are retained.
    pub(crate) fn set_response_metadata(
        &mut self,
        headers: &HeaderMap,
        fallback_length: Option<u64>,
    ) {
        self.set_content_type(headers.get(CONTENT_TYPE).cloned());
        let length = headers
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .or(fallback_length)
            .or_else(|| self.length());
        self.set_length(length);
    }

    /// Returns whether any reader has started consuming this body.
    pub fn is_used(&self) -> bool {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .phase
            != BodyPhase::Unused
    }

    /// Returns whether a consumption helper has claimed this body.
    pub fn is_locked(&self) -> bool {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .phase
            == BodyPhase::Locked
    }

    /// Returns the current lifecycle status of this body view.
    pub fn status(&self) -> BodyStatus {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .phase
            .into()
    }

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

    /// Claims this body for one of the consuming helper methods.
    pub(super) fn claim_for_helper(&self) -> Result<(), RequestError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RequestError::BodyAlreadyUsed)?;
        if state.phase != BodyPhase::Unused {
            return Err(RequestError::BodyAlreadyUsed);
        }
        state.phase = BodyPhase::Locked;
        Ok(())
    }

    /// Transfers the source to Reqwest for a request send.
    pub(crate) fn into_reqwest_body(self) -> Result<ReqwestBody, RequestError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RequestError::BodyAlreadyConsumed)?;
        if state.phase != BodyPhase::Unused {
            return Err(RequestError::BodyAlreadyConsumed);
        }
        state.phase = BodyPhase::Transferred;
        match &mut state.source {
            BodySource::Null => Ok(ReqwestBody::from(Bytes::new())),
            BodySource::Replayable(bytes) => Ok(ReqwestBody::from(bytes.clone())),
            BodySource::OneShotBytes(bytes) => bytes
                .take()
                .map(ReqwestBody::from)
                .ok_or(RequestError::BodyAlreadyConsumed),
            BodySource::Stream(stream) => stream
                .take()
                .map(ReqwestBody::wrap_stream)
                .ok_or(RequestError::BodyAlreadyConsumed),
        }
    }

    /// Separates a live stream from its scheduler permit for internal wrappers.
    pub(crate) fn into_parts(self) -> (BoxedBodyStream, Option<tokio::sync::OwnedSemaphorePermit>) {
        let mut state = self
            .state
            .lock()
            .expect("body state lock should not be poisoned");
        state.phase = BodyPhase::Transferred;
        let stream = match &mut state.source {
            BodySource::Stream(stream) => stream
                .take()
                .unwrap_or_else(|| Box::pin(futures_util::stream::empty())),
            BodySource::Null => Box::pin(futures_util::stream::empty()),
            BodySource::Replayable(bytes) => {
                let bytes = bytes.clone();
                Box::pin(futures_util::stream::once(async move { Ok(bytes) }))
            }
            BodySource::OneShotBytes(bytes) => {
                let bytes = bytes.take();
                Box::pin(futures_util::stream::iter(bytes.map(Ok)))
            }
        };
        (stream, state.permit.take())
    }

    /// Attaches the scheduler permit that remains held while a response body is live.
    pub(crate) fn attach_permit(&mut self, permit: tokio::sync::OwnedSemaphorePermit) {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .permit = Some(permit);
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
