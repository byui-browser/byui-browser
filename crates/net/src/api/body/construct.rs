//! Body constructors and request/response stream factories.

use std::io;

use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt};
use http::header::HeaderValue;

use crate::api::{cancellation::AbortSignal, error::RequestError};

use super::{Blob, Body, BodySource, BoxedBodyStream, FormData};

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
        let (bytes, media_type) = blob.into_parts();
        Self::from_bytes_with_content_type(bytes, media_type)
    }

    /// Creates a replayable URL-encoded or multipart body from form data.
    ///
    /// Text-only forms use `application/x-www-form-urlencoded`; a form with a
    /// file entry uses a deterministic multipart boundary and preserves the
    /// file media type where supplied.
    pub fn from_form_data(form: FormData) -> Self {
        let (bytes, content_type) = super::form_data::serialize(form);
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
    /// Creates a single buffered body for body-state tests.
    pub(crate) fn once(body: Vec<u8>) -> Self {
        Self::from_stream(futures_util::stream::once(
            async move { Ok(Bytes::from(body)) },
        ))
    }

    /// Stores a sendable response or request stream as a one-shot body.
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
}
