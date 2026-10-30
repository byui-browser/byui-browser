//! Body constructors and request/response stream factories.

use std::{
    io,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt};
use http::header::HeaderValue;

use crate::api::{cancellation::AbortSignal, error::RequestError};

use super::{Blob, Body, BodyPhase, BodySource, BodyState, BoxedBodyStream, FormData, form_data};

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

    /// Creates a single response body from immutable static bytes without copying them.
    pub(crate) fn once_static_with_signal(body: &'static [u8], signal: AbortSignal) -> Self {
        Self::from_stream_with_signal(
            futures_util::stream::once(async move { Ok(Bytes::from_static(body)) }),
            signal,
        )
        .with_length(Some(body.len() as u64))
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
    pub(super) fn from_body_stream<S>(
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
    pub(super) fn new(
        source: BodySource,
        content_type: Option<HeaderValue>,
        length: Option<u64>,
    ) -> Self {
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
}
