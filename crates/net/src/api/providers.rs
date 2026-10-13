//! Browser-owned resource results used by scheme-specific Fetch providers.

use std::pin::Pin;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http::{HeaderName, HeaderValue};
use url::Url;

use crate::api::{body::Body, error::RequestError, request::Origin};

/// A sendable stream supplied by a browser-owned scheme provider.
///
/// Providers may use [`SchemeBody::Bytes`] for small or already-materialized
/// resources. This stream form keeps the provider boundary ready for files,
/// blobs, and pages that should not be buffered in a later implementation.
pub type ProviderBodyStream =
    Pin<Box<dyn Stream<Item = Result<Vec<u8>, RequestError>> + Send + 'static>>;

/// Body representation returned by a scheme-specific provider.
pub enum SchemeBody {
    /// A response that must not yield body bytes.
    Null,
    /// A fully materialized response body.
    Bytes(Vec<u8>),
    /// A sendable response stream whose chunks are converted into [`Body`].
    Stream(ProviderBodyStream),
}

/// Provider-neutral resource metadata used to construct an internal response.
///
/// This is not the public Fetch [`Response`](crate::Response). Providers return
/// resource facts through this type; `net` validates and converts them into an
/// internal response before applying Fetch policy and public filtering.
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
    /// Origin assigned by the provider, including an opaque origin where
    /// required by the scheme.
    pub origin: Origin,
    /// Body bytes, stream, or explicit null-body state.
    pub body: SchemeBody,
}

impl SchemeBody {
    /// Converts provider body data into the shared engine body model.
    pub(crate) fn into_body(self) -> (Body, bool) {
        match self {
            Self::Null => (Body::null(), true),
            Self::Bytes(bytes) => (Body::from_bytes(bytes), false),
            Self::Stream(stream) => {
                let stream = stream.map(|chunk| chunk.map(Bytes::from));
                (Body::from_stream(stream), false)
            }
        }
    }
}
