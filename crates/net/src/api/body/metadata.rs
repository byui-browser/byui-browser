//! Body metadata accessors and content-type parsing helpers.

use http::{
    HeaderMap,
    header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderValue},
};

use crate::api::{Headers, error::RequestError};

use super::{Body, BodyPhase, BodySource, BodyStatus};

impl Body {
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
}

/// Returns the declared charset parameter, if present.
pub(super) fn charset_from_content_type(value: &HeaderValue) -> Option<String> {
    let value = value.to_str().ok()?;
    value.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"').to_owned())
    })
}
