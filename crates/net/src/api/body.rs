//! Shared request and response body state.

use std::{
    fmt, io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt};
use http::header::{CONTENT_TYPE, HeaderValue};
use reqwest::Body as ReqwestBody;

use crate::api::{Headers, cancellation::AbortSignal, error::RequestError};

/// A sendable stream used by a live Fetch body.
pub(crate) type BoxedBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, RequestError>> + Send>>;

/// Bytes together with an optional media type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Blob {
    bytes: Vec<u8>,
    media_type: Option<HeaderValue>,
}

impl Blob {
    /// Creates a byte payload with an optional media type.
    pub fn new(bytes: impl Into<Vec<u8>>, media_type: Option<HeaderValue>) -> Self {
        Self {
            bytes: bytes.into(),
            media_type,
        }
    }

    /// Returns the payload bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the payload media type, when supplied.
    pub fn media_type(&self) -> Option<&HeaderValue> {
        self.media_type.as_ref()
    }
}

enum BodySource {
    Null,
    Replayable(Bytes),
    OneShotBytes(Option<Bytes>),
    Stream(Option<BoxedBodyStream>),
}

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
        Self::from_bytes_with_content_type(blob.bytes, blob.media_type)
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
        Self::from_body_stream(stream, None)
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
        Self::from_body_stream(stream, None)
    }

    /// Wraps a live body stream without adding cancellation behavior.
    pub(crate) fn from_stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        Self::from_body_stream(stream, None)
    }

    /// Creates a single buffered response body that observes cancellation.
    pub(crate) fn once_with_signal(body: Vec<u8>, signal: AbortSignal) -> Self {
        Self::from_stream_with_signal(
            futures_util::stream::once(async move { Ok(Bytes::from(body)) }),
            signal,
        )
    }

    #[cfg(test)]
    pub(crate) fn once(body: Vec<u8>) -> Self {
        Self::from_stream(futures_util::stream::once(
            async move { Ok(Bytes::from(body)) },
        ))
    }

    fn from_body_stream<S>(stream: S, content_type: Option<HeaderValue>) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        Self::new(
            BodySource::Stream(Some(Box::pin(stream))),
            content_type,
            None,
        )
    }

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
        Ok(())
    }

    /// Returns whether any reader has started consuming this body.
    pub fn is_used(&self) -> bool {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .used
    }

    /// Returns whether a consumption helper has claimed this body.
    pub fn is_locked(&self) -> bool {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .locked
    }

    /// Consumes this body into owned bytes.
    pub async fn bytes(mut self) -> Result<Vec<u8>, RequestError> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| RequestError::BodyAlreadyUsed)?;
            if state.used || state.locked {
                return Err(RequestError::BodyAlreadyUsed);
            }
            state.locked = true;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = self.next().await {
            bytes.extend_from_slice(&chunk?);
        }
        Ok(bytes)
    }

    /// Consumes this body as UTF-8 text, replacing malformed byte sequences.
    pub async fn text(self) -> Result<String, RequestError> {
        Ok(String::from_utf8_lossy(&self.bytes().await?).into_owned())
    }

    /// Consumes this body as a blob-like byte payload.
    pub async fn blob(self) -> Result<Blob, RequestError> {
        let media_type = self.content_type();
        Ok(Blob::new(self.bytes().await?, media_type))
    }

    /// Transfers the source to Reqwest for a request send.
    pub(crate) fn into_reqwest_body(self) -> Result<ReqwestBody, RequestError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RequestError::BodyAlreadyConsumed)?;
        match &mut state.source {
            BodySource::Null => Ok(ReqwestBody::from(Bytes::new())),
            BodySource::Replayable(bytes) => Ok(ReqwestBody::from(bytes.clone())),
            BodySource::OneShotBytes(bytes) => bytes
                .take()
                .map(|bytes| {
                    state.used = true;
                    ReqwestBody::from(bytes)
                })
                .ok_or(RequestError::BodyAlreadyConsumed),
            BodySource::Stream(stream) => stream
                .take()
                .map(|stream| {
                    state.used = true;
                    ReqwestBody::wrap_stream(stream)
                })
                .ok_or(RequestError::BodyAlreadyConsumed),
        }
    }

    /// Separates a live stream from its scheduler permit for internal wrappers.
    pub(crate) fn into_parts(self) -> (BoxedBodyStream, Option<tokio::sync::OwnedSemaphorePermit>) {
        let mut state = self
            .state
            .lock()
            .expect("body state lock should not be poisoned");
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

#[cfg(test)]
mod tests {
    use std::io;

    use futures_util::stream;

    use super::*;

    #[test]
    fn replayable_clones_have_independent_consumption_state() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let body = Body::from_bytes(b"replayable".to_vec());
        let clone = body.clone();

        assert_eq!(runtime.block_on(body.bytes()).unwrap(), b"replayable");
        assert_eq!(runtime.block_on(clone.bytes()).unwrap(), b"replayable");
    }

    #[test]
    fn one_shot_clones_share_consumption_state() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let body = Body::one_shot_bytes(b"once".to_vec());
        let clone = body.clone();

        assert!(!body.is_locked());
        assert_eq!(runtime.block_on(clone.bytes()).unwrap(), b"once");
        assert!(body.is_locked());
        assert!(body.is_used());
        assert!(matches!(
            runtime.block_on(body.bytes()),
            Err(RequestError::BodyAlreadyUsed)
        ));
    }

    #[test]
    fn body_helpers_preserve_text_and_blob_metadata() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = Body::from_text("hello");
        assert_eq!(runtime.block_on(text.text()).unwrap(), "hello");

        let blob = Blob::new(
            b"blob".to_vec(),
            Some(HeaderValue::from_static("application/octet-stream")),
        );
        let blob = runtime.block_on(Body::from_blob(blob).blob()).unwrap();
        assert_eq!(blob.bytes(), b"blob");
        assert_eq!(
            blob.media_type(),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_stream_failure_marks_the_shared_body_used() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let body = Body::stream(stream::once(async {
            Err(io::Error::other("stream failure"))
        }));
        let clone = body.clone();

        assert!(matches!(
            runtime.block_on(clone.bytes()),
            Err(RequestError::Transport(_))
        ));
        assert!(matches!(
            runtime.block_on(body.bytes()),
            Err(RequestError::BodyAlreadyUsed)
        ));
    }
}
