use std::{
    fmt, io,
    pin::Pin,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures_util::{Stream, TryStreamExt};
use http::header::{CONTENT_TYPE, HeaderValue};
use reqwest::Body;

use crate::api::error::RequestError;

use crate::api::Headers;

type RequestStream = Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>;

/// A request body, either repeatable bytes or a shared one-shot byte/stream source.
#[derive(Clone)]
pub struct RequestBody {
    source: BodySource,
    content_type: Option<HeaderValue>,
    length: Option<u64>,
}

#[derive(Clone)]
enum BodySource {
    Bytes(Bytes),
    OneShotBytes(Arc<Mutex<Option<Bytes>>>),
    OneShotStream(Arc<Mutex<Option<RequestStream>>>),
}

impl fmt::Debug for RequestBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestBody")
            .field("replayable", &self.is_replayable())
            .field("content_type", &self.content_type)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

impl PartialEq for RequestBody {
    fn eq(&self, other: &Self) -> bool {
        if self.content_type != other.content_type || self.length != other.length {
            return false;
        }
        match (&self.source, &other.source) {
            (BodySource::Bytes(a), BodySource::Bytes(b)) => a == b,
            (BodySource::OneShotBytes(a), BodySource::OneShotBytes(b)) => Arc::ptr_eq(a, b),
            (BodySource::OneShotStream(a), BodySource::OneShotStream(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for RequestBody {}

impl RequestBody {
    /// Creates a replayable byte body with no inferred media type.
    pub fn bytes(body: impl Into<Vec<u8>>) -> Self {
        let bytes = Bytes::from(body.into());
        let length = Some(bytes.len() as u64);
        Self {
            source: BodySource::Bytes(bytes),
            content_type: None,
            length,
        }
    }

    /// Creates a replayable UTF-8 text body with the Fetch default text media type.
    pub fn text(body: impl Into<String>) -> Self {
        Self::with_content_type(
            body.into().into_bytes(),
            HeaderValue::from_static("text/plain;charset=UTF-8"),
        )
    }

    /// Creates replayable bytes with caller-selected content type.
    pub fn with_content_type(body: impl Into<Vec<u8>>, content_type: HeaderValue) -> Self {
        let bytes = Bytes::from(body.into());
        let length = Some(bytes.len() as u64);
        Self {
            source: BodySource::Bytes(bytes),
            content_type: Some(content_type),
            length,
        }
    }

    /// Creates a URL-encoded form body and sets its standard media type.
    pub fn url_encoded(body: impl Into<Vec<u8>>) -> Self {
        Self::with_content_type(
            body,
            HeaderValue::from_static("application/x-www-form-urlencoded;charset=UTF-8"),
        )
    }

    /// Creates a one-shot body from bytes. Clones share consumption state.
    pub fn one_shot_bytes(body: impl Into<Vec<u8>>) -> Self {
        let bytes = Bytes::from(body.into());
        let length = Some(bytes.len() as u64);
        Self {
            source: BodySource::OneShotBytes(Arc::new(Mutex::new(Some(bytes)))),
            content_type: None,
            length,
        }
    }

    /// Creates a one-shot streaming body. Its byte length is unknown.
    pub fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Vec<u8>, io::Error>> + Send + 'static,
    {
        Self {
            source: BodySource::OneShotStream(Arc::new(Mutex::new(Some(Box::pin(
                stream.map_ok(Bytes::from),
            ))))),
            content_type: None,
            length: None,
        }
    }

    /// Returns true when this body can be sent repeatedly, such as after a redirect.
    pub fn is_replayable(&self) -> bool {
        matches!(self.source, BodySource::Bytes(_))
    }

    /// Returns the known body length in bytes.
    pub fn length(&self) -> Option<u64> {
        self.length
    }

    /// Returns the media type inferred or assigned to this body.
    pub fn content_type(&self) -> Option<&HeaderValue> {
        self.content_type.as_ref()
    }

    pub(crate) fn apply_metadata(&self, headers: &mut Headers) {
        if let Some(content_type) = &self.content_type
            && !headers.entries.iter().any(|(name, _)| name == CONTENT_TYPE)
        {
            headers.insert_internal_header(CONTENT_TYPE, content_type.clone());
        }
    }

    pub(crate) fn into_reqwest_body(self) -> Result<Body, RequestError> {
        match self.source {
            BodySource::Bytes(bytes) => Ok(Body::from(bytes)),
            BodySource::OneShotBytes(shared) => shared
                .lock()
                .map_err(|_| RequestError::BodyAlreadyConsumed)?
                .take()
                .map(Body::from)
                .ok_or(RequestError::BodyAlreadyConsumed),
            BodySource::OneShotStream(shared) => {
                let stream = shared
                    .lock()
                    .map_err(|_| RequestError::BodyAlreadyConsumed)?
                    .take()
                    .ok_or(RequestError::BodyAlreadyConsumed)?;
                Ok(Body::wrap_stream(stream.map_err(|error| error)))
            }
        }
    }
}
