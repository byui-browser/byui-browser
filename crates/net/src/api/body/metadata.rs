//! Body metadata accessors and content-type parsing helpers.

use http::header::{CONTENT_TYPE, HeaderValue};

use crate::api::{Headers, error::RequestError};

use super::{Body, BodySource};

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

    /// Assigns response media type derived from transport or cache headers.
    pub(crate) fn set_content_type(&mut self, content_type: Option<HeaderValue>) {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .content_type = content_type;
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
