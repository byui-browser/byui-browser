//! Blob-like byte payloads used by Fetch bodies and form data.

use http::header::HeaderValue;

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

    /// Separates bytes and media type for internal body construction.
    pub(super) fn into_parts(self) -> (Vec<u8>, Option<HeaderValue>) {
        (self.bytes, self.media_type)
    }
}
