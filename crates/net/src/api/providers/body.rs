//! Provider-neutral resource and body values.

use std::pin::Pin;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http::{HeaderName, HeaderValue};
use url::Url;

use crate::api::{body::Body, cancellation::AbortSignal, error::RequestError, request::Origin};

/// A sendable stream supplied by a browser-owned scheme provider.
///
/// The stream is polled through the request's abort signal and is dropped when
/// the request is aborted, the body fails, or the caller discards the body.
pub type ProviderBodyStream =
    Pin<Box<dyn Stream<Item = Result<Vec<u8>, RequestError>> + Send + 'static>>;

/// Body representation returned by a scheme-specific provider.
pub enum SchemeBody {
    /// A response that must not yield body bytes.
    Null,
    /// A fully materialized response body.
    Bytes(Vec<u8>),
    /// A sendable response stream whose chunks are converted into an
    /// abort-aware [`Body`].
    Stream(ProviderBodyStream),
}

/// Provider-neutral resource metadata used to construct an internal response.
pub struct SchemeResource {
    /// Final URL represented by the resource.
    pub url: Url,
    /// HTTP-like status code assigned to the resource.
    pub status: u16,
    /// Optional reason phrase or diagnostic status text.
    pub status_text: Option<String>,
    /// Provider-approved response headers before Fetch filtering.
    pub headers: Vec<(HeaderName, HeaderValue)>,
    /// Media type associated with the body, when known.
    pub media_type: Option<HeaderValue>,
    /// Origin assigned by the provider, including an opaque origin where required.
    pub origin: Origin,
    /// Body bytes, stream, or explicit null-body state.
    pub body: SchemeBody,
}

impl SchemeBody {
    /// Converts provider body data into the shared abort-aware engine body model.
    pub(crate) fn into_body(self, signal: AbortSignal, force_null: bool) -> Body {
        if force_null {
            return Body::null();
        }
        match self {
            Self::Null => Body::null(),
            Self::Bytes(bytes) => Body::once_with_signal(bytes, signal),
            Self::Stream(stream) => {
                let stream = stream.map(|chunk| chunk.map(Bytes::from));
                Body::from_stream_with_signal(stream, signal)
            }
        }
    }
}
